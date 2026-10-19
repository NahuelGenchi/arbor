"""12. The five goals of 18 Oct 2026, scored over a whole stretch of work: today's Ubuntu against the same work done
with arbor, on the whole system. It runs only when named in ONLY (ONLY=goals): about 45 minutes. GOALS_PARTS picks the
parts (all five when empty): session, power, safety, ease and still. Issue #85 decided the rows before the first run.

The work is this repository at GOALS_REF, cloned into a temp folder on disk (GOALS_TMP, beside the reports: builds then
write a real disk, not tmpfs), with its Rust workspace (prototype/) built once beforehand, as in a project one has been
working on. cargo runs offline, with a CARGO_HOME of its own in that folder that reads ~/.cargo's index, crates and
config through links, as in the arbor section. No model runs: the jobs are the commands an agent runs. Three arms:
  worktree  each try in a `git worktree` (what coding agents use now): it starts without the folder's build cache, and
            keeping it is a commit and a merge
  copy      each try in a `cp -a` copy of the folder, build cache included; keeping it is a commit and a pull
  arbor     each try in a world (`arbor new`), its commands in a cell under a key that opens the toolchains, kept with
            `arbor keep` and thrown away with `arbor discard`; cells get cgroups of their own under `arbor serve`,
            which the section starts if it is not running and stops again, as the arbor section does
Between two runs the folder goes back to GOALS_REF and its build is brought up to date, outside the measured window.

session  Three tries in parallel, each: 5 searches (grep -rn), an edit, `cargo build --workspace`, `cargo test -p arbor
         --lib`, a second edit, the build and the tests again, `git status` and `git diff`. Then the first try is kept
         and the other two are thrown away, and `sync` ends the window. One warm-up run of each arm, then GOALS_RUNS (3)
         measured runs, the arms interleaved; each row is the median of the runs, with every run in its ctx:
  goal.session.ARM.mem_peak_mib   memory in use, swap included (MemTotal - MemAvailable + SwapTotal - SwapFree, read
                                  10 times a second), its peak above the mean of the 2 s before the window
  goal.session.ARM.mem_mean_mib   the same, its mean over the window
  goal.session.ARM.swap_mib       pages swapped out (pswpout: to zram and to disk)
  goal.session.ARM.disk_mib       written to the disks (/proc/diskstats, whole disks)
  goal.session.ARM.taken_mib      disk the tries took when all three had run (the filesystem's used blocks)
  goal.session.ARM.wall_s         wall time of the window
  goal.session.ARM.cpu_s          CPU time of the whole system (all cores) over the window
  goal.session.ARM.energy_j       package energy (RAPL) over the window, where this user may read it
         Each row's ctx also has the disks' cache flushes over each run and the seconds they took: in a world, every
         file a build links, renames or rewrites from the folder below is copied up by overlayfs and fsynced (#90).
power    Five tries of a change in parallel (an edit, then `cargo test -p arbor --lib`), keep one and throw four away,
         timed end to end (goal.power.ARM.tries5_s, GOALS_RUNS runs interleaved). Then undoing a kept try, in worktree
         and arbor: a try changes a tracked file, adds a file, deletes a tracked file, changes an ignored file (.env)
         and writes a file outside the project; it is kept, then undone (`git reset --hard ORIG_HEAD`; `arbor undo`).
  goal.power.ARM.kept       of the four kinds inside the project, how many the keep brought into the folder
  goal.power.ARM.escaped    changes that reached outside the try without being kept (the file outside)
  goal.power.ARM.put_back   of the five places, how many are as before once the try is undone
  goal.power.ARM.undo_ms    the undo, timed
safety   The same probes run as you (today) and in a cell under a typical agent's key (`--toolchains --home`, and
         --mem 256M), each reached or blocked: a password file in your home folder (1 byte read from each that exists,
         never kept), agents' transcripts (listed, never read), the rest of your home folder (1 byte of bench/README.md),
         writing the project's real folder, writing outside it, this computer's network address and a service on
         localhost (servers of the section, answering only this computer), the Docker socket and the session bus
         (connected to, nothing sent), signalling another program of yours, an effect (`touch` of a file outside, a
         stand-in for `git push`) run without a yes, taking 512 MiB of memory, and the kernel's keyrings (keyctl).
  goal.safety.today.reached, goal.safety.arbor.reached   how many of the probes that apply here reached; each probe's
                                                         result is in the ctx
ease     Commands a person types, counted from the procedures the section runs, and only when they succeeded:
  goal.ease.start.arbor      start an agent safely: a world, a key that opens its toolchains, one host and a home of its
                             own, and the agent in a cell (`arbor run` in place of the interactive `arbor shell`)
  goal.ease.approve.arbor    approve one effect an agent asked for: `arbor outbox`, `arbor release`
  goal.ease.keep.ARM, goal.ease.undo.ARM, goal.ease.discard4.ARM   keep a try, undo it, throw four away
  goal.ease.start.today, goal.ease.approve.today, goal.ease.approve.mission   no count: what is there instead
still    The desktop left alone, in GOALS_STILL_BLOCKS (2) blocks of GOALS_STILL_S (150) seconds per arm, interleaved:
         today (no `arbor serve` started by the section, no Mission Control) and arbor (`arbor serve` running and
         Mission Control open on the section's state, which needs a GTK 4 build of it). A block with a key press or a
         mouse move (GNOME's idle monitor) is taken again once nobody has touched the PC for 30 s, at most 3 times,
         then fails, so the PC must be left alone.
  goal.still.ARM.wakeups_s   interrupts a second, all CPUs (/proc/interrupts: timer, rescheduling, devices)
  goal.still.ARM.busy_pct    CPU busy, share of all cores
  goal.still.ARM.deep_pct    time in the deepest idle state, share of all cores (cpuidle)
  goal.still.ARM.power_w     package power (RAPL), where this user may read it; the GPU's board power, which amdgpu
                             lets anyone read, is in the ctx of wakeups_s
The temp folder is removed at the end, also on a timeout or Ctrl-C, with every world, cell, server and window.
"""
import json
import os
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', 'lib'))
sys.path.insert(0, HERE)
from common import BENCH_DIR, emit, env, fail, run  # noqa: E402
import arbor as asec  # noqa: E402
import mission as msec  # noqa: E402
import screen  # noqa: E402

