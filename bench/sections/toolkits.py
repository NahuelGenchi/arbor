"""10. The toolkits of issue #8: the same Mission Control test program, built with each candidate toolkit
(toolkits/<name>/, described in toolkits/README.md) and measured the same way on this PC, one toolkit after the other.
It runs only when named in ONLY (ONLY=toolkits): it takes about four minutes a toolkit and needs each toolkit's
development files.

toolkits/build.sh builds the programs into the temp dir, or TOOLKITS_PREBUILT names a folder it built them in before.
Each program takes one argument: `still` shows the window with its 300 rows, changes 30 of them every 100 ms from 3 s to
5 s, lets a card arrive in 200 ms at 5 s, and then changes nothing; `animate` moves the card up and down without end
(200 ms each way) while 30 rows change every 100 ms. For each toolkit T in TOOLKITS:
  ui.T.first_frame_ms     from starting the program (`still`) to the compositor showing its window's first frame: the
                          median of TOOLKITS_LAUNCHES launches, after one more that is left out (it fills the caches)
  ui.T.rss_start_mib, ui.T.pss_start_mib
                          the memory of its processes 2.5 s after it started (/proc/<pid>/smaps_rollup)
  ui.T.still_cpu_pct, ui.T.still_wakeups_s
                          its CPU time (all its threads, in % of one core) and its wakeups (voluntary context switches
                          summed over its threads) over TOOLKITS_STILL_S seconds of still screen that start 8 s after
                          it started, after the updates and the card. Both fail when a key or the mouse was used in
                          the window (GNOME's idle monitor), as in the screen section
  ui.T.rss_idle_mib, ui.T.pss_idle_mib
                          its memory at the end of that window
  ui.T.missed_idle_pct, ui.T.frame_p99_idle_ms
                          frame pacing during the animation (`animate`), with the machine as it is: the screen
                          section's method and maths, from the time the compositor showed each new frame of the
                          window, which src/probe.c, loaded into the program (LD_PRELOAD), asks it for. 1 s of warm-up
                          after the first frame, then FRAMES_S seconds
  ui.T.frame_work_p99_ms  the work of each frame in that run: the CPU time of the program, all its threads, from
                          committing one frame of the window to committing the next (the probe reads the program's
                          CPU clock at each commit), the slowest 1%; the note gives the mean
  ui.T.missed_load_pct, ui.T.frame_p99_load_ms
                          the same with a busy cell per CPU thread and a memory spike, the screen section's load
  ui.T.atspi_objects      the accessible objects it shows on the accessibility bus (AT-SPI) 8 s after it started, once
                          its card has arrived and its rows have stopped changing; the note says whether its heading,
                          its card and the words of its rows can be read there. The program runs in a D-Bus session
                          of its own, in which only the accessibility bus can be started, with accessibility turned
                          on there, its settings in memory and a runtime folder of its own (the accessibility bus puts
                          its socket in $XDG_RUNTIME_DIR/at-spi, where the desktop's own is), so the desktop's
                          accessibility bus and setting are never touched
TOOLKITS_SETTLE_S seconds (90) pass after the builds and after each toolkit's loaded run, and then the section waits
until the 1-minute load average is under 1, at most 10 minutes more, so that each toolkit is measured on a machine as
quiet as the one before; each row's context gives the load average it was measured at. The programs keep their caches
(Mesa's shaders, the toolkits' pipelines) in the temp dir (XDG_CACHE_HOME), which the launch left out of the first
frame fills. Every window is the section's own, and every program, cell and spike ends with the section, also on a
timeout or Ctrl-C. As in the screen section, it reads process and thread names and timings, never a window title or
anything on the screen; the accessibility check reads only the section's own program.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', 'lib'))
sys.path.insert(0, HERE)
from common import BENCH_DIR, WORK, emit, env, fail, median, pct, run, smaps_rollup  # noqa: E402
from sysinfo import busiest, cpu_ticks  # noqa: E402
import screen  # noqa: E402  the screen section: its cells, spike and frame-pacing maths

NAMES = {'gtk4': 'GTK 4', 'qtquick': 'Qt Quick', 'slint': 'Slint', 'iced': 'iced', 'flutter': 'Flutter'}
METRICS = ['first_frame_ms', 'rss_start_mib', 'pss_start_mib', 'still_cpu_pct', 'still_wakeups_s', 'rss_idle_mib',
           'pss_idle_mib', 'missed_idle_pct', 'frame_p99_idle_ms', 'frame_work_p99_ms', 'missed_load_pct',
           'frame_p99_load_ms', 'atspi_objects']
HZ = os.sysconf('SC_CLK_TCK')
START_S = 2.5   # memory after the start: the rows are there, the updates not begun
STILL_AT = 8.0  # the still window and the accessibility check: after the updates (3–5 s) and the card's 200 ms
D = os.path.join(WORK, 'toolkits')
PROBE = os.path.join(WORK, 'bin', 'probe.so')
DRIVERS = re.compile(r'/(libvulkan_\w+|libgallium[-\w.]*?|\w+_dri)\.so')
# What the accessibility check looks for: the heading, the card, and each row's title and words.
TITLES = ['Help pages', 'Test suite', 'Bug reports', 'Website', 'Invoices', 'Payments', 'Newsletter', 'Photo archive',
          'Sales summary', 'Translations', 'Release notes', 'Search index']
UNITS = ['pages translated', 'tests faster', 'reports sorted', 'pages lighter', 'invoices filed', 'files moved',
         'emails ready', 'photos sorted', 'rows summed', 'strings done', 'notes written', 'files indexed']


def rows(t, metrics=METRICS):
    return [f'ui.{t}.{m}' for m in metrics]


def stop(p):
    if p.poll() is None:
        p.terminate()
        try:
            p.wait(3)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()


def program_env():
    """The environment of a test program: the desktop's, with the caches it writes in the temp dir."""
    return dict(os.environ, XDG_CACHE_HOME=os.path.join(D, 'cache'))


