"""8. The arbor prototype, built from this checkout into the temp dir and measured there.

arbor runs with ARBOR_HOME in the temp dir, so it sees none of the person's worlds, keys, outbox or ledger, and its
worlds are made over small folders the section creates there. Its state is on the temp dir's filesystem (tmpfs on the
M machine, as in the M run), where the fsyncs that make a world, an effect and a keep durable cost little; the rows
name the filesystem. The binary is a release build of prototype/, offline, with the toolchain it pins and
CARGO_BUILD_JOBS (8 when unset) jobs, unless ARBOR_BIN names one.

Timings use the start-time method of runtimes.py: posix_spawn + wait4, 40 runs after 3 warm-ups per block, in 3 blocks
interleaved across the cases, output to /dev/null and errors to a file:
  arbor.run_ms         `arbor run W -- true` with ARBOR_NO_CGROUP=1: the sealed bwrap cell with its overlay, its
                       seccomp filter and outbox socket, and the ledger's two steps
  arbor.run_cgroup_ms  the same in a cgroup of its own, under `arbor serve`. If the service is not running, a first
                       run starts it, and the section stops it again once its rows are read, unless a cell that is
                       not the section's runs in it by then
  arbor.ask_ms         asking for an effect from inside the cell: `arbor run W -- arbor effect -- true` against
                       `arbor run W -- true`, the difference of the block medians. Each block asks in a world of its
                       own, since a world may have 64 effects waiting
  arbor.release_ms     `arbor release` of one of those effects (`true`), each run another one
Other rows: `arbor new` (20 runs after 3 warm-ups), keep and undo of a world that added 1,000 files (median of 11
cycles), the idle RSS and PSS of `arbor serve` when it runs the binary measured, the size of the binary, and the o200k
tokens of `arbor diff` when 100 files changed.
Nothing is pinned to a CPU, as in the rest of the suite. On a CPU with P- and E-cores, rows timed on one arbor process
at a time (new, keep, undo) move with the kind of core the scheduler puts it on.
"""
import os
import shutil
import signal
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import BENCH_DIR, WORK, emit, env, fail, median, run, same_program, smaps_rollup  # noqa: E402

A = os.path.join(WORK, 'arbor')
ERR = os.path.join(A, 'stderr')
NULL = os.open(os.devnull, os.O_RDWR)
ROWS = ['arbor.binary_mib', 'arbor.new_ms', 'arbor.run_ms', 'arbor.run_cgroup_ms', 'arbor.ask_ms',
        'arbor.release_ms', 'arbor.serve_rss_mib', 'arbor.serve_pss_mib', 'arbor.keep_ms', 'arbor.undo_ms',
        'arbor.diff100_tokens']
FILES = 1000
CYCLES = 11


def last_line(b):
    lines = b.decode(errors='replace').strip().splitlines()
    return lines[-1] if lines else 'nothing on stderr'


def build():
    """The arbor binary: ARBOR_BIN, or a release build of prototype/ in the temp dir. Returns (path, error)."""
    given = env('ARBOR_BIN')
    if given:
        return (given, None) if os.access(given, os.X_OK) else (None, f'ARBOR_BIN={given} is not a program')
    real = os.environ.get('CARGO_HOME') or os.path.expanduser('~/.cargo')
    cargo = shutil.which('cargo') or os.path.join(real, 'bin', 'cargo')
    if not os.access(cargo, os.X_OK):
        return None, 'cargo not found (or set ARBOR_BIN)'
    # cargo writes its lock, its record of when each crate was last used and the crates it unpacks in CARGO_HOME. A
    # home of its own in the temp dir keeps those out of ~/.cargo: it reads the registry's index, the crates already
    # downloaded and cargo's config through links, and unpacks the crates in the temp dir.
    target, home = os.path.join(A, 'target'), os.path.join(A, 'cargo')
    os.makedirs(os.path.join(home, 'registry', 'src'))
    for name in ('registry/index', 'registry/cache', 'config.toml', 'config'):
        if os.path.exists(os.path.join(real, name)):
            os.symlink(os.path.join(real, name), os.path.join(home, name))
    r = run([cargo, 'build', '--release', '--locked', '--offline', '--quiet'], timeout=600,
            cwd=os.path.join(BENCH_DIR, '..', 'prototype'),
            env=dict(os.environ, CARGO_HOME=home, CARGO_TARGET_DIR=target, RUSTUP_AUTO_INSTALL='0',
                     CARGO_BUILD_JOBS=os.environ.get('CARGO_BUILD_JOBS', '8')))
    if r.returncode != 0:
        return None, 'cargo build failed: ' + last_line(r.stderr)
    return os.path.join(target, 'release', 'arbor'), None