REF = env('GOALS_REF', '1e314a0ae6a8669098c8991cbcc94837e349f6e6')
RUNS = int(env('GOALS_RUNS', '3'))
STILL_S = float(env('GOALS_STILL_S', '150'))
STILL_BLOCKS = int(env('GOALS_STILL_BLOCKS', '2'))
PARTS = (env('GOALS_PARTS') or 'session power safety ease still').split()
ARMS = ('worktree', 'copy', 'arbor')
SESSION_METRICS = ('mem_peak_mib', 'mem_mean_mib', 'swap_mib', 'disk_mib', 'taken_mib', 'wall_s', 'cpu_s', 'energy_j')
RAPL = '/sys/class/powercap/intel-rapl:0'
TICK = os.sysconf('SC_CLK_TCK')
MIB = 2**20
GIT = dict(GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='bench', GIT_COMMITTER_NAME='bench',
           GIT_AUTHOR_EMAIL='bench@localhost', GIT_COMMITTER_EMAIL='bench@localhost')
CARGO = 'cargo build --offline --locked -q --workspace && cargo test --offline --locked -q -p arbor --lib >/dev/null'
# One agent-like try, in the folder $1, as try $2: searches, an edit, build and tests, a second edit, again, git.
SESSION_JOB = f'''set -eu
cd "$1/prototype"
for q in 'fn main' 'unwrap()' 'TODO' 'pub struct' 'Result<'; do
  grep -rn --exclude-dir=target --exclude-dir=.git -e "$q" .. >/dev/null || true
done
printf '\\n// try %s: first edit\\n' "$2" >> arbor/src/table.rs
{CARGO}
printf '\\npub const GOALS_TRY: u32 = %s;\\n' "$2" >> arbor/src/lib.rs
{CARGO}
git status --short >/dev/null
git diff --stat >/dev/null
'''
TRIES5_JOB = '''set -eu
cd "$1/prototype"
printf '\\n// try %s\\n' "$2" >> arbor/src/table.rs
cargo test --offline --locked -q -p arbor --lib >/dev/null
'''
# A try that changes five kinds of thing: in the folder $1, and a file outside it, $3 ($2 is the try's number).
UNDO_JOB = '''cd "$1" || exit 1
echo 'a line from the try' >> README.md
mkdir -p notes && echo 'new' > notes/try.txt
rm docs/TRACKING.md
echo changed > .env
mkdir -p "$(dirname "$3")" 2>/dev/null; echo changed > "$3" 2>/dev/null
exit 0
'''
# Run as you, or in a cell: each probe prints one JSON line as it finishes; the memory probe comes last, since the cell
# under a memory limit is stopped whole when it runs out.
PROBE = r'''
import ctypes, json, os, platform, socket, subprocess, sys
a = json.loads(sys.argv[1])
def say(name, reached, **kw):
    print(json.dumps(dict(probe=name, reached=reached, **kw)), flush=True)
def one_byte(path):
    try:
        with open(path, 'rb') as f:
            return len(f.read(1)) == 1
    except OSError:
        return False
say('password_files', sum(one_byte(p) for p in a['secrets']) > 0 if a['secrets'] else None,
    readable=sum(one_byte(p) for p in a['secrets']), of=len(a['secrets']))
def listed(d):
    try:
        return len(os.listdir(d)) > 0
    except OSError:
        return False
say('transcripts', any(listed(d) for d in a['transcripts']) if a['transcripts'] else None)
say('home_folder', one_byte(a['home_file']))
for name, path in (('write_project', a['project_file']), ('write_outside', a['outside_file'])):
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, 'w') as f:
            f.write('probe\n')
    except OSError:
        pass
    say(name, None)  # the section looks from outside
def pong(host, port):
    try:
        with socket.create_connection((host, port), timeout=3) as s:
            s.sendall(b'GET / HTTP/1.0\r\nHost: x\r\n\r\n')
            return s.recv(64).startswith(b'HTTP/1.1 200')
    except OSError:
        return False
say('network', pong(*a['lan']) if a['lan'] else None)
say('localhost', pong(*a['local']))
def unix(path):
    if not path:
        return None
    try:
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(3)
            s.connect(path)
            return True
    except OSError:
        return False
say('docker_socket', unix(a['docker']) if a['docker'] else None)
say('session_bus', unix(a['bus']) if a['bus'] else None)
try:
    os.kill(a['pid'], 0)
    say('signal_program', True)
except OSError:
    say('signal_program', False)
try:
    subprocess.run(['touch', a['marker']], stderr=subprocess.DEVNULL, timeout=30)
except OSError:
    pass
try:
    subprocess.run(['arbor', 'effect', '--why', 'goals probe', '--', 'touch', a['marker']], stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=30)
except OSError:
    pass
say('effect_without_yes', None)  # the section looks from outside
nr = {'x86_64': 250, 'aarch64': 219}.get(platform.machine())
if nr:
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    say('keyrings', libc.syscall(nr, 0, -3, 0) >= 0)  # KEYCTL_GET_KEYRING_ID of the session keyring
else:
    say('keyrings', None)
r = subprocess.run([sys.executable, '-c', 'b = b"x" * (512 << 20)'], stderr=subprocess.DEVNULL)
say('memory_512mib', r.returncode == 0)
'''
PROBES = ('password_files', 'transcripts', 'home_folder', 'write_project', 'write_outside', 'network', 'localhost',
          'docker_socket', 'session_bus', 'signal_program', 'effect_without_yes', 'keyrings', 'memory_512mib')