def launch(argv, mode, probe=None):
    """Starts a program, its output going to its log; returns it, and the CLOCK_MONOTONIC ns just before it started."""
    e = program_env()
    if probe:
        e.update(LD_PRELOAD=PROBE, ARBOR_PROBE=probe)
    with open(os.path.join(D, 'logs', os.path.basename(argv[0]) + '.log'), 'ab') as log:
        t0 = time.monotonic_ns()
        p = subprocess.Popen([*argv, mode], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=e,
                             cwd=os.path.dirname(argv[0]), preexec_fn=screen.die_with_us)
    return p, t0


def sleep_until(ns):
    left = ns / 1e9 - time.monotonic()
    if left > 0:
        time.sleep(left)


def tree(pid):
    """pid and every process below it."""
    kids = {}
    for d in os.listdir('/proc'):
        if d.isdigit():
            try:
                with open(f'/proc/{d}/stat') as f:
                    st = f.read()
                kids.setdefault(int(st[st.rindex(')') + 2:].split()[1]), []).append(int(d))
            except (OSError, ValueError):
                pass
    out, todo = [], [pid]
    while todo:
        q = todo.pop()
        out.append(q)
        todo.extend(kids.get(q, []))
    return out


def gpu_memory(pid):
    """MiB of VRAM and of GTT (system memory mapped for the GPU) its DRM clients hold, where the driver says."""
    seen, total = set(), {}
    try:
        fds = os.listdir(f'/proc/{pid}/fdinfo')
    except OSError:
        return {}
    for fd in fds:
        try:
            with open(f'/proc/{pid}/fdinfo/{fd}') as f:
                text = f.read()
        except OSError:
            continue
        client = re.search(r'^drm-client-id:\s*(\d+)', text, re.M)
        if not client or client.group(1) in seen:
            continue
        seen.add(client.group(1))
        for region, n, unit in re.findall(r'^drm-total-(vram|gtt):\s*(\d+)\s*(\w*)', text, re.M):
            total[region] = total.get(region, 0) + int(n) * {'MiB': 1024, 'GiB': 2**20}.get(unit, 1)
    return {f'gpu_{k}_mib': round(v / 1024, 1) for k, v in total.items()}


