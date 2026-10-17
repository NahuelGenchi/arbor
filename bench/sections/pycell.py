"""2b. Python cell with 13 stdlib imports: a fresh interpreter vs a fork of a warm template (zygote).

Fresh: `python3 -c "import <13 modules>"`, median of 20 runs after 3 warm-ups.
Fork: os.fork() of this process after it imported the same modules; the child exits at once; median of 50 after 3.
PSS: median over 10 live forked children vs 10 live fresh interpreters, from /proc/<pid>/smaps_rollup.
Runs in its own interpreter so the template holds only these imports.
"""
import os
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import emit, fail, same_program, smaps_rollup  # noqa: E402

IMPORTS = ("import json,ssl,asyncio,http.client,urllib.request,subprocess,email,decimal,sqlite3,"
           "xml.etree.ElementTree,zipfile,csv,hashlib")


def pss(pid):
    return smaps_rollup(pid)['Pss'] / 1024


def main():
    ts = []
    for _ in range(23):
        t0 = time.perf_counter_ns()
        subprocess.run([sys.executable, '-c', IMPORTS], check=True)
        ts.append((time.perf_counter_ns() - t0) / 1e6)
    fresh = statistics.median(ts[3:])

    exec(IMPORTS)  # this process becomes the warm template
    ts = []
    for _ in range(53):
        t0 = time.perf_counter_ns()
        pid = os.fork()
        if pid == 0:
            os._exit(0)
        os.waitpid(pid, 0)
        ts.append((time.perf_counter_ns() - t0) / 1e6)
    fork = statistics.median(ts[3:])

    kids = []
    try:
        for _ in range(10):
            r, w = os.pipe()
            pid = os.fork()
            if pid == 0:
                os.close(w)
                os.read(r, 1)
                os._exit(0)
            os.close(r)
            kids.append((pid, w))
        time.sleep(0.5)
        child_pss = statistics.median(pss(p) for p, _ in kids)
        template_pss = pss(os.getpid())
    finally:
        for p, w in kids:
            os.write(w, b'x')
            os.waitpid(p, 0)

    procs = [subprocess.Popen([sys.executable, '-c', IMPORTS + ';import time;time.sleep(5)']) for _ in range(10)]
    try:
        time.sleep(2.5)
        fresh_pss = statistics.median(pss(p.pid) for p in procs)
    finally:
        for p in procs:
            p.kill()
            p.wait()

    emit('cell.py_fresh_ms', fresh)
    emit('cell.py_fork_ms', fork, {'speedup': round(fresh / fork)})
    emit('cell.py_fresh_pss', fresh_pss)
    emit('cell.py_fork_pss', child_pss)
    emit('cell.py_template_pss', template_pss, {'same_program_procs': same_program(os.getpid())})


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        fail(['cell.py_fresh_ms', 'cell.py_fork_ms', 'cell.py_fresh_pss', 'cell.py_fork_pss', 'cell.py_template_pss'], e)