def spawn(argv, environ):
    """Wall time in ms of one run, from posix_spawn to wait4. Raises with the last line of its errors if it fails."""
    t0 = time.perf_counter_ns()
    pid = os.posix_spawn(argv[0], argv, environ, file_actions=[
        (os.POSIX_SPAWN_DUP2, NULL, 0), (os.POSIX_SPAWN_DUP2, NULL, 1),
        (os.POSIX_SPAWN_OPEN, 2, ERR, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)])
    _, status, _ = os.wait4(pid, 0)
    ms = (time.perf_counter_ns() - t0) / 1e6
    if status != 0:
        with open(ERR, 'rb') as f:
            raise RuntimeError(f'`arbor {" ".join(argv[1:])}` exited with {os.waitstatus_to_exitcode(status)}: '
                               f'{last_line(f.read())}')
    return ms


class Arbor:
    def __init__(self, binary):
        self.binary = binary
        self.env = dict(os.environ, ARBOR_HOME=os.path.join(A, 'state'))
        self.sealed = dict(self.env, ARBOR_NO_CGROUP='1')

    def __call__(self, *args, environ=None):
        """Runs arbor and returns what it printed; raises with its error if it fails."""
        r = run([self.binary, *args], env=environ or self.sealed, stdin=subprocess.DEVNULL)
        if r.returncode != 0:
            raise RuntimeError(f'`arbor {" ".join(args)}` exited with {r.returncode}: {last_line(r.stderr)}')
        return r.stdout.decode(errors='replace')

    def argv(self, *args):
        return [self.binary, *args]


def folder(name, files):
    root = os.path.join(A, name)
    for rel, text in files.items():
        os.makedirs(os.path.dirname(os.path.join(root, rel)), exist_ok=True)
        with open(os.path.join(root, rel), 'w') as f:
            f.write(text)
    return root


def blocks(cases, n=40, warm=3, count=3):
    """Block medians per case, the blocks interleaved across cases. A case is (key, argv of block b, environment)."""
    meds, errs = {key: [] for key, _, _ in cases}, {}
    for b in range(count):
        for key, argv, environ in cases:
            if key in errs:
                continue
            try:
                meds[key].append(statistics.median([spawn(argv(b), environ) for _ in range(n + warm)][warm:]))
            except Exception as e:
                errs[key] = e
    return meds, errs


def service():
    """How systemd says `arbor serve` is (`active`, `inactive`, ...), or None where there is no user manager."""
    if not shutil.which('systemctl'):
        return None
    r = run(['systemctl', '--user', 'is-active', 'arbor'], timeout=30)
    return r.stdout.decode().strip() or None


def unit(prop):
    return run(['systemctl', '--user', 'show', '-p', prop, '--value', 'arbor'], timeout=30).stdout.decode().strip()


