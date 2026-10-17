"""3b. Per-4K-page compressibility of an agent-like process heap: zstd vs lzo (plus deflate and lz4).

A node process parses a large Claude Code transcript and keeps every line in memory, the way an agent runtime holds
its context. Its anonymous mappings are then read page by page through /proc/<pid>/mem (allowed for a child under
yama ptrace_scope 1) and every 4 KiB page is compressed on its own, as zram does. Same-filled pages are counted
apart (zram stores them for free); a compressed page is capped at 4096 bytes (zram stores incompressible pages raw).

The heap is read twice:
  1. with the file text still referenced: the M state. The text is a UTF-16 string twice the file size; M read
     the heap a second after loading, before V8's first major GC had freed it (459 MiB then, ~217 MiB after).
     That GC lands anywhere from before 'loaded' to a second after, so this run holds the text to make it repeatable.
     V8's memory reducer is off: this read takes ~10 s (M's took ~2 s) and the reducer would shrink the heap midway.
  2. after dropping the text and a full memory-reducing GC (what the reducer does once an agent idles): live
     objects only (heap.live_* rows).

Source transcript: $HEAP_JSONL, else the transcript of exactly $HEAP_JSONL_BYTES bytes (the M input), else the
3rd-largest top-level transcript (the M rule).
lzo is LZO1X-1 from liblzo2 (userspace); the kernel's lzo-rle is LZO1X-1 with a run-length extension.
"""
import ctypes
import ctypes.util
import glob
import os
import shutil
import subprocess
import sys
import time
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import emit, env, fail  # noqa: E402

KEYS = ['heap.zstd3', 'heap.zstd1', 'heap.lzo', 'heap.lz4', 'heap.deflate1', 'heap.same_filled', 'heap.mib',
        'heap.live_mib', 'heap.live_same_filled', 'heap.live_zstd3', 'heap.live_lzo']
PAGE = 4096
JS = r"""
const fs = require('fs');
let text = fs.readFileSync(process.argv[1], 'utf8');
const lines = text.split('\n').filter(Boolean).map(l => { try { return JSON.parse(l) } catch (e) { return null } });
globalThis.keep = lines; globalThis.text = text; text = null;
console.log('loaded', lines.length);
process.stdin.once('data', () => {
  globalThis.text = null;
  gc({ type: 'major', execution: 'sync', flavor: 'last-resort' });
  console.log('collected');
});
setTimeout(() => {}, 120000);
"""


def pick_source():
    if env('HEAP_JSONL'):
        return env('HEAP_JSONL')
    root = env('TRANSCRIPTS_DIR', os.path.expanduser('~/.claude/projects'))
    files = sorted(glob.glob(os.path.join(root, '*', '*.jsonl')), key=os.path.getsize)
    want = int(env('HEAP_JSONL_BYTES', '0'))
    exact = [f for f in files if os.path.getsize(f) == want]
    if exact:
        return exact[0]
    return files[-3] if len(files) >= 3 else (files[-1] if files else None)


def compressors():
    out = []
    try:
        from compression import zstd  # Python 3.14+
        out += [('heap.zstd3', lambda b: len(zstd.compress(b, 3))), ('heap.zstd1', lambda b: len(zstd.compress(b, 1)))]
    except ImportError:
        fail(['heap.zstd3', 'heap.zstd1'], 'compression.zstd needs Python 3.14+')
    lzo_name = ctypes.util.find_library('lzo2') or 'liblzo2.so.2'
    try:
        lzo = ctypes.CDLL(lzo_name)
        ver = lzo.lzo_version()
        # lzo_init(): sizes of short, int, long, lzo_uint32_t, lzo_uint, dict entry, char*, void*, lzo_callback_t
        if lzo.__lzo_init_v2(ctypes.c_uint(ver), 2, 4, 8, 4, 8, 8, 8, 8, 48) != 0:
            raise OSError('lzo_init failed')
        dst, dlen, wrk = ctypes.create_string_buffer(2 * PAGE), ctypes.c_size_t(0), ctypes.create_string_buffer(1 << 20)
        out_buf, olen = ctypes.create_string_buffer(PAGE), ctypes.c_size_t(PAGE)

        def lzo1x(b):
            if lzo.lzo1x_1_compress(b, ctypes.c_size_t(len(b)), dst, ctypes.byref(dlen), wrk) != 0:
                raise OSError('lzo1x_1_compress failed')
            return dlen.value

        def lzo_check(b):  # round trip, proves the ctypes binding
            lzo1x(b)
            olen.value = PAGE
            return lzo.lzo1x_decompress_safe(dst, dlen, out_buf, ctypes.byref(olen), None) == 0 and out_buf.raw[:olen.value] == b
        out.append(('heap.lzo', lzo1x))
    except OSError as e:
        lzo_check = None
        fail('heap.lzo', f'liblzo2 not usable: {e}')
    try:
        lz4 = ctypes.CDLL(ctypes.util.find_library('lz4') or 'liblz4.so.1')
        ldst = ctypes.create_string_buffer(2 * PAGE)
        out.append(('heap.lz4', lambda b: lz4.LZ4_compress_default(b, ldst, len(b), 2 * PAGE)))
    except OSError as e:
        fail('heap.lz4', f'liblz4 not usable: {e}')
    out.append(('heap.deflate1', lambda b: len(zlib.compress(b, 1))))
    return out, lzo_check


