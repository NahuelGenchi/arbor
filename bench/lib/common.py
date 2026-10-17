"""Helpers shared by the bench sections: result records, process timing and memory readings.

Every section appends one JSON object per measurement to $BENCH_RESULTS:
  {"key": ..., "value": number|string|null, "ctx": {...}, "err": "..."}
lib/report.py joins these records with the M values in lib/reference.py.
"""
import json
import os
import re
import resource
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
            except Exception as e:  # missing binary, namespace denied, ...
                errs[key] = e
    return meds, errs


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