# What a person types for each step, per arm; Arm counts the same steps as it runs them.
TYPED = {
    'worktree': {'keep': ['git add -A', 'git commit', 'git merge BRANCH'],
                 'discard': ['git worktree remove --force DIR', 'git branch -D BRANCH'],
                 'undo': ['git reset --hard ORIG_HEAD']},
    'copy': {'keep': ['git add -A', 'git commit', 'git pull DIR HEAD'], 'discard': ['rm -rf DIR']},
    'arbor': {'keep': ['arbor keep W'], 'discard': ['arbor discard W'], 'undo': ['arbor undo W']},
}
# Places in a home folder where logins are kept (prototype/README.md, Keys), and where agents keep transcripts.
SECRETS = ('.ssh', '.gnupg', '.aws/credentials', '.config/gh/hosts.yml', '.git-credentials', '.netrc', '.npmrc',
           '.pypirc', '.docker/config.json', '.cargo/credentials.toml', '.claude/.credentials.json', '.claude.json',
           '.codex/auth.json', '.local/share/opencode/auth.json', '.local/share/keyrings')
TRANSCRIPTS = ('.claude/projects', '.codex/sessions', '.local/share/opencode/storage', '.gemini/tmp')


def sh(argv, cwd=None, extra=None, timeout=900):
    r = run(argv, cwd=cwd, env=dict(os.environ, **GIT, **(extra or {})), timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f'`{" ".join(map(str, argv))}` exited with {r.returncode}: {asec.last_line(r.stderr)}')
    return r.stdout.decode(errors='replace')


# Whole-system readings ---------------------------------------------------------------------------------------------

def mem_used():
    """MiB in use: memory (MemTotal - MemAvailable) and swap. A page swapped out leaves the first for the second, so
    under memory pressure the sum still shows what the work holds."""
    m = {}
    with open('/proc/meminfo') as f:
        for line in f:
            k, v = line.split(':')
            m[k] = int(v.split()[0])
    return (m['MemTotal'] - m['MemAvailable'] + m['SwapTotal'] - m['SwapFree']) / 1024


def vmstat(name):
    with open('/proc/vmstat') as f:
        for line in f:
            k, v = line.split()
            if k == name:
                return int(v)
    return 0


def disks():
    return [d for d in os.listdir('/sys/block') if not d.startswith(('loop', 'ram', 'zram', 'dm-', 'md', 'sr'))]


def sectors_written():
    names = set(disks())
    with open('/proc/diskstats') as f:
        return sum(int(p[9]) for p in (line.split() for line in f) if p[2] in names)


def flushes():
    """Cache flushes the disks completed and the seconds they took (/proc/diskstats, whole disks): each fsync ends in
    one, and in a world each file overlayfs copies up is fsynced (#90)."""
    names = set(disks())
    with open('/proc/diskstats') as f:
        ps = [p for p in (line.split() for line in f) if p[2] in names and len(p) >= 20]
    return sum(int(p[18]) for p in ps), sum(int(p[19]) for p in ps) / 1000


def cpu_busy_s():
    with open('/proc/stat') as f:
        t = [int(x) for x in f.readline().split()[1:9]]
    return (t[0] + t[1] + t[2] + t[5] + t[6] + t[7]) / TICK, sum(t) / TICK


def rapl():
    """Package energy in J, or the reason it cannot be read."""
    try:
        with open(os.path.join(RAPL, 'energy_uj')) as f:
            return int(f.read()) / 1e6, None
    except PermissionError:
        return None, ('package energy (RAPL) is readable only by root on this PC, and perf may not read it '
                      f"(kernel.perf_event_paranoid = {open('/proc/sys/kernel/perf_event_paranoid').read().strip()})")
    except OSError as e:
        return None, f'no package energy counter here: {e}'


def rapl_range():
    try:
        return int(open(os.path.join(RAPL, 'max_energy_range_uj')).read()) / 1e6
    except OSError:
        return None


def interrupts():
    n = 0
    with open('/proc/interrupts') as f:
        cpus = len(f.readline().split())
        for line in f:
            for x in line.split()[1:1 + cpus]:
                if x.isdigit():
                    n += int(x)
    return n


def deep_idle_us():
    """Time in the deepest idle state, summed over the CPUs, in µs."""
    total = 0
    for c in os.listdir('/sys/devices/system/cpu'):
        d = f'/sys/devices/system/cpu/{c}/cpuidle'
        if c.startswith('cpu') and c[3:].isdigit() and os.path.isdir(d):
            states = sorted(s for s in os.listdir(d) if s.startswith('state'))
            if states:
                total += int(open(os.path.join(d, states[-1], 'time')).read())
    return total


def gpu_w():
    for h in os.listdir('/sys/class/hwmon'):
        p = f'/sys/class/hwmon/{h}/power1_average'
        try:
            if open(f'/sys/class/hwmon/{h}/name').read().strip() == 'amdgpu':
                return int(open(p).read()) / 1e6
        except OSError:
            pass
    return None


