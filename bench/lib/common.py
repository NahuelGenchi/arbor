"""Helpers shared by the bench sections: result records, process timing, memory readings, namespace access and
the repository the git rows read.

Every section appends one JSON object per measurement to $BENCH_RESULTS:
  {"key": ..., "value": number|string|null, "ctx": {...}, "err": "..."}
lib/report.py joins these records with the M values in lib/reference.py.
"""
import json
import os
import re
import resource
import shutil
import statistics
import subprocess
import time

RESULTS = os.environ.get('BENCH_RESULTS')
WORK = os.environ.get('BENCH_WORK') or '/tmp'
BENCH_DIR = os.environ.get('BENCH_DIR') or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEVNULL = subprocess.DEVNULL


def emit(key, value, ctx=None, err=None):
    rec = {'key': key, 'value': value}
    if ctx:
        rec['ctx'] = ctx
    if err:
        rec['err'] = str(err).strip()[-400:]
    line = json.dumps(rec, default=str)
    if RESULTS:
        with open(RESULTS, 'a') as f:
            f.write(line + '\n')
    print(line, flush=True)


def fail(keys, err):
    for k in ([keys] if isinstance(keys, str) else keys):
        emit(k, None, err=err)


def median(xs):
    return statistics.median(xs) if xs else None


def pct(xs, p):
    """Nearest-rank percentile on a sorted copy (the M scripts used this form)."""
    s = sorted(xs)
    return s[min(len(s) - 1, int(len(s) * p))] if s else None


def env(name, default=None):
    v = os.environ.get(name)
    return v if v not in (None, '') else default


def spawn_ms(argv, n=40, warm=3):
    """posix_spawn + wait4 wall time in ms for n runs after `warm` warm-up runs (the M method)."""
    devnull = os.open(os.devnull, os.O_RDWR)
    try:
        ts = []
        for i in range(n + warm):
            t0 = time.perf_counter_ns()
            pid = os.posix_spawn(argv[0], argv, os.environ,
                                 file_actions=[(os.POSIX_SPAWN_DUP2, devnull, 1), (os.POSIX_SPAWN_DUP2, devnull, 2)])
            _, status, _ = os.wait4(pid, 0)
            t1 = time.perf_counter_ns()
            if status != 0:
                raise RuntimeError(f'{" ".join(argv)} exited with status {status >> 8}')
            if i >= warm:
                ts.append((t1 - t0) / 1e6)
        return ts
    finally:
        os.close(devnull)


def spawn_blocks(cases, blocks=3, n=40):
    """Run every case n times per block, blocks interleaved across cases to spread drift.
    Returns {key: [block medians]} and {key: error} for cases that failed."""
    meds, errs = {k: [] for k, _ in cases}, {}
    for _ in range(blocks):
        for key, argv in cases:
            if key in errs:
                continue
            try:
                meds[key].append(statistics.median(spawn_ms(argv, n)))
            except RuntimeError as e:  # it ran and failed: the timed runs discard stderr, so run it once more for it
                errs[key] = f'{e}: {stderr_tail(argv)}'
            except Exception as e:  # missing binary, ...
                errs[key] = e
    return meds, errs


