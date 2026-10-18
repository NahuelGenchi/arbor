"""11. Mission Control, the first app of issue #9 (prototype/mission), measured as the toolkits section measures the
test programs of issue #8, on arbor state made for it in the temp dir. It runs only when named in ONLY (ONLY=mission):
about fifteen minutes.

It builds arbor and arbor-mission (cargo, release, offline) into the temp dir, or ARBOR_BIN and MISSION_BIN name them.
Then, in a state folder of its own (ARBOR_HOME in the temp dir), it makes what the window reads, with arbor itself:
four tries of made-up folders, two effects waiting for a person, a kept try and about twenty steps of history. The
window runs as `arbor-mission --bench still` (the window as it is) and `--bench animate` (its first line moves as the
test programs' card does, and it reads ten times as often as it would: the ledger's new steps every 100 ms, and
the rooms every second). Two states, the first of them twice:
  ui.mission.*       the state above, with nothing running: the still screen has nothing to show
  ui.mission_cairo.* the same, drawn by GTK's cairo renderer (GSK_RENDERER=cairo), which loads no GPU driver: what the
                     driver's libraries add to the window's memory, and what drawing on the CPU costs its frames
  ui.mission_busy.*  the same with MISSION_ROOMS (300) more tries, a room in each (a sealed cell with a memory limit,
                     which sleeps), while another try runs a short command every 0.2 s, so the ledger moves: the
                     brief's "busy but calm" with 300 agents, whose rooms the window reads every 10 s
Each gets the toolkits section's rows (first frame, memory 2.5 s after the start and after the still minute, still CPU
and wakeups, frame pacing idle and under load, work per frame, AT-SPI objects), and:
  ui.T.still_frames_s     the frames the compositor showed of the window a second during the still minute, through
                          the probe (src/probe.c)
The AT-SPI check looks for the window's own words: its heading, the two effects' commands and the tries' names. The
cells need a systemd user manager for their cgroups: the section starts `arbor serve` if it is not running and stops it
again, as the arbor section does. Every cell, window and spike ends with the section, also on a timeout or Ctrl-C.
"""
import os
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', 'lib'))
sys.path.insert(0, HERE)
from common import BENCH_DIR, WORK, env, fail, run  # noqa: E402
import screen  # noqa: E402
import toolkits  # noqa: E402

D = os.path.join(WORK, 'mission')
ROOMS = int(env('MISSION_ROOMS', '300'))
PAUSE_S = 0.2  # between the short commands of the busy state, each of which records two steps
METRICS = toolkits.METRICS + ['still_frames_s']


def build():
    """arbor and arbor-mission: ARBOR_BIN and MISSION_BIN, or a release build of prototype/ in the temp dir."""
    arbor, mission = env('ARBOR_BIN'), env('MISSION_BIN')
    if arbor and mission:
        return arbor, mission
    real = os.environ.get('CARGO_HOME') or os.path.expanduser('~/.cargo')
    cargo = shutil.which('cargo') or os.path.join(real, 'bin', 'cargo')
    if not os.access(cargo, os.X_OK):
        raise RuntimeError('cargo not found (or set ARBOR_BIN and MISSION_BIN)')
    # As in the arbor section: a CARGO_HOME of its own reads ~/.cargo's index, crates and config through links.
    target, home = os.path.join(D, 'target'), os.path.join(D, 'cargo')
    os.makedirs(os.path.join(home, 'registry', 'src'))
    for name in ('registry/index', 'registry/cache', 'config.toml', 'config'):
        if os.path.exists(os.path.join(real, name)):
            os.symlink(os.path.join(real, name), os.path.join(home, name))
    r = run([cargo, 'build', '--release', '--locked', '--offline', '--quiet', '-p', 'arbor', '-p', 'mission'],
            timeout=1200, cwd=os.path.join(BENCH_DIR, '..', 'prototype'),
            env=dict(os.environ, CARGO_HOME=home, CARGO_TARGET_DIR=target, RUSTUP_AUTO_INSTALL='0',
                     CARGO_BUILD_JOBS=os.environ.get('CARGO_BUILD_JOBS', '8')))
    if r.returncode != 0:
        raise RuntimeError('cargo build failed (it needs GTK 4\'s development files, and the crates fetched once with '
                           '`cargo fetch` in prototype/): ' + screen.last_line(r.stderr))
    shutil.rmtree(home, ignore_errors=True)
    return (arbor or os.path.join(target, 'release', 'arbor'),
            mission or os.path.join(target, 'release', 'arbor-mission'))


class Arbor:
    """arbor, on the section's own state folder."""

    def __init__(self, binary):
        self.binary = binary
        self.env = dict(os.environ, ARBOR_HOME=os.path.join(D, 'state'))

    def __call__(self, *args, cwd=None):
        r = run([self.binary, *args], timeout=60, env=self.env, cwd=cwd)
        if r.returncode != 0:
            raise RuntimeError(f"arbor {' '.join(args)} failed: {screen.last_line(r.stderr)}")
        return r.stdout.decode()

    def start(self, *args):
        return subprocess.Popen([self.binary, *args], env=self.env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, preexec_fn=screen.die_with_us)


def folders():
    projects = os.path.join(D, 'projects')
    files = {'help-pages/README.md': '# Help pages\n', 'shop/cart.rs': 'fn total() {}\n',
             'site/index.html': '<h1>Shop</h1>\n', 'notes/todo.txt': 'call the bookkeeper\n'}
    for path, text in files.items():
        os.makedirs(os.path.dirname(os.path.join(projects, path)), exist_ok=True)
        with open(os.path.join(projects, path), 'w') as f:
            f.write(text)
    return projects