class Sampler(threading.Thread):
    """Memory in use, 10 times a second, until stopped."""

    def __init__(self):
        super().__init__(daemon=True)
        self.xs, self.done = [], threading.Event()

    def run(self):
        while not self.done.is_set():
            self.xs.append(mem_used())
            self.done.wait(0.1)

    def stop(self):
        self.done.set()
        self.join()
        return self.xs


def fs_used(path):
    s = os.statvfs(path)
    return (s.f_blocks - s.f_bfree) * s.f_frsize


def settle():
    """Write out what earlier runs left in the page cache, and wait, at most 2 minutes, until the machine is at rest:
    under 5% of its CPU busy over 2 s. Returns that share, in %, as it was when the wait ended."""
    subprocess.run(['sync'])
    for _ in range(60):
        b0, t0 = cpu_busy_s()
        time.sleep(2)
        b1, t1 = cpu_busy_s()
        if (b1 - b0) / (t1 - t0) < 0.05:
            break
    return round((b1 - b0) / (t1 - t0) * 100, 1)


# The three arms -------------------------------------------------------------------------------------------------------

class Lab:
    def __init__(self, root, binary):
        self.root, self.binary = root, binary
        self.main = os.path.join(root, 'project')
        self.tries = os.path.join(root, 'tries')
        self.outside = os.path.join(root, 'outside')
        self.cargo = os.path.join(root, 'cargo')
        self.state = os.path.join(root, 'state')
        self.env = dict(os.environ, **GIT, CARGO_HOME=self.cargo, ARBOR_HOME=self.state,
                        CARGO_BUILD_JOBS=os.environ.get('CARGO_BUILD_JOBS', str(os.cpu_count())))
        os.makedirs(self.tries)
        os.makedirs(self.outside)

    def arbor(self, *args, environ=None):
        r = run([self.binary, *args], env=environ or self.env, stdin=subprocess.DEVNULL, timeout=900)
        if r.returncode != 0:
            raise RuntimeError(f'`arbor {" ".join(args)}` exited with {r.returncode}: {asec.last_line(r.stderr)}')
        return r.stdout.decode(errors='replace')

    def setup(self):
        """The project at REF, its workspace built, and a CARGO_HOME of its own."""
        real = os.environ.get('CARGO_HOME') or os.path.expanduser('~/.cargo')
        os.makedirs(os.path.join(self.cargo, 'registry', 'src'))
        for name in ('registry/index', 'registry/cache', 'config.toml', 'config'):
            if os.path.exists(os.path.join(real, name)):
                os.symlink(os.path.join(real, name), os.path.join(self.cargo, name))
        sh(['git', 'clone', '-q', '--no-checkout', os.path.join(BENCH_DIR, '..'), self.main])
        sh(['git', 'remote', 'remove', 'origin'], cwd=self.main)
        sh(['git', 'checkout', '-q', '-B', 'work', REF], cwd=self.main)
        with open(os.path.join(self.main, '.git', 'info', 'exclude'), 'a') as f:
            f.write('.env\n')
        self.warm()
        self.arbor('key', 'new', 'dev', '--toolchains', '--read', self.cargo, '--env', 'CARGO_HOME', '--for', '12h')

    def warm(self):
        r = run(['bash', '-c', CARGO.replace(' -q -p arbor --lib >/dev/null', ' -q -p arbor --lib --no-run')],
                cwd=os.path.join(self.main, 'prototype'), env=self.env, timeout=900)
        if r.returncode != 0:
            raise RuntimeError(f'the build of the project failed: {asec.last_line(r.stderr)}')

    def reset(self):
        """Back to REF with nothing else around, its build brought up to date; outside the measured window."""
        for w in self.worlds():
            self.arbor('discard', w)
        for d in os.listdir(self.tries):
            shutil.rmtree(os.path.join(self.tries, d))
        sh(['git', 'worktree', 'prune'], cwd=self.main)
        sh(['git', 'reset', '-q', '--hard', REF], cwd=self.main)
        sh(['git', 'clean', '-fdq'], cwd=self.main)
        for b in sh(['git', 'branch', '--format=%(refname:short)'], cwd=self.main).split():
            if b != 'work':
                sh(['git', 'branch', '-q', '-D', b], cwd=self.main)
        with open(os.path.join(self.main, '.env'), 'w') as f:
            f.write('original\n')
        with open(os.path.join(self.outside, 'note.txt'), 'w') as f:
            f.write('original\n')
        self.warm()

    def worlds(self):
        out = self.arbor('worlds').splitlines()[1:]
        return [line.split()[0] for line in out if line.split()]