def drivers(pid):
    """The GPU drivers the program has loaded (Vulkan, or Mesa's OpenGL), from its memory map."""
    try:
        with open(f'/proc/{pid}/maps') as f:
            return sorted(set(DRIVERS.findall(f.read())))
    except OSError:
        return []


def memory(pid):
    """RSS and PSS of the program's processes in MiB, with what they are made of."""
    m, procs = {}, 0
    for q in tree(pid):
        try:
            r = smaps_rollup(q)
        except OSError:
            continue
        procs += 1
        for k in ('Rss', 'Pss', 'Pss_Anon', 'Pss_File', 'Pss_Shmem'):
            m[k] = m.get(k, 0) + r.get(k, 0)
    if not procs:
        raise RuntimeError('the program ended before its memory was read')
    return {'rss': m['Rss'] / 1024, 'pss': m['Pss'] / 1024, 'procs': procs,
            'pss_anon_mib': round(m.get('Pss_Anon', 0) / 1024, 1), 'pss_file_mib': round(m.get('Pss_File', 0) / 1024, 1),
            'pss_shmem_mib': round(m.get('Pss_Shmem', 0) / 1024, 1), **gpu_memory(pid)}


def reading(pid):
    """The program's CPU ticks (all its threads, also those that ended), and each thread's name, wakeups and ticks."""
    with open(f'/proc/{pid}/stat') as f:
        st = f.read()
    fields = st[st.rindex(')') + 2:].split()
    threads = {}
    for tid in os.listdir(f'/proc/{pid}/task'):
        try:
            with open(f'/proc/{pid}/task/{tid}/status') as f:
                s = f.read()
            with open(f'/proc/{pid}/task/{tid}/stat') as f:
                ts = f.read()
        except OSError:  # the thread ended
            continue
        tf = ts[ts.rindex(')') + 2:].split()
        threads[tid] = (re.search(r'^Name:\s*(.*)$', s, re.M).group(1),
                        int(re.search(r'^voluntary_ctxt_switches:\s*(\d+)', s, re.M).group(1)),
                        int(tf[11]) + int(tf[12]))
    return {'t': time.monotonic(), 'ticks': int(fields[11]) + int(fields[12]), 'threads': threads,
            'gpu_ns': screen.gpu_ns(pid)}


def by_thread(a, b, field, seconds, top=3):
    """The threads that did the most between two readings, summed by name without its number: 'name 12.3/s'
    for wakeups (field 1), 'name 45%' of a core for CPU (field 2)."""
    use = {}
    for tid, t in b['threads'].items():
        d = t[field] - a['threads'][tid][field] if tid in a['threads'] else t[field]
        name = re.sub(r'[-:]?\d+$', '', t[0]) or t[0]
        use[name] = use.get(name, 0) + d
    out = sorted(use.items(), key=lambda x: -x[1])[:top]
    if field == 1:
        return ', '.join(f'{n} {c / seconds:.1f}/s' for n, c in out if c)
    return ', '.join(f'{n} {c / HZ / seconds * 100:.0f}%' for n, c in out if c)


# ---- what the probe wrote ----

