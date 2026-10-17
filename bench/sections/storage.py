"""6. Storage: forking a large tree with an overlay copy-on-write mount, and duplicate bytes by unique inode.

Fork: inside `unshare -Urm` (a private user + mount namespace, so nothing is mounted outside it) an overlay is
mounted over $FORK_TREE with upper/work dirs in the temp dir. Timed like the M run: `mount -t overlay` bracketed by
`date +%s%N` in bash (so it includes starting mount(8)); the mount(2) syscall alone is timed too. A write into the
fork must not reach the lower tree. This needs this process to be allowed user namespaces; where AppArmor blocks them
(Ubuntu 24.04 and later, from an ordinary terminal) both rows fail with that reason (common.userns_blocked).
The same fork through bwrap, which runs there where AppArmor has a profile for it, is a different method with its own
rows: `true` started in a sandbox whose view of the tree is an overlay with a tmpfs upper (`--tmp-overlay`), mounted
over the tree's own path, and the overlay's share of that start against the same sandbox without it.
Baseline: `git worktree add` from a `git clone --shared` of the same repo. Default tree: the largest directory
directly under $PROJECTS_DIR.

Dedup: every regular file under $DEDUP_ROOT, counted once per inode; files that share a size are hashed (BLAKE2b,
whole file; reads of files >= 256 KiB use O_DIRECT so the scan does not flush the page cache) and every copy after
the first (shortest path) of identical content counts as duplicate bytes, attributed to the path class it sits in.
"""
import collections
import hashlib
import mmap
import os
import shutil
import statistics
import subprocess
import sys
import time
from multiprocessing import Pool

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import (BLOCKED_TOO, WORK, child_disk_mib, emit, env, fail, run, spawn_blocks,  # noqa: E402
                    userns_blocked)

DIRECT_MIN = 256 * 1024
CLASSES = ('/node_modules/', '/.claude/worktrees/', '/.git/', '/target/', '/build/', '/dist/', '/.venv/', '/venv/',
           '/site-packages/')
_buf = None


def tree_sizes(root):
    """Apparent size of each directory directly under root (via du)."""
    out = subprocess.run(['du', '-s', '--apparent-size', '--block-size=1'] +
                         [os.path.join(root, d) for d in sorted(os.listdir(root))
                          if os.path.isdir(os.path.join(root, d)) and not os.path.islink(os.path.join(root, d))],
                         capture_output=True, text=True).stdout
    return {p: int(s) for s, p in (line.split('\t', 1) for line in out.splitlines() if '\t' in line)}


FORK_SH = r'''set -e
L="$1"; W="$2"; N="$3"
for i in $(seq 1 "$N"); do
  mkdir -p "$W/up$i" "$W/work$i" "$W/merged$i"
  s=$(date +%s%N)
  mount -t overlay overlay -o "lowerdir=$L,upperdir=$W/up$i,workdir=$W/work$i" "$W/merged$i"
  e=$(date +%s%N)
  echo "cmd $(( (e-s)/1000 ))"
done
echo "entries $(find "$W/merged1" -maxdepth 2 | wc -l)"
echo hello > "$W/merged1/__arbor_fork_test.txt"
if [ -e "$L/__arbor_fork_test.txt" ]; then echo "leak yes"; else echo "leak no"; fi
"$4" -I - "$L" "$W" "$N" <<'EOF'
import ctypes, os, sys, time
libc = ctypes.CDLL(None, use_errno=True)
L, W, N = sys.argv[1], sys.argv[2], int(sys.argv[3])
for i in range(N):
    for d in ('rup', 'rwork', 'rmerged'):
        os.makedirs(f'{W}/{d}{i}', exist_ok=True)
    opts = f'lowerdir={L},upperdir={W}/rup{i},workdir={W}/rwork{i}'.encode()
    t0 = time.perf_counter_ns()
    r = libc.mount(b'overlay', f'{W}/rmerged{i}'.encode(), b'overlay', 0, opts)
    t1 = time.perf_counter_ns()
    if r != 0:
        raise SystemExit('mount(2) failed: ' + os.strerror(ctypes.get_errno()))
    print('sys', (t1 - t0) // 1000)
EOF
'''