class Arm:
    """A way to try changes: make a try, run a job in it, keep it, throw it away. `typed` counts what a person types."""

    def __init__(self, lab, name):
        self.lab, self.name, self.typed = lab, name, {}

    def count(self, what, n=1):
        self.typed[what] = self.typed.get(what, 0) + n

    def path(self, i):
        return self.lab.main if self.name == 'arbor' else os.path.join(self.lab.tries, f'{self.name[0]}{i}')

    def make(self, i):
        lab = self.lab
        if self.name == 'worktree':
            sh(['git', 'worktree', 'add', '-q', '-b', f't{i}', self.path(i), 'work'], cwd=lab.main)
        elif self.name == 'copy':
            sh(['cp', '-a', lab.main, self.path(i)])
        else:
            lab.arbor('new', f't{i}', '--from', lab.main)
        self.count('make')

    def start(self, i, job, key=True, args=()):
        argv = ['bash', '-c', job, 'bash', self.path(i), str(i), *args]
        if self.name == 'arbor':
            argv = [self.lab.binary, 'run', f't{i}', *(['--key', 'dev'] if key else []), '--', *argv]
        log = open(os.path.join(self.lab.root, f'{self.name}-{i}.log'), 'w')
        p = subprocess.Popen(argv, cwd=self.lab.root, env=self.lab.env, stdin=subprocess.DEVNULL, stdout=log,
                             stderr=subprocess.STDOUT)
        p.log = log.name
        return p

    def keep(self, i):
        lab, path = self.lab, self.path(i)
        if self.name == 'arbor':
            lab.arbor('keep', f't{i}')
            self.count('keep')
            return
        sh(['git', 'add', '-A'], cwd=path)
        sh(['git', 'commit', '-q', '-m', f'try {i}'], cwd=path)
        if self.name == 'worktree':
            sh(['git', 'merge', '-q', '--ff-only', f't{i}'], cwd=lab.main)
        else:
            sh(['git', 'pull', '-q', '--ff-only', path, 'HEAD'], cwd=lab.main)
        self.count('keep', 3)

    def throw(self, i):
        lab, path = self.lab, self.path(i)
        if self.name == 'worktree':
            sh(['git', 'worktree', 'remove', '--force', path], cwd=lab.main)
            sh(['git', 'branch', '-q', '-D', f't{i}'], cwd=lab.main)
            self.count('discard', 2)
        elif self.name == 'copy':
            sh(['rm', '-rf', '--', path])
            self.count('discard')
        else:
            lab.arbor('discard', f't{i}')
            self.count('discard')

    def undo(self):
        if self.name == 'arbor':
            self.lab.arbor('undo', 't0')
        else:
            sh(['git', 'reset', '-q', '--hard', 'ORIG_HEAD'], cwd=self.lab.main)
        self.count('undo')


def wait_all(procs):
    for p in procs:
        rc = p.wait()
        p.stdout and p.stdout.close()
        if rc != 0:
            with open(p.log, 'rb') as f:
                raise RuntimeError(f'a try exited with {rc}: {asec.last_line(f.read())}')


def tries(arm, n, job, phases=None):
    """Make n tries, run job in each at once, keep the first, throw the others away. Returns the disk taken (bytes);
    phases, when given, gets the seconds of each step."""
    arm.typed = {}
    phases = {} if phases is None else phases
    before = fs_used(arm.lab.root)
    t = time.monotonic()

    def lap(name):
        nonlocal t
        now = time.monotonic()
        phases[name] = round(now - t, 2)
        t = now
    for i in range(n):
        arm.make(i)
    lap('make')
    wait_all([arm.start(i, job) for i in range(n)])
    lap('jobs')
    taken = fs_used(arm.lab.root) - before
    arm.keep(0)
    lap('keep')
    with open(os.path.join(arm.lab.main, 'prototype', 'arbor', 'src', 'table.rs')) as f:
        if '// try 0' not in f.read():
            raise RuntimeError('the kept try did not reach the folder')
    for i in range(1, n):
        arm.throw(i)
    lap('throw')
    return taken


# Parts ----------------------------------------------------------------------------------------------------------------

def session_once(arm):
    busy = settle()
    base = statistics.mean(mem_used() for _ in range(20) if not time.sleep(0.1))
    e0, err = rapl()
    swap0, disk0, (busy0, _), flush0 = vmstat('pswpout'), sectors_written(), cpu_busy_s(), flushes()
    s = Sampler()
    s.start()
    t0 = time.monotonic()
    try:
        phases = {}
        taken = tries(arm, 3, SESSION_JOB, phases)
        t = time.monotonic()
        subprocess.run(['sync'])
        phases['sync'] = round(time.monotonic() - t, 2)
    finally:
        xs = s.stop()
    wall = time.monotonic() - t0
    e1, _ = rapl()
    flush1 = flushes()
    energy = None
    if e0 is not None and e1 is not None:
        energy = e1 - e0 if e1 >= e0 else e1 + rapl_range() - e0
    return {'mem_peak_mib': max(xs) - base, 'mem_mean_mib': statistics.mean(xs) - base,
            'swap_mib': (vmstat('pswpout') - swap0) * os.sysconf('SC_PAGE_SIZE') / MIB,
            'disk_mib': (sectors_written() - disk0) * 512 / MIB, 'taken_mib': taken / MIB, 'wall_s': wall,
            'cpu_s': cpu_busy_s()[0] - busy0, 'energy_j': energy, 'energy_err': err,
            'flushes': flush1[0] - flush0[0], 'flush_s': round(flush1[1] - flush0[1], 2),
            'load1': float(open('/proc/loadavg').read().split()[0]), 'busy_before_pct': busy, 'phases': phases}


def session(lab, arms):
    runs = {a.name: [] for a in arms}
    errs = {}
    for rep in range(RUNS + 1):  # the first round warms the page cache and is not counted
        for arm in arms:
            if arm.name in errs:
                continue
            try:
                lab.reset()
                r = session_once(arm)
                if rep:
                    runs[arm.name].append(r)
                print(f'session {arm.name} run {rep}: {json.dumps(r)}', flush=True)
            except Exception as e:
                errs[arm.name] = e
    for arm in arms:
        keys = [f'goal.session.{arm.name}.{m}' for m in SESSION_METRICS]
        if arm.name in errs:
            fail(keys, errs[arm.name])
            continue
        rs = runs[arm.name]
        for m, key in zip(SESSION_METRICS, keys):
            xs = [r[m] for r in rs if r[m] is not None]
            if not xs:
                fail(key, rs[0]['energy_err'] if rs else 'no runs')
                continue
            emit(key, statistics.median(xs), {'blocks': [round(x, 2) for x in xs], 'runs': len(xs),
                                              'loads': [r['load1'] for r in rs], 'blocks_label': 'runs',
                                              'busy_before_pct': [r['busy_before_pct'] for r in rs],
                                              'disk_flushes': [r['flushes'] for r in rs],
                                              'disk_flush_s': [r['flush_s'] for r in rs],
                                              'phases_s': [r['phases'] for r in rs]})