def probe_lines(path, pid):
    try:
        with open(path) as f:
            lines = f.read().splitlines()
    except OSError:
        return {}
    out = {'p': [], 'd': [], 'c': [], 'error': []}
    for line in lines:
        f = line.split()
        if not f:
            continue
        if f[0] in ('start', 'connect', 'ready') and len(f) == 3 and int(f[1]) == pid:
            out[f[0]] = int(f[2])
        elif f[0] == 'clock' and len(f) == 2:
            out['clock'] = int(f[1])
        elif f[0] == 'p' and len(f) == 8:
            out['p'].append([int(x) for x in f[1:]])
        elif f[0] == 'd' and len(f) == 3:
            out['d'].append([int(x) for x in f[1:]])
        elif f[0] == 'c' and len(f) == 5:
            out['c'].append([int(x) for x in f[1:]])
        elif f[0] == 'error':
            out['error'].append(line[6:])
    out['p'].sort(key=lambda s: s[2])  # threads of the program may dispatch the probe's replies out of order
    return out


def first_shown(path, p, timeout=20):
    """Waits until the probe reports the window's first frame shown; returns the probe's lines."""
    limit, got = time.monotonic() + timeout, {}
    while time.monotonic() < limit:
        got = probe_lines(path, p.pid)
        if got.get('error'):
            raise RuntimeError('the probe could not watch the program: ' + got['error'][0])
        if got.get('p'):
            if got.get('clock') != time.CLOCK_MONOTONIC:
                raise RuntimeError(f"the compositor's presentation clock is {got.get('clock')}, not CLOCK_MONOTONIC")
            return got
        if p.poll() is not None:
            raise RuntimeError(f'the program ended (status {p.returncode}) before its window was shown; see its log')
        time.sleep(0.005)
    if not got.get('connect'):
        raise RuntimeError(f'the program did not connect to the Wayland compositor in {timeout} s')
    raise RuntimeError(f'the compositor showed no frame of the window in {timeout} s')


def as_frames(got):
    """The probe's lines as frames.c prints them, for the screen section's maths."""
    first = got['p'][0][2] if got['p'] else 0
    before = [c[3] for c in got['c'] if c[0] <= first]
    lines = [f"clock {got.get('clock')}",
             f"focus {before[-1] if before else -1} {got['c'][-1][3] if got['c'] else -1}"]
    lines += ['p ' + ' '.join(map(str, s[:6])) for s in got['p']]  # without the CPU time
    lines += ['d ' + ' '.join(map(str, s)) for s in got['d']]
    return '\n'.join(lines)


# ---- the measurements ----

def first_frame(t, argv, n):
    ms, parts = [], {'loaded': [], 'connected': [], 'committed': [], 'probe': []}
    for i in range(n + 1):
        path = os.path.join(D, f'{t}-first-{i}.txt')
        p, t0 = launch(argv, 'still', probe=path)
        try:
            got = first_shown(path, p)
        finally:
            stop(p)
        if i:
            ms.append((got['p'][0][2] - t0) / 1e6)
            parts['loaded'].append((got['start'] - t0) / 1e6)
            parts['connected'].append((got['connect'] - t0) / 1e6)
            parts['committed'].append((got['p'][0][1] - t0) / 1e6)
            parts['probe'].append((got['ready'] - got['connect']) / 1e6)
        time.sleep(1)
    emit(f'ui.{t}.first_frame_ms', median(ms), {
        'launches_ms': [round(x, 1) for x in ms], 'load1': screen.load1(),
        # medians, from its start: the program was loaded (before main), connected, committed the first frame
        **{f'{k}_ms': round(median(v), 2 if k == 'probe' else 1) for k, v in parts.items()}})