def fork(tree, blocked):
    keys = ['storage.fork_ms', 'storage.fork_syscall_ms']
    if blocked or not shutil.which('unshare'):
        return fail(keys, blocked or 'unshare not found')
    W = os.path.join(WORK, 'ov')
    os.makedirs(W, exist_ok=True)
    sh = os.path.join(W, 'fork.sh')
    open(sh, 'w').write(FORK_SH)
    r = run(['unshare', '-Urm', 'bash', sh, tree, W, '7', sys.executable], timeout=120)
    out = r.stdout.decode(errors='replace')
    cmd = [int(line.split()[1]) / 1000 for line in out.splitlines() if line.startswith('cmd ')]
    sysc = [int(line.split()[1]) / 1000 for line in out.splitlines() if line.startswith('sys ')]
    leak = next((line.split()[1] for line in out.splitlines() if line.startswith('leak ')), '?')
    entries = next((int(line.split()[1]) for line in out.splitlines() if line.startswith('entries ')), None)
    subprocess.run(['chmod', '-R', 'u+rwX', W], capture_output=True)
    shutil.rmtree(W, ignore_errors=True)
    if r.returncode != 0 or not cmd:
        return fail(keys, f'overlay in a user namespace failed: {r.stderr.decode(errors="replace")}')
    ctx = {'tree': os.path.basename(tree), 'runs_ms': [round(x, 2) for x in cmd], 'visible_entries_depth2': entries,
           'write_reached_lower': leak}
    emit('storage.fork_ms', statistics.median(cmd), ctx)
    if sysc:
        emit('storage.fork_syscall_ms', statistics.median(sysc), {'runs_ms': [round(x, 3) for x in sysc]})
    else:
        fail('storage.fork_syscall_ms', 'mount(2) timing did not run: ' + r.stderr.decode(errors='replace'))


def fork_bwrap(tree, blocked):
    """The fork through bwrap, timed as the cells (posix_spawn + wait4, 40 runs after 3 warm-ups per block) in 5
    interleaved blocks, since the overlay's share is the difference of two close medians."""
    keys = ['storage.fork_bwrap_ms', 'storage.fork_bwrap_overlay_ms']
    bwrap, true_bin = shutil.which('bwrap'), shutil.which('true') or '/bin/true'
    if not bwrap:
        return fail(keys, 'bwrap not found')
    sandbox = [bwrap, '--ro-bind', '/', '/', '--die-with-parent']
    overlay = ['--overlay-src', tree, '--tmp-overlay', tree]
    # The fork shows the tree and takes a write that stays in its tmpfs upper (the rest of / is read-only in there).
    probe = os.path.join(tree, f'__arbor_fork_test_{os.getpid()}.txt')
    r = run(sandbox + overlay + ['sh', '-c', 'find "$1" -maxdepth 2 | wc -l; echo hello > "$2" && cat "$2"',
                                 'sh', tree, probe], timeout=120)
    out = r.stdout.decode(errors='replace').split()
    if r.returncode != 0 or out[1:] != ['hello']:
        err = (r.stderr.decode(errors='replace').strip().splitlines() or [f'exit status {r.returncode}'])[-1]
        return fail(keys, f'overlay through bwrap failed: {err}' + (BLOCKED_TOO if blocked else ''))
    leak = os.path.exists(probe)
    if leak:
        os.remove(probe)
    meds, errs = spawn_blocks([('sandbox', sandbox + [true_bin]), ('fork', sandbox + overlay + [true_bin])], blocks=5)
    if errs:
        return fail(keys, '; '.join(f'{k}: {e}' for k, e in errs.items()))
    share = [f - b for f, b in zip(meds['fork'], meds['sandbox'])]
    emit('storage.fork_bwrap_ms', statistics.median(meds['fork']),
         {'blocks': [round(x, 4) for x in meds['fork']], 'tree': os.path.basename(tree),
          'visible_entries_depth2': int(out[0]), 'write_reached_lower': 'yes' if leak else 'no'})
    emit('storage.fork_bwrap_overlay_ms', statistics.median(share),
         {'blocks': [round(x, 4) for x in share], 'sandbox_ms': round(statistics.median(meds['sandbox']), 4)})


def worktree(tree):
    key = 'storage.git_worktree_ms'
    if subprocess.run(['git', '-C', tree, 'rev-parse', '--git-dir'], capture_output=True).returncode != 0:
        return fail(key, f'{os.path.basename(tree)} is not a git repository')
    G = os.path.join(WORK, 'gw')
    os.makedirs(G, exist_ok=True)
    try:
        r = run(['git', 'clone', '-q', '--shared', '--no-checkout', tree, f'{G}/repo'], timeout=300)
        if r.returncode != 0:
            return fail(key, r.stderr.decode(errors='replace'))
        runs, disk = [], []
        for k in range(3):  # M timed one; three detached worktrees of HEAD, median
            d0, t0 = child_disk_mib(), time.perf_counter()
            r = run(['git', '-C', f'{G}/repo', 'worktree', 'add', '-q', '--detach', f'{G}/wt{k}', 'HEAD'], timeout=300)
            runs.append((time.perf_counter() - t0) * 1000)
            disk.append(round(child_disk_mib() - d0, 1))
            if r.returncode != 0:
                return fail(key, r.stderr.decode(errors='replace'))
        du = subprocess.run(['du', '-sm', f'{G}/wt0'], capture_output=True, text=True).stdout.split()
        files = subprocess.run(['git', '-C', f'{G}/wt0', 'ls-files'], capture_output=True).stdout.count(b'\n')
        emit(key, statistics.median(runs), {'checkout_mib': int(du[0]) if du else None, 'tracked_files': files,
                                            'runs_ms': [round(x) for x in runs], 'disk_mib': disk})
    finally:
        shutil.rmtree(G, ignore_errors=True)


