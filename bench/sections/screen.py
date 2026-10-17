"""9. The screen: what the compositor costs on a still screen, and how evenly it shows a client's frames, idle and under
load. The baseline that the Mission Control interface is compared against (docs/INTERFACE.md, Targets).

The compositor is the process that listens on the Wayland socket ($XDG_RUNTIME_DIR/$WAYLAND_DISPLAY): the socket names
it (SO_PEERCRED), so no other process is searched. The section reads process and thread names and timings only, never
a window title or anything on the screen.

Still screen, over SCREEN_STILL_S seconds (60, the window of the brief's target) while the section waits:
  screen.still_cpu_pct      the compositor's CPU time over the window, all its threads (utime + stime in
                            /proc/<pid>/stat, which keeps the time of threads that ended), in % of one core
  screen.still_wakeups_s    its wakeups per second: voluntary context switches summed over its threads
                            (/proc/<pid>/task/*/status), each one a sleep that a thread was woken from
Both fail when a key or the mouse was used during the window (GNOME's idle monitor, over D-Bus; on other desktops input
is not checked and the note says so). For the note: the compositor's GPU time over the window where the driver reports
it (drm-engine-* in /proc/<pid>/fdinfo; above 0 means it drew frames, so something on the screen changed), and the CPU
the rest of the machine used, with its busiest processes.

Frame pacing: src/frames.c, built static into the temp dir, opens a 480×270 window and draws a new frame each time the
compositor asks for one (wl_surface.frame), for 1 s of warm-up and then FRAMES_S seconds, and reports when each frame
was presented (the presentation-time protocol). A refresh in which no new frame reached the screen is a missed frame;
they are counted from the presentation sequence numbers, or from the timestamps where the compositor gives none:
  screen.missed_idle_pct    missed frames in % of the refreshes, with the machine as it is
  screen.frame_p99_idle_ms  the slowest 1% of frame times: the 99th percentile of the time between two frames shown
  screen.missed_load_pct, screen.frame_p99_load_ms   the same while FRAMES_CELLS cells (one per CPU thread when unset)
                            each run a busy loop in `bwrap --unshare-all`, and a memory spike: 2 s into the measured
                            time a process touches FRAMES_SPIKE_MIB MiB (when unset, all the memory available then but
                            1 GiB) and holds it to the end
The window is the section's own and closes at the end. The client, the cells and the spike are killed when the section
ends, also on a timeout or Ctrl-C.
"""
import ctypes
import os
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import BENCH_DIR, WORK, emit, env, fail, median, pct, run  # noqa: E402
from sysinfo import busiest, cpu_ticks  # noqa: E402

HZ = os.sysconf('SC_CLK_TCK')
STILL = ['screen.still_cpu_pct', 'screen.still_wakeups_s']
IDLE = ['screen.missed_idle_pct', 'screen.frame_p99_idle_ms']
LOAD = ['screen.missed_load_pct', 'screen.frame_p99_load_ms']
WARMUP = 1.0      # seconds of frames left out: the window opening
SPIKE_AT = 2.0    # seconds into the measured time
LEAVE_MIB = 1024  # what the default spike leaves available
CELL = ['--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc', '--unshare-all', '--die-with-parent',
        'sh', '-c', 'while :; do :; done']
# Touches every page of an anonymous mapping, says how long that took, and holds it (at most argv[2] seconds).
SPIKE = '''import mmap, sys, time
n = int(sys.argv[1]) << 20
t0 = time.perf_counter()
m = mmap.mmap(-1, n, flags=mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
m[::4096] = b'\\1' * len(range(0, n, 4096))
print(round(time.perf_counter() - t0, 2), flush=True)
time.sleep(float(sys.argv[2]))
'''
LIBC = ctypes.CDLL(None, use_errno=True)


def die_with_us():
    """In a child before exec: PR_SET_PDEATHSIG, so it is killed when the section ends, however it ends."""
    LIBC.prctl(1, signal.SIGKILL)


def last_line(b):
    lines = b.decode(errors='replace').strip().splitlines()
    return lines[-1] if lines else 'nothing on stderr'