def still(t, argv, comp, seconds):
    p, t0 = launch(argv, 'still')
    try:
        sleep_until(t0 + START_S * 1e9)
        start = memory(p.pid)
        loaded = drivers(p.pid)
        sleep_until(t0 + STILL_AT * 1e9)
        before = screen.load1()
        a, ca = reading(p.pid), screen.reading(comp[0])
        time.sleep(seconds)
        b, cb = reading(p.pid), screen.reading(comp[0])
        idle_ms = screen.idle_ms()
        end = memory(p.pid)
        if p.poll() is not None:
            raise RuntimeError(f'the program ended (status {p.returncode}) on the still screen; see its log')
    finally:
        stop(p)
    for when, m in (('start', start), ('idle', end)):
        ctx = {k: v for k, v in m.items() if k not in ('rss', 'pss')}
        emit(f'ui.{t}.rss_{when}_mib', m['rss'], dict(ctx, drivers=loaded))
        emit(f'ui.{t}.pss_{when}_mib', m['pss'], ctx)
    dt = b['t'] - a['t']
    if idle_ms is not None and idle_ms < dt * 1000:
        return fail(rows(t, ['still_cpu_pct', 'still_wakeups_s']),
                    f'a key or the mouse was used {idle_ms / 1000:.0f} s before the end of the {dt:.0f} s window, so '
                    'the screen was not still: leave the PC alone during this section')
    wakeups = sum(w - (a['threads'][tid][1] if tid in a['threads'] else 0) for tid, (_, w, _) in b['threads'].items())
    cpu = (b['ticks'] - a['ticks']) / HZ / dt * 100
    comp_cpu = (cb['ticks'] - ca['ticks']) / HZ / dt * 100
    comp_wakeups = sum(w - (ca['threads'][tid][1] if tid in ca['threads'] else 0) for tid, (_, w) in cb['threads'].items())
    gpu = None if a['gpu_ns'] is None or b['gpu_ns'] is None else round((b['gpu_ns'] - a['gpu_ns']) / 1e6, 1)
    ctx = {'window_s': round(dt, 1), 'threads': len(b['threads']), 'gpu_ms': gpu, 'compositor': comp[1],
           'compositor_cpu_pct': round(comp_cpu, 2), 'compositor_wakeups_s': round(comp_wakeups / dt, 1),
           'rest_cpu_pct': round((cb['machine'] - ca['machine']) / HZ / dt * 100 - cpu - comp_cpu, 1),
           'busy': busiest(ca['procs'], cb['procs'], dt, floor=10), 'load1': before,
           'input': 'none in the window' if idle_ms is not None else 'not checked: no GNOME idle monitor'}
    emit(f'ui.{t}.still_cpu_pct', cpu, dict(ctx, by_thread=by_thread(a, b, 2, dt)))
    emit(f'ui.{t}.still_wakeups_s', wakeups / dt, dict(ctx, by_thread=by_thread(a, b, 1, dt)))


def pacing(t, argv, seconds, loaded):
    keys = rows(t, ['missed_load_pct', 'frame_p99_load_ms'] if loaded else
                ['missed_idle_pct', 'frame_p99_idle_ms', 'frame_work_p99_ms'])
    path = os.path.join(D, f"{t}-{'load' if loaded else 'idle'}.txt")
    before, procs, spike, spike_s, t_spike, mib, available = screen.load1(), [], None, None, None, None, None
    n = int(env('FRAMES_CELLS') or os.cpu_count())
    try:
        if loaded:
            screen.start_cells(procs, n)
            mib, available = screen.spike_mib()
        p, _ = launch(argv, 'animate', probe=path)
        procs.append(p)
        first = first_shown(path, p)['p'][0][2]
        sleep_until(first + screen.WARMUP * 1e9)
        a, procs_a = reading(p.pid), cpu_ticks()
        if loaded:
            sleep_until(first + (screen.WARMUP + screen.SPIKE_AT) * 1e9)
            t_spike = time.monotonic_ns()
            spike = screen.start_spike(mib, seconds + 10)
            procs.append(spike)
        sleep_until(first + (screen.WARMUP + seconds) * 1e9)
        b, procs_b = reading(p.pid), cpu_ticks()
        time.sleep(0.5)  # the replies about the last frames
        if spike:
            spike_s = screen.spike_ended(spike, mib)
        if p.poll() is not None:
            raise RuntimeError(f'the program ended (status {p.returncode}) during the animation; see its log')
    finally:
        for q in procs:
            q.kill()
        for q in procs:
            q.wait()
    got = probe_lines(path, p.pid)
    res = screen.pacing_of(as_frames(got), seconds, until=seconds)
    dt, frames = b['t'] - a['t'], res['ctx']['frames']
    ctx = dict(res['ctx'], load1=before, busy=busiest(procs_a, procs_b, dt, floor=10),
               window=f"{got['c'][-1][1]}×{got['c'][-1][2]}" if got['c'] else None,
               cpu_pct=round((b['ticks'] - a['ticks']) / HZ / dt * 100, 1), threads=by_thread(a, b, 2, dt))
    if a['gpu_ns'] is not None and b['gpu_ns'] is not None and frames:
        ctx['gpu_ms_per_frame'] = round((b['gpu_ns'] - a['gpu_ns']) / 1e6 / frames, 2)
    if loaded:
        during = None
        if spike_s is not None:
            during = sum(c - 1 for at, c in res['steps'] if t_spike <= at <= t_spike + spike_s * 1e9)
        ctx.update(cells=n, spike_mib=mib, spike_s=spike_s, missed_during_spike=during,
                   available_gib=round(available / 2**30, 1))
    emit(keys[0], res['missed_pct'], ctx)
    emit(keys[1], res['p99_ms'], ctx)
    if not loaded:
        emit(keys[2], *work(got, seconds))