def power(lab, arms):
    runs, errs = {a.name: [] for a in arms}, {}
    for rep in range(RUNS):
        for arm in arms:
            if arm.name in errs:
                continue
            try:
                lab.reset()
                settle()
                t0 = time.monotonic()
                tries(arm, 5, TRIES5_JOB)
                runs[arm.name].append(time.monotonic() - t0)
                if rep == 0:
                    arm.typed_tries5 = dict(arm.typed)
            except Exception as e:
                errs[arm.name] = e
    for arm in arms:
        key = f'goal.power.{arm.name}.tries5_s'
        if arm.name in errs:
            fail(key, errs[arm.name])
        else:
            xs = runs[arm.name]
            emit(key, statistics.median(xs), {'blocks': [round(x, 2) for x in xs], 'blocks_label': 'runs'})
            typed = arm.typed_tries5
            emit(f'goal.ease.keep.{arm.name}', typed['keep'], {'typed': TYPED[arm.name]['keep']})
            emit(f'goal.ease.discard4.{arm.name}', typed['discard'], {'typed': TYPED[arm.name]['discard']})
    for arm in arms:
        if arm.name == 'copy':
            continue  # kept and undone with git, as the worktrees are
        keys = [f'goal.power.{arm.name}.{m}' for m in ('kept', 'escaped', 'put_back', 'undo_ms')]
        try:
            lab.reset()
            undo_once(lab, arm, keys)
        except Exception as e:
            fail(keys, e)


def places(lab):
    """What a try changes: the four places in the folder and the one outside, as they are now."""
    main = lab.main

    def text(p):
        try:
            return open(p).read()
        except OSError:
            return None
    return {'tracked': text(os.path.join(main, 'README.md')), 'added': text(os.path.join(main, 'notes', 'try.txt')),
            'deleted': text(os.path.join(main, 'docs', 'TRACKING.md')), 'ignored': text(os.path.join(main, '.env')),
            'outside': text(os.path.join(lab.outside, 'note.txt'))}


def undo_once(lab, arm, keys):
    arm.make(0)
    before = places(lab)
    wait_all([arm.start(0, UNDO_JOB, key=False, args=(os.path.join(lab.outside, 'note.txt'),))])
    arm.keep(0)
    kept_now = places(lab)
    inside = ('tracked', 'added', 'deleted', 'ignored')
    kept = sum(kept_now[k] != before[k] for k in inside)
    escaped = int(kept_now['outside'] != before['outside'])
    arm.typed = {}
    t0 = time.perf_counter()
    arm.undo()
    ms = (time.perf_counter() - t0) * 1000
    emit(f'goal.ease.undo.{arm.name}', arm.typed['undo'], {'typed': TYPED[arm.name]['undo']})
    after = places(lab)
    back = sum(after[k] == before[k] for k in before)
    ctx = {'kept': [k for k in inside if kept_now[k] != before[k]], 'not_back': [k for k in before
                                                                                 if after[k] != before[k]]}
    for key, v in zip(keys, (kept, escaped, back, ms)):
        emit(key, v, ctx)


def safety(lab):
    home = os.path.expanduser('~')
    secrets = []
    for rel in SECRETS:
        p = os.path.join(home, rel)
        for q in ([os.path.join(p, n) for n in sorted(os.listdir(p))] if os.path.isdir(p) else [p]):
            if os.path.isfile(q) and os.access(q, os.R_OK) and os.path.getsize(q) > 0:
                secrets.append(q)
    transcripts = [os.path.join(home, d) for d in TRANSCRIPTS
                   if os.path.isdir(os.path.join(home, d)) and os.listdir(os.path.join(home, d))]
    bus = (os.environ.get('DBUS_SESSION_BUS_ADDRESS') or '').partition('unix:path=')[2].split(',')[0] or \
        f'/run/user/{os.getuid()}/bus'
    docker = '/var/run/docker.sock'
    servers, lan = [], None
    ip = asec.own_address()
    for host in ([ip] if ip else []) + ['127.0.0.1']:
        listener = socket.create_server((host, 0))
        threading.Thread(target=asec.pong, args=(listener, host), daemon=True).start()
        servers.append(listener)
    if ip:
        lan = [ip, servers[0].getsockname()[1]]
    sleeper = subprocess.Popen(['sleep', '900'])
    files = {'project_file': os.path.join(lab.main, '.goals-probe'),
             'outside_file': os.path.join(lab.outside, 'probe.txt'), 'marker': os.path.join(lab.outside, 'pushed')}
    args = dict(files, secrets=secrets, transcripts=transcripts, home_file=os.path.join(BENCH_DIR, 'README.md'),
                lan=lan, local=['127.0.0.1', servers[-1].getsockname()[1]],
                docker=docker if os.path.exists(docker) else None, bus=bus if os.path.exists(bus) else None,
                pid=sleeper.pid)
    py = shutil.which('python3', path='/usr/local/bin:/usr/bin:/bin')
    try:
        lab.arbor('key', 'new', 'agent', '--toolchains', '--home', '--for', '1h')
        for arm in ('today', 'arbor'):
            key = f'goal.safety.{arm}.reached'
            try:
                for p in files.values():
                    if os.path.exists(p):
                        os.remove(p)
                argv = [py, '-I', '-c', PROBE, json.dumps(args)]
                if arm == 'arbor':
                    lab.arbor('new', 'safe', '--from', lab.main)
                    argv = [lab.binary, 'run', 'safe', '--key', 'agent', '--mem', '256M', '--', *argv]
                r = run(argv, env=lab.env, timeout=300, stdin=subprocess.DEVNULL)
                got = {}
                for line in r.stdout.decode(errors='replace').splitlines():
                    if line.startswith('{'):
                        d = json.loads(line)
                        got[d['probe']] = d['reached']
                # What a probe wrote, the section looks for from outside, in the real folders.
                got['write_project'] = os.path.exists(files['project_file'])
                got['write_outside'] = os.path.exists(files['outside_file'])
                got['effect_without_yes'] = os.path.exists(files['marker'])
                if 'memory_512mib' not in got and 'keyrings' in got and r.returncode != 0:
                    got['memory_512mib'] = False  # the cell was stopped whole when it ran out of memory
                missing = [p for p in PROBES if p not in got]
                if missing:
                    raise RuntimeError(f'the probes stopped before {missing[0]} (exit {r.returncode}): '
                                       f'{asec.last_line(r.stderr)}')
                probes = {p: 'n/a' if got[p] is None else 'reached' if got[p] else 'blocked' for p in PROBES}
                emit(key, sum(v == 'reached' for v in probes.values()),
                     {'of': sum(v != 'n/a' for v in probes.values()), 'probes': probes, 'exit': r.returncode})
            except Exception as e:
                fail(key, e)
            finally:
                if arm == 'arbor':
                    for line in lab.arbor('outbox').splitlines()[1:]:
                        if line.split()[2:3] == ['safe']:
                            lab.arbor('refuse', line.split()[0])
                    lab.arbor('discard', 'safe')
    finally:
        sleeper.kill()
        sleeper.wait()
        for s in servers:
            s.shutdown(socket.SHUT_RDWR)
            s.close()
        for p in files.values():
            if os.path.exists(p):
                os.remove(p)