def anon_regions(pid):
    with open(f'/proc/{pid}/maps') as m:
        for line in m:
            a, perm, off, dev, ino, *name = line.split()
            if ino != '0' or 'r' not in perm:
                continue  # anonymous, readable mappings only
            if name and name[0] in ('[vvar]', '[vsyscall]', '[vdso]', '[vvar_vclock]'):
                continue
            s, e = (int(x, 16) for x in a.split('-'))
            yield s, e


def pages(pid):
    """Yield every readable 4 KiB page of the anonymous mappings (read 256 KiB at a time, page by page on errors)."""
    with open(f'/proc/{pid}/mem', 'rb', 0) as mem:
        for s, e in anon_regions(pid):
            for chunk in range(s, e, 1 << 18):
                n = min(1 << 18, e - chunk)
                try:
                    mem.seek(chunk)
                    buf = mem.read(n)
                except OSError:
                    buf = None
                if buf is not None and len(buf) == n:
                    for i in range(0, n, PAGE):
                        yield buf[i:i + PAGE]
                    continue
                for addr in range(chunk, chunk + n, PAGE):
                    try:
                        mem.seek(addr)
                        b = mem.read(PAGE)
                    except OSError:
                        continue
                    if len(b) == PAGE:
                        yield b


def measure(pid, comps, lzo_check=None):
    """-> pages read, same-filled pages, compressed bytes and seconds per compressor, lzo round trips (checked, ok)"""
    n = same = checked = ok = 0
    size = {k: 0 for k, _ in comps}
    secs = {k: 0.0 for k, _ in comps}
    for b in pages(pid):
        n += 1
        if b.count(b[0:1]) == PAGE:
            same += 1
            continue
        for k, fn in comps:
            t0 = time.perf_counter()
            size[k] += min(fn(b), PAGE)
            secs[k] += time.perf_counter() - t0
        if lzo_check and n % 97 == 0:
            checked += 1
            ok += lzo_check(b)
    return n, same, size, secs, (checked, ok)


def main():
    src = pick_source()
    if not src:
        return fail(KEYS, 'no transcript found for the heap test')
    node = shutil.which('node')
    if not node:
        return fail(KEYS, 'node not found')
    node_version = subprocess.run([node, '--version'], capture_output=True, text=True).stdout.strip()
    comps, lzo_check = compressors()
    p = subprocess.Popen([node, '--expose-gc', '--no-memory-reducer', '-e', JS, src], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    try:
        first = p.stdout.readline().split()
        lines = int(first[1]) if len(first) == 2 else None
        time.sleep(1)  # as M
        before = measure(p.pid, comps, lzo_check)
        p.stdin.write('gc\n')
        p.stdin.flush()
        collected = p.stdout.readline().strip() == 'collected'
        time.sleep(0.5)
        live = measure(p.pid, [c for c in comps if c[0] in ('heap.zstd3', 'heap.lzo')]) if collected else None
    finally:
        p.kill()
        p.wait()
    n, same, size, secs, (checked, ok) = before
    data = n - same
    if not data:
        return fail(KEYS, 'no pages read (is ptrace access to child processes blocked?)')
    ctx = {'source_mib': os.path.getsize(src) // 2**20, 'lines': lines, 'pages': n, 'node': node_version}
    for k, _ in comps:
        c = dict(ctx, mib_per_s=round(data * PAGE / secs[k] / 2**20) if secs[k] else None)
        if k == 'heap.lzo':
            c['roundtrip_checked'] = f'{ok}/{checked}'
        emit(k, data * PAGE / size[k], c)
    emit('heap.same_filled', 100 * same / n, ctx)
    emit('heap.mib', n * PAGE / 2**20, ctx)
    if live is None:
        return fail(['heap.live_mib', 'heap.live_same_filled', 'heap.live_zstd3', 'heap.live_lzo'],
                    'node did not confirm the forced GC')
    n, same, size, _, _ = live
    ctx = dict(ctx, pages=n)
    emit('heap.live_mib', n * PAGE / 2**20, ctx)
    emit('heap.live_same_filled', 100 * same / n, ctx)
    for k in size:
        emit(k.replace('heap.', 'heap.live_'), (n - same) * PAGE / size[k], ctx)


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        fail(KEYS, e)