def work(got, seconds):
    """The work of each frame shown in the measured window, the CPU time of the program from committing the frame
    before (the next lower number) to committing it: the slowest 1%, and the mean in the context."""
    start = got['p'][0][2] + screen.WARMUP * 1e9
    cpu = {s[0]: s[6] for s in got['p'] if start <= s[2] <= start + seconds * 1e9 and s[6]}
    first = {s[0]: s[6] for s in got['p'] if s[6]}
    ms = [(c - first[f - 1]) / 1e6 for f, c in cpu.items() if f - 1 in first]
    if not ms:
        raise RuntimeError('the probe read no CPU time at the frames of the window')
    return pct(ms, 0.99), {'frames': len(ms), 'mean_ms': round(sum(ms) / len(ms), 3),
                           'median_ms': round(median(ms), 3), 'worst_ms': round(max(ms), 2)}


# ---- accessibility ----

def bus_config(runtime):
    """A session bus, listening in runtime, on which only the accessibility bus can be started: a program that asks for
    a portal or any other service hears at once that there is none, and nothing else of the desktop's starts twice."""
    service = '/usr/share/dbus-1/services/org.a11y.Bus.service'
    try:
        with open(service) as f:
            launcher = re.search(r'^Exec=(\S+)', f.read(), re.M).group(1)
    except (OSError, AttributeError):
        raise RuntimeError(f'no accessibility bus to start: {service} is missing (the at-spi2-core package)')
    services = os.path.join(D, 'a11y', 'services')
    os.makedirs(services, exist_ok=True)
    with open(os.path.join(services, 'org.a11y.Bus.service'), 'w') as f:
        f.write(f'[D-BUS Service]\nName=org.a11y.Bus\nExec={launcher}\n')
    config = os.path.join(D, 'a11y', 'session.conf')
    with open(config, 'w') as f:
        f.write(f'''<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={runtime}</listen>
  <auth>EXTERNAL</auth>
  <servicedir>{services}</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
''')
    return config