def ease(lab):
    start = [('new', 'ease', '--from', lab.main),
             ('key', 'new', 'agent2', '--toolchains', '--home', '--net', 'example.com', '--for', '1h'),
             ('run', 'ease', '--key', 'agent2', '--', 'true')]
    try:
        for c in start:
            lab.arbor(*c)
        emit('goal.ease.start.arbor', len(start), {'typed': ['arbor new W --from DIR', 'arbor key new KEY --toolchains '
                                                             '--net HOST --home', 'arbor shell W --key KEY']})
    except Exception as e:
        fail('goal.ease.start.arbor', e)
    try:
        lab.arbor('run', 'ease', '--', 'arbor', 'effect', '--', 'true')  # the agent asks, from inside
        typed = ['arbor outbox', 'arbor release ID']
        waiting = [line.split()[0] for line in lab.arbor('outbox').splitlines()[1:] if line.split()[2:3] == ['ease']]
        lab.arbor('release', waiting[0])
        emit('goal.ease.approve.arbor', len(typed), {'typed': typed})
    except Exception as e:
        fail('goal.ease.approve.arbor', e)
    emit('goal.ease.approve.mission', 'cannot act yet', {'why': 'Mission Control is read-only (#82)'})
    emit('goal.ease.start.today', 'no stock way', {'why': 'Ubuntu has no stock way to give one program a copy of a '
                                                          'folder, one network host and none of your logins'})
    emit('goal.ease.approve.today', 'nothing waits', {'why': 'a command an agent runs takes effect at once; only the '
                                                            "agent's own prompt, when it has one, asks first"})
    for w in ('ease',):
        try:
            lab.arbor('discard', w)
        except Exception:
            pass


class InputSeen(RuntimeError):
    pass


def still_block(seconds):
    i0, (b0, t0), d0, (e0, err) = interrupts(), cpu_busy_s(), deep_idle_us(), rapl()
    m0 = time.monotonic()
    gpu = []
    while time.monotonic() - m0 < seconds:
        time.sleep(1)
        g = gpu_w()
        if g is not None:
            gpu.append(g)
    dt = time.monotonic() - m0
    i1, (b1, t1), d1, (e1, _) = interrupts(), cpu_busy_s(), deep_idle_us(), rapl()
    idle = screen.idle_ms()
    if idle is not None and idle < dt * 1000:
        raise InputSeen(f'a key or the mouse was used {idle / 1000:.0f} s before the end of a {dt:.0f} s block')
    return {'wakeups_s': (i1 - i0) / dt, 'busy_pct': (b1 - b0) / (t1 - t0) * 100,
            'deep_pct': (d1 - d0) / (os.cpu_count() * dt * 1e6) * 100,
            'power_w': None if e0 is None or e1 is None else (e1 - e0 if e1 >= e0 else e1 + rapl_range() - e0) / dt,
            'power_err': err, 'gpu_w': round(statistics.mean(gpu), 2) if gpu else None,
            'input': 'none in the block' if idle is not None else 'not checked: no GNOME idle monitor',
            'load1': float(open('/proc/loadavg').read().split()[0])}