def main():
    # run.sh ends a section with SIGTERM, on a timeout or Ctrl-C: leave through the finally blocks, so that a service
    # the section started is stopped again.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    os.makedirs(A)
    binary, err = build()
    if not binary:
        return fail(ROWS, err)
    built = 'ARBOR_BIN' if env('ARBOR_BIN') else 'release build of prototype/'
    emit('arbor.binary_mib', os.path.getsize(binary) / 2**20, {'binary': built})
    arbor = Arbor(binary)
    fs = run(['stat', '-f', '-c', '%T', A]).stdout.decode().strip()
    tree = folder('project', {'README': 'one\n', 'src/a': 'two\n'})

    try:
        ts = [spawn(arbor.argv('new', f'n{i}', '--from', tree), arbor.sealed) for i in range(23)][3:]
        for i in range(23):
            arbor('discard', f'n{i}')
        for w in ('cell', 'ask0', 'ask1', 'ask2'):
            arbor('new', w, '--from', tree)
    except Exception as e:
        # Without worlds nothing else can run: the reason is the same for every row.
        return fail(ROWS[1:], e)
    emit('arbor.new_ms', median(ts), {'runs': 20, 'state_fs': fs})

    cases = [('arbor.run_ms', lambda b: arbor.argv('run', 'cell', '--', 'true'), arbor.sealed),
             ('ask', lambda b: arbor.argv('run', f'ask{b}', '--', 'arbor', 'effect', '--', 'true'), arbor.sealed)]
    # A cell in a cgroup of its own needs `arbor serve`. A limit is refused without a cgroup, so a run with one shows
    # that the cell got one; that run starts the service if it was not running.
    before = cgroup_err = None
    try:
        try:
            before = service()
            if before is None:
                raise RuntimeError('cells cannot have a cgroup here: there is no systemd user manager to reach')
            arbor('run', 'cell', '--pids', '4096', '--', 'true', environ=arbor.env)
            cases.append(('arbor.run_cgroup_ms', lambda b: arbor.argv('run', 'cell', '--', 'true'), arbor.env))
        except Exception as e:
            cgroup_err = e
        started = before not in (None, 'active') and service() == 'active'
        errs = timed(arbor, binary, cases, fs, started, cgroup_err)
    finally:
        if before not in (None, 'active') and service() == 'active':
            # The section started it: stopped again, unless a cell that is not the section's runs in it by now.
            cells = '/sys/fs/cgroup' + unit('ControlGroup') + '/cells'
            others = [d for d in os.listdir(cells) if os.path.isdir(os.path.join(cells, d))] \
                if os.path.isdir(cells) else []
            if not others:
                run(['systemctl', '--user', 'stop', 'arbor'], timeout=60)
            else:
                print(f'arbor serve left running: {len(others)} cells of someone else run in it', flush=True)

    try:
        waiting = [line.split()[0] for line in arbor('outbox').splitlines()[1:] if line.split()[2:3] == ['ask0']]
        if len(waiting) < 43:
            # Without the asks there is nothing to release: their error is the reason.
            raise errs.get('ask') or RuntimeError(f'world ask0 has {len(waiting)} effects waiting, not 43')
        ts = [spawn(arbor.argv('release', i), arbor.sealed) for i in waiting[:43]][3:]
        emit('arbor.release_ms', median(ts), {'runs': len(ts), 'state_fs': fs})
    except Exception as e:
        fail('arbor.release_ms', e)

    try:
        keep_and_undo(arbor, fs)
    except Exception as e:
        fail(['arbor.keep_ms', 'arbor.undo_ms'], e)
    try:
        diff_tokens(arbor)
    except Exception as e:
        fail('arbor.diff100_tokens', e)