def quiet_state(a):
    """Four tries, two effects waiting, a kept try: what the window shows, with nothing running."""
    projects = folders()
    for name in ('help-pages', 'shop', 'site', 'notes'):
        a('new', name, '--from', os.path.join(projects, name))
    a('run', 'shop', '--', 'sh', '-c', 'echo "fn total() -> u32 { 0 }" > cart.rs; mkdir tests; echo t > tests/cart.rs')
    a('run', 'shop', '--', 'arbor', 'effect', '--cost', '0.40', '--why', 'publish the fix for rounding the total', '--',
      'git', 'push', 'origin', 'fix')
    a('run', 'site', '--', 'sh', '-c', 'echo "<img loading=lazy>" >> index.html')
    a('run', 'site', '--', 'arbor', 'effect', '--why', 'deploy the faster gallery to the test site', '--', './deploy',
      '--to', 'test-site')
    a('run', 'notes', '--', 'sh', '-c', 'echo "call the bookkeeper on Monday" > todo.txt')
    a('keep', 'notes')
    return ['2 things need you', 'Nothing is running', 'git push origin fix', './deploy --to test-site', 'help-pages',
            'shop', 'site', 'notes']


def busy_state(a, procs):
    """ROOMS more tries with a room in each, and a try that runs a short command every PAUSE_S seconds."""
    projects = os.path.join(D, 'projects')
    for i in range(ROOMS):
        a('new', f'room-{i:03}', '--from', os.path.join(projects, 'help-pages'))
    a('new', 'ticker', '--from', os.path.join(projects, 'help-pages'))
    # A limit gives each cell a cgroup of its own, so the window reads what it uses; the first starts `arbor serve`.
    a('run', 'ticker', '--pids', '64', '--', 'true')
    for i in range(ROOMS):
        procs.append(a.start('run', f'room-{i:03}', '--mem', '256M', '--', 'sleep', '3600'))
    limit = time.monotonic() + 120
    while time.monotonic() < limit:
        running = len(a('ps').splitlines()) - 1
        if running >= ROOMS:
            break
        time.sleep(1)
    else:
        raise RuntimeError(f'only {running} of the {ROOMS} rooms started in 2 minutes')
    ticker = subprocess.Popen(
        ['sh', '-c', f'while :; do "$0" run ticker -- true >/dev/null 2>&1; sleep {PAUSE_S}; done', a.binary],
        env=a.env, stdin=subprocess.DEVNULL, preexec_fn=screen.die_with_us)
    procs.append(ticker)
    return ['2 things need you', 'rooms are working', 'room-000', 'git push origin fix']


def measure(t, argv, comp, expect):
    seconds, launches = float(env('FRAMES_S', '20')), int(env('TOOLKITS_LAUNCHES', '5'))
    still_s = float(env('TOOLKITS_STILL_S', '60'))
    steps = ((['first_frame_ms'], lambda: toolkits.first_frame(t, argv, launches)),
             (['rss_start_mib', 'pss_start_mib', 'still_cpu_pct', 'still_wakeups_s', 'rss_idle_mib', 'pss_idle_mib',
               'still_frames_s'], lambda: toolkits.still(t, argv, comp, still_s, frames=True)),
             (['atspi_objects'], lambda: toolkits.atspi(t, argv, expect)),
             (['missed_idle_pct', 'frame_p99_idle_ms', 'frame_work_p99_ms'],
              lambda: toolkits.pacing(t, argv, seconds, False)),
             (['missed_load_pct', 'frame_p99_load_ms'], lambda: toolkits.pacing(t, argv, seconds, True)))
    for metrics, step in steps:
        try:
            step()
        except Exception as e:
            fail(toolkits.rows(t, metrics), e)


def service():
    return run(['systemctl', '--user', 'is-active', 'arbor'], timeout=30).stdout.decode().strip() or None


def main():
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    os.makedirs(D, exist_ok=True)
    os.makedirs(os.path.join(toolkits.D, 'logs'), exist_ok=True)
    keys = [k for t in ('mission', 'mission_cairo', 'mission_busy') for k in toolkits.rows(t, METRICS)]
    try:
        comp = screen.compositor()
        binary, mission = build()
        toolkits.build_probe()
    except Exception as e:
        return fail(keys, e)
    a = Arbor(binary)
    # The window reads the section's state folder: ARBOR_HOME reaches it through the environment of every launch.
    os.environ['ARBOR_HOME'] = a.env['ARBOR_HOME']
    argv = [mission, '--bench']
    settle = float(env('TOOLKITS_SETTLE_S', '90'))
    procs, before = [], service()
    try:
        try:
            expect = quiet_state(a)
        except Exception as e:
            return fail(keys, e)
        toolkits.settle_down(settle)
        measure('mission', argv, comp, expect)
        toolkits.settle_down(settle)  # the loaded run's cells and spike: let the machine come back
        renderer = os.environ.get('GSK_RENDERER')
        os.environ['GSK_RENDERER'] = 'cairo'
        try:
            measure('mission_cairo', argv, comp, expect)
        finally:
            if renderer is None:
                del os.environ['GSK_RENDERER']
            else:
                os.environ['GSK_RENDERER'] = renderer
        try:
            expect = busy_state(a, procs)
        except Exception as e:
            return fail(toolkits.rows('mission_busy', METRICS), e)
        toolkits.settle_down(settle)
        measure('mission_busy', argv, comp, expect)
    finally:
        for p in procs:
            p.terminate()  # an arbor that runs a cell stops it and records how it ended
        for p in procs:
            try:
                p.wait(10)
            except subprocess.TimeoutExpired:
                p.kill()
        if before != 'active' and service() == 'active':
            run(['systemctl', '--user', 'stop', 'arbor'], timeout=60)


if __name__ == '__main__':
    main()