def stderr_tail(argv):
    """The last line a command prints to stderr."""
    try:
        r = subprocess.run(argv, stdout=DEVNULL, stderr=subprocess.PIPE, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as e:
        return str(e)
    lines = r.stderr.decode(errors='replace').strip().splitlines()
    return lines[-1] if lines else 'nothing on stderr'


def emit_blocks(meds, errs, extra=None):
    for key, bm in meds.items():
        if key in errs:
            emit(key, None, err=errs[key])
        elif bm:
            ctx = {'blocks': [round(x, 4) for x in bm]}
            ctx.update((extra or {}).get(key, {}))
            emit(key, statistics.median(bm), ctx)


def smaps_rollup(pid):
    """Rss/Pss/Private_* in kB from /proc/<pid>/smaps_rollup."""
    d = {}
    with open(f'/proc/{pid}/smaps_rollup') as f:
        for line in f:
            m = re.match(r'^(\w+):\s+(\d+) kB', line)
            if m:
                d[m.group(1)] = int(m.group(2))
    return d


def rss_split(pid):
    """(anon MiB, file MiB) of the resident set, from /proc/<pid>/status."""
    d = {}
    with open(f'/proc/{pid}/status') as f:
        for line in f:
            if line.startswith(('RssAnon:', 'RssFile:')):
                d[line.split(':')[0]] = int(line.split()[1]) / 1024
    return round(d.get('RssAnon', 0), 2), round(d.get('RssFile', 0), 2)


def same_program(pid):
    """How many other processes run the same executable (they share its pages, which lowers PSS).
    Compares /proc/<pid>/exe where it is readable (own processes), else the process name."""
    try:
        exe = os.readlink(f'/proc/{pid}/exe')
        name = open(f'/proc/{pid}/comm').read().strip()
    except OSError:
        return None
    n = 0
    for q in os.listdir('/proc'):
        if not q.isdigit() or int(q) == pid:
            continue
        try:
            n += os.readlink(f'/proc/{q}/exe') == exe
            continue
        except OSError:
            pass
        try:
            n += open(f'/proc/{q}/comm').read().strip() == name
        except OSError:
            pass
    return n


def run(argv, timeout=120, **kw):
    return subprocess.run(argv, capture_output=True, timeout=timeout, **kw)


def sysctl(name):
    """A kernel setting from /proc/sys ('user.max_user_namespaces'), or None where the kernel has no such setting."""
    try:
        with open('/proc/sys/' + name.replace('.', '/')) as f:
            return f.read().strip()
    except OSError:
        return None


BLOCKED_TOO = '; this process may not use user namespaces (see the unshare rows)'


def userns_blocked():
    """Why this process cannot do what `unshare -Ur` does (create a user namespace and map its own uid in it), or None
    when it can, or when unshare is missing and it cannot tell. Ubuntu 24.04 and later set
    kernel.apparmor_restrict_unprivileged_userns = 1: a process whose AppArmor label does not allow user namespaces (a
    shell in an ordinary terminal runs unconfined) can still create one, but AppArmor confines it there to the
    unprivileged_userns profile, which denies every capability, so writing its uid_map fails. bwrap is exempt where
    AppArmor has a profile for it (bwrap-userns-restrict)."""
    unshare = shutil.which('unshare')
    if not unshare:
        return None
    try:
        r = run([unshare, '-Ur', 'true'], timeout=30)
    except (OSError, subprocess.TimeoutExpired) as e:
        return f'`unshare -Ur` did not run: {e}'
    if r.returncode == 0:
        return None
    err = (r.stderr.decode(errors='replace').strip().splitlines() or [f'exit status {r.returncode}'])[-1]
    if sysctl('user.max_user_namespaces') == '0':
        return f'user namespaces are turned off (user.max_user_namespaces = 0): {err}'
    if sysctl('kernel.unprivileged_userns_clone') == '0':
        return f'unprivileged user namespaces are turned off (kernel.unprivileged_userns_clone = 0): {err}'
    if sysctl('kernel.apparmor_restrict_unprivileged_userns') == '1':
        label = '/proc/self/attr/apparmor/current'
        try:  # the profile of this process, and the one a user namespace it creates is put under
            outer = open(label).read().strip()
            inner = run([unshare, '-U', 'cat', label], timeout=30).stdout.decode(errors='replace')
        except (OSError, subprocess.TimeoutExpired):
            outer = inner = ''
        if inner.startswith('unprivileged_userns'):
            return ('blocked by AppArmor: kernel.apparmor_restrict_unprivileged_userns = 1 (Ubuntu 24.04 and later), '
                    f'and this process runs as `{outer}`, which does not allow user namespaces, so the one `unshare` '
                    'makes is confined to the `unprivileged_userns` profile, which denies the capabilities it needs '
                    f'({err})')
    return f'`unshare -Ur` failed: {err}'


def child_disk_mib():
    """MiB that finished child processes have read from storage (page-cache hits not included)."""
    return resource.getrusage(resource.RUSAGE_CHILDREN).ru_inblock * 512 / 2**20


def timed(fn, n):
    """Median wall time in ms of n calls, and the last result."""
    ts, r = [], None
    for _ in range(n):
        t0 = time.perf_counter()
        r = fn()
        ts.append((time.perf_counter() - t0) * 1000)
    return statistics.median(ts), r, ts


def git_target():
    """(repo, ref): $GIT_LOG_REPO if set, else the repo under $PROJECTS_DIR that holds $GIT_LOG_REF,
    else the most recently committed repo there at HEAD."""
    ref, repo = env('GIT_LOG_REF', 'HEAD'), env('GIT_LOG_REPO')
    if repo:
        return repo, ref
    root = env('PROJECTS_DIR')
    if not root or not os.path.isdir(root):
        return None, ref
    repos = [os.path.join(root, d) for d in sorted(os.listdir(root)) if os.path.isdir(os.path.join(root, d, '.git'))]
    for r in repos:
        if subprocess.run(['git', '-C', r, 'cat-file', '-e', f'{ref}^{{commit}}'], capture_output=True).returncode == 0:
            return r, ref
    latest = []
    for r in repos:
        t = subprocess.run(['git', '-C', r, 'log', '-1', '--format=%ct'], capture_output=True, text=True).stdout.strip()
        if t:
            latest.append((int(t), r))
    return (max(latest)[1], 'HEAD') if latest else (None, ref)