def atspi(t, argv):
    """The program in a D-Bus session of its own, with accessibility turned on there: what it shows on AT-SPI."""
    dbus = shutil.which('dbus-run-session')
    if not dbus:
        raise RuntimeError('dbus-run-session not found (the dbus package)')
    # The accessibility bus's socket goes in $XDG_RUNTIME_DIR/at-spi, where the desktop's own is: a runtime folder of
    # its own, and the Wayland socket by its full path. Turning accessibility on writes a setting: GSettings keeps it
    # in memory. Without DISPLAY, the bus launcher cannot announce the private bus on the X display, where every X
    # client would find it.
    runtime = os.path.join(D, 'a11y', 'run')
    os.makedirs(runtime, mode=0o700, exist_ok=True)
    if len(runtime) > 80:  # a socket's path has room for 107 bytes
        raise RuntimeError(f'the temp dir is too deep for the sockets of the accessibility check ({runtime}): set '
                           'BENCH_TMP to a shorter folder')
    wayland = os.environ.get('WAYLAND_DISPLAY', '')
    e = dict(program_env(), GSETTINGS_BACKEND='memory', XDG_RUNTIME_DIR=runtime,
             WAYLAND_DISPLAY=wayland if wayland.startswith('/') else
             os.path.join(os.environ.get('XDG_RUNTIME_DIR', ''), wayland))
    e.pop('DISPLAY', None)
    r = run([dbus, f'--config-file={bus_config(runtime)}', '--', sys.executable, '-I', '-B',
             os.path.abspath(__file__), '--atspi', *argv], timeout=120, env=e)
    lines = [x for x in r.stdout.decode(errors='replace').splitlines() if x.startswith('{')]
    if not lines:
        raise RuntimeError('the accessibility check printed nothing: ' + screen.last_line(r.stderr))
    got = json.loads(lines[-1])
    if 'error' in got:
        raise RuntimeError(got['error'])
    emit(f'ui.{t}.atspi_objects', got.pop('objects'), got)