def still(lab, mission_bin, before):
    metrics = ('wakeups_s', 'busy_pct', 'deep_pct', 'power_w')
    if not mission_bin:
        fail([f'goal.still.{a}.{m}' for a in ('today', 'arbor') for m in metrics],
             'Mission Control could not be built (GTK 4.14+ and its development files)')
        return
    lab.arbor('new', 'still-a', '--from', lab.main)
    lab.arbor('new', 'still-b', '--from', lab.main)
    lab.arbor('run', 'still-a', '--', 'arbor', 'effect', '--', 'true')
    blocks, errs = {'today': [], 'arbor': []}, {}
    for _ in range(STILL_BLOCKS):
        for arm in ('today', 'arbor'):
            window = None
            try:
                if arm == 'today':
                    serve_stop(before)
                else:
                    lab.arbor('run', 'still-b', '--pids', '64', '--', 'true')  # starts arbor serve if it is not running
                    window = subprocess.Popen([mission_bin], env=lab.env, stdin=subprocess.DEVNULL,
                                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                time.sleep(30)  # the window's first frame, and the machine coming to rest
                r = None
                for attempt in range(3):  # a block that sees input is taken again, once nobody has touched the PC
                    try:                  # for 30 s
                        r = still_block(STILL_S)
                        break
                    except InputSeen as e:
                        print(f'still {arm}: {e}; again', flush=True)
                        for _ in range(60):
                            idle = screen.idle_ms()
                            if idle is None or idle > 30000:
                                break
                            time.sleep(10)
                if r is None:
                    raise InputSeen(f'input in 3 attempts at a {STILL_S:.0f} s block')
                r['attempts'] = attempt + 1
                r['serve'] = asec.service()
                blocks[arm].append(r)
                print(f'still {arm}: {json.dumps(r)}', flush=True)
            except Exception as e:
                errs.setdefault(arm, e)
            finally:
                if window:
                    window.terminate()
                    window.wait()
    for arm in ('today', 'arbor'):
        rs = blocks[arm]
        for m in metrics:
            key = f'goal.still.{arm}.{m}'
            xs = [r[m] for r in rs if r[m] is not None]
            if not xs:
                fail(key, errs.get(arm) or (rs[0]['power_err'] if rs else 'no blocks'))
                continue
            ctx = {'blocks': [round(x, 3) for x in xs], 'blocks_label': 'blocks', 'seconds': STILL_S,
                   'serve': [r['serve'] for r in rs], 'input': rs[0]['input'], 'loads': [r['load1'] for r in rs],
                   'attempts': [r['attempts'] for r in rs]}
            if m == 'wakeups_s':
                ctx['gpu_w'] = [r['gpu_w'] for r in rs]
            if arm in errs:
                ctx['failed_block'] = str(errs[arm])[-300:]
            emit(key, statistics.mean(xs), ctx)


def serve_stop(before):
    """Stops `arbor serve` if the section started it, unless a cell that is not the section's runs in it."""
    if before not in (None, 'active') and asec.service() == 'active':
        cells = '/sys/fs/cgroup' + asec.unit('ControlGroup') + '/cells'
        others = [d for d in os.listdir(cells) if os.path.isdir(os.path.join(cells, d))] if os.path.isdir(cells) else []
        if not others:
            run(['systemctl', '--user', 'stop', 'arbor'], timeout=60)
        else:
            print(f'arbor serve left running: {len(others)} cells of someone else run in it', flush=True)


def all_rows():
    rows = [f'goal.session.{a}.{m}' for a in ARMS for m in SESSION_METRICS]
    rows += [f'goal.power.{a}.tries5_s' for a in ARMS]
    rows += [f'goal.power.{a}.{m}' for a in ('worktree', 'arbor') for m in ('kept', 'escaped', 'put_back', 'undo_ms')]
    rows += [f'goal.safety.{a}.reached' for a in ('today', 'arbor')]
    rows += ['goal.ease.start.arbor', 'goal.ease.approve.arbor']
    rows += [f'goal.ease.{w}.{a}' for w in ('keep', 'discard4') for a in ARMS]
    rows += [f'goal.ease.undo.{a}' for a in ('worktree', 'arbor')]
    rows += [f'goal.still.{a}.{m}' for a in ('today', 'arbor') for m in ('wakeups_s', 'busy_pct', 'deep_pct', 'power_w')]
    return rows


def build():
    """(arbor, arbor-mission or None, error): Mission Control only when the still part runs."""
    if 'still' in PARTS:
        try:
            a, m = msec.build()
            return a, m, None
        except Exception as e:
            print(f'Mission Control not built: {e}', flush=True)
    os.makedirs(asec.A, exist_ok=True)
    a, err = asec.build()
    return a, None, err


def main():
    # run.sh ends a section with SIGTERM, on a timeout or Ctrl-C: leave through the finally blocks.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    base = env('GOALS_TMP') or os.path.join(BENCH_DIR, 'results')
    os.makedirs(base, exist_ok=True)
    root = tempfile.mkdtemp(prefix='.goals-', dir=base)
    print(f'parts: {" ".join(PARTS)}; runs: {RUNS}; temp folder on {run(["stat", "-f", "-c", "%T", root]).stdout.decode().strip()}',
          flush=True)
    try:
        binary, mission_bin, err = build()
        if not binary:
            return fail(all_rows(), err)
        lab = Lab(root, binary)
        before = asec.service()
        try:
            lab.setup()
            arms = [Arm(lab, n) for n in ARMS]
            if 'session' in PARTS:
                session(lab, arms)
            if 'power' in PARTS:
                power(lab, arms)
            lab.reset()
            if 'safety' in PARTS:
                safety(lab)
            if 'ease' in PARTS:
                ease(lab)
            if 'still' in PARTS:
                still(lab, mission_bin, before)
        except Exception as e:
            fail(all_rows(), f'the section stopped: {e}')  # rows already measured keep their values (report.load)
        finally:
            serve_stop(before)
    finally:
        subprocess.run(['chmod', '-R', 'u+rwX', root], stderr=subprocess.DEVNULL)  # overlay work dirs have no rights
        shutil.rmtree(root, ignore_errors=True)


if __name__ == '__main__':
    main()
