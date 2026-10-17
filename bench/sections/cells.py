"""2. Cell start times: unshare (user ns; user+net+pid+mnt), bwrap --unshare-all, docker run (median of 8).

The namespace cells use the start-time method of runtimes.py. docker runs an image that is already present
(--pull never, --network none, --rm), so nothing is downloaded and no container is left behind.
"""
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import emit, emit_blocks, env, fail, median, run, spawn_blocks  # noqa: E402


def main():
    true_bin = shutil.which('true') or '/bin/true'
    unshare, bwrap = shutil.which('unshare'), shutil.which('bwrap')
    cases = []
    if unshare:
        cases += [('cell.unshare_user', [unshare, '-Ur', true_bin]),
                  ('cell.unshare_4ns', [unshare, '-Urnpfm', '--mount-proc', true_bin])]
    else:
        fail(['cell.unshare_user', 'cell.unshare_4ns'], 'unshare not found')
    if bwrap:
        cases.append(('cell.bwrap', [bwrap, '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                                     '--unshare-all', '--die-with-parent', true_bin]))
    else:
        fail('cell.bwrap', 'bwrap not found')
    meds, errs = spawn_blocks(cases)
    emit_blocks(meds, errs)
    docker()


def docker():
    key, img = 'cell.docker', env('DOCKER_IMAGE', 'ubuntu:24.04')
    dk = shutil.which('docker')
    if not dk:
        return fail(key, 'docker not installed')
    try:
        r = run([dk, 'image', 'inspect', '--format', '{{.Id}}', img], timeout=30)
    except subprocess.TimeoutExpired:
        return fail(key, 'docker daemon did not answer within 30 s')
    if r.returncode != 0:
        return fail(key, f'image {img} not available locally (it is never pulled) or daemon not reachable: '
                         + r.stderr.decode(errors='replace'))
    ts = []
    for _ in range(8):
        t0 = time.perf_counter_ns()
        r = subprocess.run([dk, 'run', '--rm', '--pull', 'never', '--network', 'none', img, '/bin/true'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=120)
        t1 = time.perf_counter_ns()
        if r.returncode != 0:
            return fail(key, r.stderr.decode(errors='replace'))
        ts.append((t1 - t0) / 1e6)
    emit(key, median(ts), {'runs': [round(t) for t in ts], 'image': img})


if __name__ == '__main__':
    main()