def compositor():
    """(pid, name) of the process that listens on the Wayland socket."""
    name = os.environ.get('WAYLAND_DISPLAY')
    if not name:
        raise RuntimeError('no Wayland session: WAYLAND_DISPLAY is not set')
    path = name if name.startswith('/') else os.path.join(os.environ.get('XDG_RUNTIME_DIR', ''), name)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        try:
            s.connect(path)
        except OSError as e:
            raise RuntimeError(f'cannot reach the Wayland compositor at $XDG_RUNTIME_DIR/{name}: {e.strerror}')
        pid = struct.unpack('3i', s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
    with open(f'/proc/{pid}/comm') as f:
        return pid, f.read().strip()


def gpu_ns(pid):
    """GPU time the process has used, summed from the drm-engine-* lines of its DRM file descriptors, or None where no
    driver reports it. Descriptors of one DRM client count once."""
    seen, total = set(), None
    for fd in os.listdir(f'/proc/{pid}/fdinfo'):
        try:
            with open(f'/proc/{pid}/fdinfo/{fd}') as f:
                text = f.read()
        except OSError:
            continue
        engines = re.findall(r'^drm-engine-[\w-]+:\s*(\d+) ns', text, re.M)
        client = re.search(r'^drm-client-id:\s*(\d+)', text, re.M)
        pdev = re.search(r'^drm-pdev:\s*(\S+)', text, re.M)
        key = (pdev and pdev.group(1), client.group(1) if client else fd)
        if engines and key not in seen:
            seen.add(key)
            total = (total or 0) + sum(int(x) for x in engines)
    return total


def machine_busy():
    """Clock ticks all CPUs spent busy so far (user to steal, from /proc/stat)."""
    with open('/proc/stat') as f:
        t = [int(x) for x in f.readline().split()[1:9]]
    return sum(t) - t[3] - t[4]  # less idle and iowait


def reading(pid):
    with open(f'/proc/{pid}/stat') as f:
        st = f.read()
    fields = st[st.rindex(')') + 2:].split()
    threads = {}
    for tid in os.listdir(f'/proc/{pid}/task'):
        try:
            with open(f'/proc/{pid}/task/{tid}/status') as f:
                s = f.read()
        except OSError:  # the thread ended
            continue
        threads[tid] = (re.search(r'^Name:\s*(.*)$', s, re.M).group(1),
                        int(re.search(r'^voluntary_ctxt_switches:\s*(\d+)', s, re.M).group(1)))
    return {'t': time.monotonic(), 'ticks': int(fields[11]) + int(fields[12]), 'threads': threads,
            'gpu_ns': gpu_ns(pid), 'machine': machine_busy(), 'procs': cpu_ticks()}


def idle_ms():
    """Milliseconds since the last key press or mouse move, from GNOME's idle monitor, or None where there is none."""
    gdbus = shutil.which('gdbus')
    if not gdbus:
        return None
    try:
        r = run([gdbus, 'call', '--session', '--dest', 'org.gnome.Mutter.IdleMonitor', '--object-path',
                 '/org/gnome/Mutter/IdleMonitor/Core', '--method', 'org.gnome.Mutter.IdleMonitor.GetIdletime'],
                timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    m = re.search(r'uint64 (\d+)', r.stdout.decode(errors='replace'))
    return int(m.group(1)) if r.returncode == 0 and m else None


def load1():
    with open('/proc/loadavg') as f:
        return float(f.read().split()[0])


def still(pid, name, seconds):
    before = load1()
    a = reading(pid)
    time.sleep(seconds)
    b = reading(pid)
    idle = idle_ms()
    dt = b['t'] - a['t']
    if idle is not None and idle < dt * 1000:
        return fail(STILL, f'a key or the mouse was used {idle / 1000:.0f} s before the end of the {dt:.0f} s window, '
                           'so the screen was not still: leave the PC alone during this section, or leave it out '
                           'with SKIP=screen')
    wakeups, by_name = 0, {}
    for tid, (tname, n) in b['threads'].items():
        d = n - a['threads'][tid][1] if tid in a['threads'] else n  # a thread that started in the window: all of it
        wakeups += d
        by_name[re.sub(r'\d+$', '', tname)] = by_name.get(re.sub(r'\d+$', '', tname), 0) + d
    cpu = (b['ticks'] - a['ticks']) / HZ / dt * 100
    gpu = None if a['gpu_ns'] is None or b['gpu_ns'] is None else round((b['gpu_ns'] - a['gpu_ns']) / 1e6, 1)
    ctx = {'window_s': round(dt, 1), 'compositor': name, 'threads': len(b['threads']),
           'threads_ended': len(set(a['threads']) - set(b['threads'])), 'gpu_ms': gpu,
           'rest_cpu_pct': round((b['machine'] - a['machine']) / HZ / dt * 100 - cpu, 1),
           'busy': busiest(a['procs'], b['procs'], dt, floor=10), 'load1': before,
           'input': 'none in the window' if idle is not None else 'not checked: no GNOME idle monitor'}
    emit('screen.still_cpu_pct', cpu, ctx)
    top = sorted(by_name.items(), key=lambda x: -x[1])[:3]
    emit('screen.still_wakeups_s', wakeups / dt, dict(ctx, by_thread=', '.join(f'{n} {c / dt:.1f}/s' for n, c in top)))


def build():
    cc = shutil.which('gcc') or shutil.which('cc')
    if not cc:
        raise RuntimeError('gcc not found')
    out = os.path.join(WORK, 'bin', 'frames')
    os.makedirs(os.path.dirname(out), exist_ok=True)
    r = run([cc, '-O2', '-static', '-o', out, os.path.join(BENCH_DIR, 'src', 'frames.c')])
    if r.returncode != 0:
        raise RuntimeError('cannot build src/frames.c: ' + last_line(r.stderr))
    return out


def client(binary, seconds):
    return subprocess.Popen([binary, str(WARMUP + seconds)], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, preexec_fn=die_with_us)


def pacing(p, seconds):
    """Missed frames and frame times over the measured time, from what the client printed."""
    out, err = p.communicate(timeout=WARMUP + seconds + 30)
    if p.returncode != 0:
        raise RuntimeError(f'the test client failed: {last_line(err)}')
    shown, discarded, focus, clock = [], [], None, None
    for line in out.decode().splitlines():
        f = line.split()
        if f[0] == 'p':  # frame, commit, present, refresh, seq, flags
            shown.append([int(x) for x in f[1:]])
        elif f[0] == 'd':
            discarded.append(int(f[2]))
        elif f[0] == 'focus':
            focus = f[2] == '1'
        elif f[0] == 'clock':
            clock = int(f[1])
    if len(shown) < 2:
        raise RuntimeError('the compositor showed no frames of the test window')
    start = shown[0][2] + WARMUP * 1e9
    shown = [s for s in shown if s[2] >= start]
    pairs = list(zip(shown, shown[1:]))
    if not pairs:
        raise RuntimeError('the compositor stopped showing the test window after its first second')
    intervals = [(b[2] - a[2]) / 1e6 for a, b in pairs]
    refresh = median([s[3] for s in shown if s[3]]) or median(intervals) * 1e6
    by_seq = all(s[5] & 1 for s in shown) and all(b[4] > a[4] for a, b in pairs)  # VSYNC, and the counter moves
    cycles = [b[4] - a[4] if by_seq else max(1, round((b[2] - a[2]) / refresh)) for a, b in pairs]
    missed, total = sum(cycles) - len(cycles), sum(cycles)
    if total and missed / total > 0.5 and not focus:
        raise RuntimeError(f'{missed} of {total} refreshes showed no new frame and the test window never had the '
                           'focus, so it was probably covered or on another workspace')
    lat = [(s[2] - s[1]) / 1e6 for s in shown]
    return {'missed_pct': 100 * missed / total, 'p99_ms': pct(intervals, 0.99), 'clock': clock,
            'steps': [(b[2], c) for (a, b), c in zip(pairs, cycles)],  # when each frame was shown, refreshes since
            'ctx': {'frames': len(shown), 'refreshes': total, 'missed': missed,
                    'discarded': sum(t >= start for t in discarded), 'refresh_hz': round(1e9 / refresh, 2),
                    'worst_ms': round(max(intervals), 1), 'to_screen_ms': round(median(lat), 1),
                    'to_screen_p99_ms': round(pct(lat, 0.99), 1), 'counted_by': 'sequence' if by_seq else 'timestamps',
                    'focus': focus, 'seconds': seconds}}


def emit_pacing(keys, res, ctx):
    ctx = dict(res['ctx'], **ctx)
    emit(keys[0], res['missed_pct'], ctx)
    emit(keys[1], res['p99_ms'], ctx)


def idle(binary, seconds):
    before, a, t0 = load1(), cpu_ticks(), time.monotonic()
    res = pacing(client(binary, seconds), seconds)
    emit_pacing(IDLE, res, {'load1': before, 'busy': busiest(a, cpu_ticks(), time.monotonic() - t0, floor=10)})


def meminfo(field):
    with open('/proc/meminfo') as f:
        return int(re.search(rf'^{field}:\s*(\d+)', f.read(), re.M).group(1)) * 1024


def loaded(binary, seconds):
    bwrap = shutil.which('bwrap')
    if not bwrap:
        raise RuntimeError('bwrap not found: the cells run in bwrap')
    n = int(env('FRAMES_CELLS') or os.cpu_count())
    before, procs = load1(), []
    try:
        for _ in range(n):
            procs.append(subprocess.Popen([bwrap, *CELL], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                          stderr=subprocess.PIPE, preexec_fn=die_with_us))
        time.sleep(1)
        ended = [p for p in procs if p.poll() is not None]
        if ended:
            raise RuntimeError(f'{len(ended)} of {n} cells ended at once: {last_line(ended[0].stderr.read())}')
        available = meminfo('MemAvailable')
        mib = int(env('FRAMES_SPIKE_MIB') or max(256, available // 2**20 - LEAVE_MIB))
        p = client(binary, seconds)
        procs.append(p)
        time.sleep(WARMUP + SPIKE_AT)
        t_spike = time.monotonic_ns()  # CLOCK_MONOTONIC, the clock the compositor's timestamps use here (clock 1)
        spike = subprocess.Popen([sys.executable, '-I', '-S', '-c', SPIKE, str(mib), str(seconds + 10)],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 preexec_fn=die_with_us)
        procs.append(spike)
        res = pacing(p, seconds)
        spike.kill()
        out, err = spike.communicate()
        if spike.returncode != -signal.SIGKILL and not out.strip():
            raise RuntimeError(f'the memory spike of {mib:,} MiB failed: {last_line(err)}')
        touched = out.decode().split()
    finally:
        for q in procs:
            q.kill()
        for q in procs:
            q.wait()
    spike_s = float(touched[0]) if touched else None
    during = None
    if res['clock'] == time.CLOCK_MONOTONIC and spike_s is not None:
        during = sum(c - 1 for t, c in res['steps'] if t_spike <= t <= t_spike + spike_s * 1e9)
    emit_pacing(LOAD, res, {'load1': before, 'cells': n, 'spike_mib': mib, 'spike_s': spike_s,
                            'missed_during_spike': during, 'available_gib': round(available / 2**30, 1)})


def main():
    # run.sh ends a section with SIGTERM, on a timeout or Ctrl-C: leave through the finally blocks, so that the cells
    # and the spike are killed at once (they would die with the section anyway).
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    try:
        pid, name = compositor()
    except Exception as e:
        return fail(STILL + IDLE + LOAD, e)
    try:
        still(pid, name, float(env('SCREEN_STILL_S', '60')))
    except Exception as e:
        fail(STILL, e)
    seconds = float(env('FRAMES_S', '20'))
    try:
        binary = build()
        idle(binary, seconds)
    except Exception as e:
        # The loaded run would fail the same way, and is not worth loading every CPU for.
        return fail(IDLE + LOAD, e)
    try:
        loaded(binary, seconds)
    except Exception as e:
        fail(LOAD, e)


if __name__ == '__main__':
    main()