def timed(arbor, binary, cases, fs, started, cgroup_err):
    """The cell rows, asking, and the service's memory. Returns the errors of the cases that failed."""
    meds, errs = blocks(cases)
    # A cell with no limit runs without a cgroup when it cannot have one, so the service must still run at the end.
    if meds.get('arbor.run_cgroup_ms') and 'arbor.run_cgroup_ms' not in errs and service() != 'active':
        errs['arbor.run_cgroup_ms'] = RuntimeError('arbor serve was not running when the section finished timing '
                                                   'cells, so some may have run without a cgroup')
    ctx = {'state_fs': fs}
    for key in ('arbor.run_ms', 'arbor.run_cgroup_ms'):
        if key in errs:
            fail(key, errs[key])
        elif meds.get(key):
            emit(key, statistics.median(meds[key]), dict(ctx, blocks=[round(x, 4) for x in meds[key]]))
        else:
            fail(key, cgroup_err)
    if 'ask' in errs or 'arbor.run_ms' in errs:
        fail('arbor.ask_ms', errs.get('ask') or errs['arbor.run_ms'])
    else:
        share = [a - r for a, r in zip(meds['ask'], meds['arbor.run_ms'])]
        emit('arbor.ask_ms', statistics.median(share),
             dict(ctx, blocks=[round(x, 4) for x in share], run_ms=round(statistics.median(meds['arbor.run_ms']), 4),
                  with_ask_ms=round(statistics.median(meds['ask']), 4)))

    try:
        if cgroup_err is not None:
            raise cgroup_err
        pid = int(unit('MainPID'))
        # The service only holds the cells' cgroups, so a cell starts the same under any arbor serve; its memory is
        # that of the binary it runs.
        if not os.path.samestat(os.stat(f'/proc/{pid}/exe'), os.stat(binary)):
            raise RuntimeError('arbor serve was already running another arbor binary; with no cell in it, '
                               '`systemctl --user stop arbor` lets the section start this one')
        mem = smaps_rollup(pid)
        how = 'started by the section' if started else 'already running'
        rss, anon = mem['Rss'] / 1024, mem['Anonymous'] / 1024
        emit('arbor.serve_rss_mib', rss, {'service': how, 'anon_mib': round(anon, 2), 'file_mib': round(rss - anon, 2)})
        emit('arbor.serve_pss_mib', mem['Pss'] / 1024, {'service': how, 'same_program_procs': same_program(pid)})
    except Exception as e:
        fail(['arbor.serve_rss_mib', 'arbor.serve_pss_mib'], e)
    return errs


def keep_and_undo(arbor, fs):
    """Keep, then undo, a world that added FILES files, CYCLES times over: undo puts them back in the world."""
    tree = folder('keep', {'README': 'one\n'})
    arbor('new', 'keep', '--from', tree)
    arbor('run', 'keep', '--', 'sh', '-c',
          f'mkdir add && i=0 && while [ $i -lt {FILES} ]; do echo $i > add/f$i; i=$((i+1)); done')
    keeps, undos = [], []
    for _ in range(CYCLES):
        keeps.append(spawn(arbor.argv('keep', 'keep'), arbor.sealed))
        if len(os.listdir(os.path.join(tree, 'add'))) != FILES:
            raise RuntimeError(f'keep did not bring {FILES} files into the folder')
        undos.append(spawn(arbor.argv('undo', 'keep'), arbor.sealed))
        if os.path.exists(os.path.join(tree, 'add')):
            raise RuntimeError('undo did not take the files back out of the folder')
    emit('arbor.keep_ms', median(keeps), {'files': FILES, 'runs_ms': [round(t, 1) for t in keeps], 'state_fs': fs})
    emit('arbor.undo_ms', median(undos), {'files': FILES, 'runs_ms': [round(t, 1) for t in undos], 'state_fs': fs})


def diff_tokens(arbor):
    """o200k tokens of `arbor diff` after a command in a world changed 100 files."""
    node = shutil.which('node')
    if not node:
        raise RuntimeError('node not found')
    tree = folder('tokens', {f'src/file_{i:03}.txt': 'one\n' for i in range(100)})
    arbor('new', 'tokens', '--from', tree)
    arbor('run', 'tokens', '--', 'sh', '-c', 'for f in src/file_*.txt; do echo two > "$f"; done')
    listing = arbor('diff', 'tokens')
    if len(listing.splitlines()) != 100:
        raise RuntimeError(f'`arbor diff` listed {len(listing.splitlines())} changes, not 100')
    path = os.path.join(A, 'diff.txt')
    with open(path, 'w') as f:
        f.write(listing)
    r = run([node, os.path.join(BENCH_DIR, 'lib', 'o200k.js'), path], timeout=120)
    if r.returncode != 0:
        raise RuntimeError('o200k counter failed: ' + last_line(r.stderr))
    emit('arbor.diff100_tokens', int(r.stdout.decode().strip()), {'bytes': len(listing.encode())})


if __name__ == '__main__':
    main()