def atspi_inside(argv):
    """In the private session: turn accessibility on, start the program, and walk its accessible objects."""
    r = run(['gdbus', 'call', '--session', '--dest', 'org.a11y.Bus', '--object-path', '/org/a11y/bus', '--method',
             'org.freedesktop.DBus.Properties.Set', 'org.a11y.Status', 'IsEnabled', '<true>'], timeout=30)
    if r.returncode != 0:
        return print(json.dumps({'error': 'cannot turn accessibility on: ' + screen.last_line(r.stderr)}))
    p = subprocess.Popen([*argv, 'still'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, cwd=os.path.dirname(argv[0]), preexec_fn=screen.die_with_us)
    try:
        import gi
        gi.require_version('Atspi', '2.0')
        from gi.repository import Atspi
        time.sleep(STILL_AT)
        app, limit = None, time.monotonic() + 5
        while app is None and time.monotonic() < limit:
            desktop = Atspi.get_desktop(0)
            for i in range(desktop.get_child_count()):
                try:
                    c = desktop.get_child_at_index(i)
                    if c is not None and c.get_process_id() == p.pid:
                        app = c
                except Exception:
                    pass
            if app is None:
                time.sleep(0.25)
        if app is None:
            return print(json.dumps({'objects': 0, 'registered': False}))
        words, roles, n, todo = set(), {}, 0, [(app, 0)]
        while todo and n < 5000:
            node, depth = todo.pop()
            n += 1
            try:
                role = node.get_role_name()
                roles[role] = roles.get(role, 0) + 1
                words.add((node.get_name() or '').strip())
                text = node.get_text_iface()
                if text is not None:
                    words.add((Atspi.Text.get_text(text, 0, -1) or '').strip())
                count = node.get_child_count() if depth < 60 else 0
            except Exception:
                continue
            for i in range(count):
                try:
                    c = node.get_child_at_index(i)
                except Exception:
                    c = None
                if c is not None:
                    todo.append((c, depth + 1))
        joined = '\n'.join(words)
        print(json.dumps({
            'objects': n, 'registered': True, 'toolkit': app.get_toolkit_name(),
            'heading': '3 things need you' in joined, 'card': 'Payments, moved to version 2' in joined,
            'row_titles': sum(bool(re.search(rf'{re.escape(TITLES[i % 12])} {i // 12 + 1}(?!\d)', joined))
                              for i in range(300)),
            'row_words': len(re.findall(r'\d+ of \d+ (?:' + '|'.join(UNITS) + ')', joined)),
            'roles': ', '.join(f'{k} {v}' for k, v in sorted(roles.items(), key=lambda x: -x[1])[:8])}))
    except Exception as ex:
        print(json.dumps({'error': f'the accessibility check failed: {ex}'}))
    finally:
        stop(p)


# ---- building ----

def build(names):
    """The programs, {toolkit: argv}, and why the others could not be built, {toolkit: text}."""
    os.makedirs(os.path.dirname(PROBE), exist_ok=True)
    r = run([os.environ.get('CC') or 'gcc', '-O2', '-shared', '-fPIC', '-fvisibility=hidden', '-pthread', '-o', PROBE,
             os.path.join(BENCH_DIR, 'src', 'probe.c')])
    if r.returncode != 0:
        raise RuntimeError('cannot build src/probe.c: ' + screen.last_line(r.stderr))
    out, errors = env('TOOLKITS_PREBUILT'), {}
    if not out:
        out = os.path.join(D, 'build')
        r = run(['bash', os.path.join(BENCH_DIR, 'toolkits', 'build.sh'), out, *names], timeout=2400)
        for line in r.stdout.decode(errors='replace').splitlines():
            if line.startswith('failed '):
                name, _, why = line[7:].partition(': ')
                errors[name] = why
    programs = {}
    for t in names:
        path = os.path.join(out, t, 'bin')
        if t in errors:
            continue
        if os.access(path, os.X_OK):
            programs[t] = [os.path.realpath(path)]
        else:
            errors[t] = f'no program at {path}'
    return programs, errors


def settle_down(seconds, limit_s=600):
    """Waits the seconds, and then until the 1-minute load average is under 1, at most limit_s more."""
    time.sleep(seconds)
    limit = time.monotonic() + limit_s
    while os.getloadavg()[0] >= 1 and time.monotonic() < limit:
        time.sleep(5)


def main():
    # run.sh ends a section with SIGTERM, on a timeout or Ctrl-C: leave through the finally blocks, so that the
    # programs, the cells and the spike are killed at once (they would die with the section anyway).
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    names = (env('TOOLKITS') or ' '.join(NAMES)).split()
    unknown = [t for t in names if t not in NAMES]
    if unknown:
        return fail('section.toolkits', f"unknown toolkits in TOOLKITS: {' '.join(unknown)} (known: {' '.join(NAMES)})")
    os.makedirs(os.path.join(D, 'logs'), exist_ok=True)
    try:
        comp = screen.compositor()
        programs, errors = build(names)
    except Exception as e:
        return fail([k for t in names for k in rows(t)], e)
    settle = float(env('TOOLKITS_SETTLE_S', '90'))
    if not env('TOOLKITS_PREBUILT'):
        settle_down(settle)  # after the builds
    launches, still_s = int(env('TOOLKITS_LAUNCHES', '5')), float(env('TOOLKITS_STILL_S', '60'))
    seconds = float(env('FRAMES_S', '20'))
    for i, t in enumerate(names):
        if t in errors:
            fail(rows(t), f'{NAMES[t]} could not be built: {errors[t]}')
            continue
        argv = programs[t]
        steps = ((['first_frame_ms'], lambda: first_frame(t, argv, launches)),
                 (METRICS[1:7], lambda: still(t, argv, comp, still_s)),
                 (['atspi_objects'], lambda: atspi(t, argv)),
                 (['missed_idle_pct', 'frame_p99_idle_ms', 'frame_work_p99_ms'], lambda: pacing(t, argv, seconds, False)),
                 (['missed_load_pct', 'frame_p99_load_ms'], lambda: pacing(t, argv, seconds, True)))
        for metrics, step in steps:
            try:
                step()
            except Exception as e:
                fail(rows(t, metrics), e)
        if i < len(names) - 1:
            settle_down(settle)  # the loaded run's cells and spike: let the machine come back


if __name__ == '__main__':
    if sys.argv[1:2] == ['--atspi']:
        atspi_inside(sys.argv[2:])
    else:
        main()
