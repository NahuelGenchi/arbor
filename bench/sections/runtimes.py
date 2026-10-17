"""1. Runtime floors and start times: static C, Rust, bash, python3, node (plus /bin/true, busybox and python3 -I -S).

Start: posix_spawn + wait4 wall time, median of 40 runs after 3 warm-ups. Three blocks run interleaved across the
cases; the reported value is the median of the block medians and the block spread goes into the note.
Idle floor: Rss and Pss from /proc/<pid>/smaps_rollup 1.5 s after start. ru_maxrss is not used: under posix_spawn
it reports the parent's size.
"""
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import (BENCH_DIR, WORK, emit, emit_blocks, fail, rss_split, run, same_program,  # noqa: E402
                    smaps_rollup, spawn_blocks)

BIN = os.path.join(WORK, 'bin')
os.makedirs(BIN, exist_ok=True)


def which(name, *extra):
    for c in (shutil.which(name), *extra):
        if c and os.access(c, os.X_OK):
            return c
    return None


def build():
    """Compile the two native hellos into the temp dir (same flags as the M run)."""
    out = {}
    cc = which('gcc') or which('cc')
    if cc:
        r = run([cc, '-O2', '-static', '-o', f'{BIN}/hello_static', f'{BENCH_DIR}/src/hello.c'])
        out['c'] = f'{BIN}/hello_static' if r.returncode == 0 else r.stderr.decode(errors='replace')
    else:
        out['c'] = 'gcc not found'
    rustc = which('rustc', os.path.expanduser('~/.cargo/bin/rustc'))
    if rustc:
        r = run([rustc, '-C', 'opt-level=3', '-C', 'panic=abort', '-o', f'{BIN}/hello_rust', f'{BENCH_DIR}/src/hello.rs'],
                timeout=300, cwd=BIN)
        out['rust'] = f'{BIN}/hello_rust' if r.returncode == 0 else r.stderr.decode(errors='replace')
    else:
        out['rust'] = 'rustc not found'
    return out


def main():
    b = build()
    c, rust = (b[k] if b[k].startswith(BIN) else None for k in ('c', 'rust'))
    true_bin = which('true', '/bin/true')
    busybox, bash, py, node = which('busybox'), which('bash'), which('python3'), which('node')

    starts = [('rt.start.c_static', [c] if c else None),
              ('rt.start.true', [true_bin] if true_bin else None),
              ('rt.start.rust', [rust] if rust else None),
              ('rt.start.busybox', [busybox, 'true'] if busybox else None),
              ('rt.start.bash', [bash, '-c', 'true'] if bash else None),
              ('rt.start.python_IS', [py, '-I', '-S', '-c', 'pass'] if py else None),
              ('rt.start.python', [py, '-c', 'pass'] if py else None),
              ('rt.start.node', [node, '-e', '0'] if node else None)]
    for key, argv in starts:
        if argv is None:
            fail(key, b['c'] if 'c_static' in key else b['rust'] if 'rust' in key else 'binary not found')
    meds, errs = spawn_blocks([(k, a) for k, a in starts if a])
    emit_blocks(meds, errs)

    # Idle floors: one process at a time (concurrent copies would share pages and lower each other's PSS).
    idle = [('c_static', [c, '30'] if c else None),
            ('rust', [rust, '30'] if rust else None),
            ('busybox_sh', [busybox, 'sh', '-c', 'sleep 30'] if busybox else None),
            ('bash', [bash, '-c', 'sleep 30; :'] if bash else None),
            ('python', [py, '-c', 'import time; time.sleep(30)'] if py else None),
            ('python_net', [py, '-c', 'import json,ssl,asyncio,http.client,urllib.request,subprocess,time; time.sleep(30)'] if py else None),
            ('node', [node, '-e', 'setTimeout(()=>{},30000)'] if node else None)]
    for name, argv in idle:
        keys = [f'rt.rss.{name}', f'rt.pss.{name}']
        if argv is None:
            fail(keys, 'binary not found')
            continue
        # Most of an idle RSS is file pages, and a fault also maps the cached neighbours of the page it needs: the
        # reading follows how much of the binary is in the page cache. Reading it first makes that the same each run.
        with open(os.path.realpath(argv[0]), 'rb') as f:
            while f.read(1 << 20):
                pass
        p = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            time.sleep(1.5)
            r = smaps_rollup(p.pid)
            anon, file = rss_split(p.pid)
            priv = (r.get('Private_Clean', 0) + r.get('Private_Dirty', 0)) / 1024
            emit(keys[0], r['Rss'] / 1024, {'private_mib': round(priv, 2), 'anon_mib': anon, 'file_mib': file})
            emit(keys[1], r['Pss'] / 1024, {'same_program_procs': same_program(p.pid)})
        except Exception as e:
            fail(keys, e)
        finally:
            p.kill()
            p.wait()


if __name__ == '__main__':
    main()