def digest(task):
    """BLAKE2b of a whole file; O_DIRECT for big files (falls back to buffered reads)."""
    global _buf
    path, size = task
    h = hashlib.blake2b(digest_size=16)
    try:
        if size >= DIRECT_MIN:
            try:
                fd = os.open(path, os.O_RDONLY | os.O_DIRECT | os.O_NOFOLLOW)
            except OSError:
                fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                if _buf is None:
                    _buf = mmap.mmap(-1, 4 << 20)
                mv = memoryview(_buf)
                while True:
                    try:
                        n = os.readv(fd, [_buf])
                    except OSError:  # O_DIRECT refused by this filesystem: start over buffered
                        h = hashlib.blake2b(digest_size=16)
                        with open(path, 'rb') as f:
                            for chunk in iter(lambda: f.read(4 << 20), b''):
                                h.update(chunk)
                        break
                    if n <= 0:
                        break
                    h.update(mv[:n])
            finally:
                os.close(fd)
        else:
            with open(path, 'rb') as f:
                h.update(f.read())
    except OSError:
        return path, size, None
    return path, size, h.digest()


def dedup(root):
    keys = ['storage.dedup_pct', 'storage.dedup_gib', 'storage.total_gib', 'storage.files']
    t0 = time.time()
    out = subprocess.run(['find', root, '-type', 'f', '-printf', r'%D %i %s %p\0'], capture_output=True).stdout
    bysize, seen, total, n = collections.defaultdict(list), set(), 0, 0
    for rec in out.split(b'\0'):
        if not rec:
            continue
        dev, ino, size, p = rec.split(b' ', 3)
        size = int(size)
        if size == 0 or (dev, ino) in seen:
            continue
        seen.add((dev, ino))
        total += size
        n += 1
        bysize[size].append(os.fsdecode(p))
    del out, seen
    if not n:
        return fail(keys, f'no files under {root}')
    tasks = sorted(((p, s) for s, ps in bysize.items() if len(ps) > 1 for p in ps), key=lambda t: -t[1])
    del bysize
    groups = collections.defaultdict(list)
    unreadable = 0
    with Pool(min(20, os.cpu_count() or 4)) as pool:
        for p, s, dg in pool.imap_unordered(digest, tasks, chunksize=64):
            if dg is None:
                unreadable += 1
            else:
                groups[(s, dg)].append(p)
    by_class, dup = collections.Counter(), 0
    for (s, _), g in groups.items():
        if len(g) < 2:
            continue
        g.sort(key=lambda p: (len(p), p))  # keep the shortest path as the original
        for p in g[1:]:
            dup += s
            by_class[next((c.strip('/') for c in CLASSES if c in p), 'other')] += s
    ctx = {'files': n, 'total_gib': round(total / 2**30, 2), 'hashed_files': len(tasks), 'unreadable': unreadable,
           'scan_s': round(time.time() - t0)}
    emit('storage.files', n)
    emit('storage.total_gib', total / 2**30)
    emit('storage.dedup_gib', dup / 2**30, ctx)
    emit('storage.dedup_pct', 100 * dup / total, ctx)
    for c in ('.claude/worktrees', 'build', 'other', '.git', 'node_modules'):
        emit(f'storage.dedup.{c}', by_class[c] / 2**30)


def main():
    projects = env('PROJECTS_DIR')
    tree = env('FORK_TREE')
    if not tree and projects and os.path.isdir(projects):
        sizes = tree_sizes(projects)
        tree = max(sizes, key=sizes.get) if sizes else None
    if tree and os.path.isdir(tree):
        tree = os.path.abspath(tree)
        du = subprocess.run(['du', '-s', '--apparent-size', '--block-size=1', tree], capture_output=True, text=True)
        if du.stdout.split():
            emit('storage.fork_tree_gib', int(du.stdout.split()[0]) / 2**30, {'tree': os.path.basename(tree)})
        blocked = userns_blocked()
        try:
            fork(tree, blocked)
        except Exception as e:
            fail(['storage.fork_ms', 'storage.fork_syscall_ms'], e)
        try:
            fork_bwrap(tree, blocked)
        except Exception as e:
            fail(['storage.fork_bwrap_ms', 'storage.fork_bwrap_overlay_ms'], e)
        try:
            worktree(tree)
        except Exception as e:
            fail('storage.git_worktree_ms', e)
    else:
        fail(['storage.fork_ms', 'storage.fork_syscall_ms', 'storage.fork_bwrap_ms', 'storage.fork_bwrap_overlay_ms',
              'storage.git_worktree_ms'], 'no tree to fork (set FORK_TREE)')
    root = env('DEDUP_ROOT', projects)
    if root and os.path.isdir(root):
        try:
            dedup(root)
        except Exception as e:
            fail(['storage.dedup_pct', 'storage.dedup_gib'], e)
    else:
        fail(['storage.dedup_pct', 'storage.dedup_gib'], 'DEDUP_ROOT not set or missing')


if __name__ == '__main__':
    main()
