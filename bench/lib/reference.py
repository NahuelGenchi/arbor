"""M values: what the 16 Oct 2026 investigation measured on this PC, in the order the report lists them.

src 'opt' = the number is quoted in OPTIMIZATION.md (some rounded there); 'run' = it appears only in that
investigation's raw output, kept here so the rows next to the quoted ones have a baseline too. m=None marks a
metric this suite adds (no M value).

kind decides how a difference beyond the tolerance is explained:
  perf    timing or size on fixed inputs; expected to reproduce
  rss     resident set of a fresh process: mapped file pages follow the page cache
  pss     proportional set size: shared pages are split among all processes that map them
  corpus  depends on a data set that keeps changing ($HOME, the projects folder, the transcripts)
  input   depends on one input file that the suite pins (see config.sh)
  state   a reading of the live system (what runs, what is swapped out right now)
  boot    read from the current boot's systemd records
  config  installed files and settings (initramfs, modules, swap layout)
why(r) returns the specific explanation; r is report.Row access to every result (r.v(key), r.corpus(...)).
"""

M_INFO = {
    'date': '16 Oct 2026',
    'boot': '2026-10-16 10:53:50',  # `uptime -s` of the boot that systemd-analyze described
    'epp': 'performance',
    'initrd_bytes': 90693854,
    'git_ref': 'f44fb79aaf8f',
    'node': 'v22.23.3',
    'blame_read': '2026-10-16 15:51:23',  # when M ran systemd-analyze blame
}

TOL = 0.15  # relative; a row may also set abs_tol (MiB rows: M printed them to 0.1 MiB)


class E(dict):
    def __init__(self, key, sec, label, m, unit, kind, src='run', **kw):
        super().__init__(key=key, sec=sec, label=label, m=m, unit=unit, kind=kind, src=src, **kw)


PSS_WHY = ('PSS divides shared pages among every process that maps them, so it moves with what else runs; '
           'RSS is the stable floor')
SWAP_WHY = ('live reading: M was taken mid-investigation under memory pressure (9.6 GiB swapped, 0.9 GiB '
            'available, headless Chrome automation running)')
RAM_WHY = 'live reading: depends on which agents, apps and browser automation are open right now'
M_PRESSURE = ('faster than M: the M investigation ran under memory pressure (0.9 GiB RAM available, 9.6 GiB '
              'swapped), which slows what has to be paged back in')


def rss_why(r, key):
    file, anon = r.ctx(key, 'file_mib'), r.ctx(key, 'anon_mib')
    if file is None:
        return ''
    return (f'{file:.1f} MiB of it are file pages, {anon:.1f} MiB anonymous; file pages count when mapped, and a fault '
            'also maps the cached neighbours of the page it needs, so the reading moves with the page cache '
            f"({r.meta.get('mem_available_gib', '?')} GiB RAM available at the start)")

ENTRIES = [
    # 1. Runtimes
    E('rt.start.c_static', 'Runtime', 'start: static C', 0.21, 'ms', 'perf'),
    E('rt.start.true', 'Runtime', 'start: /bin/true (dynamic C)', 0.32, 'ms', 'perf'),
    E('rt.start.rust', 'Runtime', 'start: Rust (std)', 0.42, 'ms', 'perf', 'opt'),
    E('rt.start.busybox', 'Runtime', 'start: busybox true', 0.25, 'ms', 'perf'),
    E('rt.start.bash', 'Runtime', 'start: bash -c true', 0.66, 'ms', 'perf'),
    E('rt.start.python_IS', 'Runtime', 'start: python3 -I -S', 5.61, 'ms', 'perf'),
    E('rt.start.python', 'Runtime', 'start: python3', 8.39, 'ms', 'perf'),
    E('rt.start.node', 'Runtime', 'start: node -e 0', 12.84, 'ms', 'perf', 'opt'),
    E('rt.rss.c_static', 'Runtime', 'idle RSS: static C', 0.6, 'MiB', 'rss', 'opt', dp=2, abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.c_static')),
    E('rt.rss.rust', 'Runtime', 'idle RSS: Rust', 2.2, 'MiB', 'rss', 'opt', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.rust')),
    E('rt.rss.busybox_sh', 'Runtime', 'idle RSS: busybox sh', 1.4, 'MiB', 'rss', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.busybox_sh')),
    E('rt.rss.bash', 'Runtime', 'idle RSS: bash', 3.8, 'MiB', 'rss', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.bash')),
    E('rt.rss.python', 'Runtime', 'idle RSS: python3', 9.6, 'MiB', 'rss', 'opt', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.python')),
    E('rt.rss.python_net', 'Runtime', 'idle RSS: python3 + json,ssl,asyncio,http', 24.8, 'MiB', 'rss', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.python_net')),
    E('rt.rss.node', 'Runtime', 'idle RSS: node', 41.7, 'MiB', 'rss', 'opt', abs_tol=0.1,
      why=lambda r: rss_why(r, 'rt.rss.node')),
    E('rt.pss.c_static', 'Runtime', 'idle PSS: static C', 0.6, 'MiB', 'pss', dp=2, abs_tol=0.1),
    E('rt.pss.rust', 'Runtime', 'idle PSS: Rust', 0.4, 'MiB', 'pss', dp=2, abs_tol=0.1),
    E('rt.pss.busybox_sh', 'Runtime', 'idle PSS: busybox sh', 1.4, 'MiB', 'pss', abs_tol=0.1),
    E('rt.pss.bash', 'Runtime', 'idle PSS: bash', 0.7, 'MiB', 'pss', dp=2, abs_tol=0.1),
    E('rt.pss.python', 'Runtime', 'idle PSS: python3', 4.5, 'MiB', 'pss', abs_tol=0.1),
    E('rt.pss.python_net', 'Runtime', 'idle PSS: python3 + json,ssl,asyncio,http', 17.9, 'MiB', 'pss', abs_tol=0.1),
    E('rt.pss.node', 'Runtime', 'idle PSS: node', 39.1, 'MiB', 'pss', abs_tol=0.1),

    # 2. Cells
    E('cell.unshare_user', 'Cells', 'start: unshare user ns', 0.67, 'ms', 'perf'),
    E('cell.unshare_4ns', 'Cells', 'start: unshare user+net+pid+mnt', 1.28, 'ms', 'perf', 'opt'),
    E('cell.bwrap', 'Cells', 'start: bwrap --unshare-all', 3.08, 'ms', 'perf'),
    E('cell.docker', 'Cells', 'start: docker run --rm (median of 8)', 125, 'ms', 'perf', 'opt', dp=0,
      why=lambda r: f"runs {r.ctx('cell.docker', 'runs')} ms; docker start time follows dockerd/containerd state"),
    E('cell.py_fresh_ms', 'Cells', 'Python cell, 13 imports: fresh start', 46.4, 'ms', 'perf', 'opt'),
    E('cell.py_fork_ms', 'Cells', 'Python cell: fork of warm template', 0.73, 'ms', 'perf', 'opt'),
    E('cell.py_fresh_pss', 'Cells', 'Python cell PSS: fresh', 15.2, 'MiB', 'pss', 'opt', abs_tol=0.1),
    E('cell.py_fork_pss', 'Cells', 'Python cell PSS: forked', 2.1, 'MiB', 'pss', 'opt', abs_tol=0.1),
    E('cell.py_template_pss', 'Cells', 'Python template PSS', 8.1, 'MiB', 'pss', abs_tol=0.1),

    # 3. Memory
    E('mem.swapfile_gib', 'Memory', 'disk swap size', 8, 'GiB', 'config', 'opt', dp=1),
    E('mem.zram_gib', 'Memory', 'zram size', 4, 'GiB', 'config', 'opt', dp=1),
    E('mem.zram_algo', 'Memory', 'zram algorithm', 'lzo-rle', '', 'config'),
    E('mem.swap_used_gib', 'Memory', 'swap in use (zram + disk)', 9.6, 'GiB', 'state', 'opt', why=lambda r: SWAP_WHY),
    E('mem.swap_disk_used_gib', 'Memory', 'disk swap in use', 6, 'GiB', 'state', 'opt', why=lambda r: SWAP_WHY),
    E('mem.available_gib', 'Memory', 'RAM available', 0.9, 'GiB', 'state', 'opt', why=lambda r: SWAP_WHY),
    E('mem.zram_ratio', 'Memory', 'zram live ratio (data / RAM used)', 2.0, '×', 'state', 'opt', dp=2,
      why=lambda r: f"live ratio of whatever is swapped right now ({r.ctx('mem.zram_ratio', 'orig_gib')} GiB in zram; "
                    "M: 1.83 GiB); use the heap rows below for a fixed input"),
    E('ram.claude_code_mib', 'Memory', 'RSS: Claude Code sessions', 1568, 'MiB', 'state', 'opt', dp=0,
      why=lambda r: f"{r.ctx('ram.claude_code_mib', 'procs')} sessions now ({r.ctx('ram.claude_code_mib', 'per_process_mib')} "
                    "MiB each) vs 6 at M (250–334 MiB each); " + RAM_WHY),
    E('ram.electron_mib', 'Memory', 'RSS: Electron UI (Claude desktop)', 1354, 'MiB', 'state', 'opt', dp=0,
      why=lambda r: f"{r.ctx('ram.electron_mib', 'procs')} processes now vs 22 at M; " + RAM_WHY),
    E('ram.chrome_mib', 'Memory', 'RSS: Chrome (headless automation at M)', 2054, 'MiB', 'state', 'opt', dp=0,
      why=lambda r: f"{r.ctx('ram.chrome_mib', 'procs')} processes now vs 18 at M; " + RAM_WHY),
    E('ram.gnome_mib', 'Memory', 'RSS: GNOME stack', 513, 'MiB', 'state', 'opt', dp=0,
      why=lambda r: f"{r.ctx('ram.gnome_mib', 'procs')} processes now vs 64 at M; " + RAM_WHY),
    E('heap.mib', 'Memory', 'agent heap before GC (node holding a transcript)', 459, 'MiB', 'input', dp=0),
    E('heap.same_filled', 'Memory', 'heap before GC: same-filled pages', 9.0, '%', 'input'),
    E('heap.zstd3', 'Memory', 'heap per 4K page: zstd-3', 2.15, '×', 'input', 'opt', dp=2),
    E('heap.zstd1', 'Memory', 'heap per 4K page: zstd-1', 2.15, '×', 'input', dp=2),
    E('heap.lzo', 'Memory', 'heap per 4K page: lzo1x-1', None, '×', 'input', dp=2),
    E('heap.lz4', 'Memory', 'heap per 4K page: lz4', None, '×', 'input', dp=2),
    E('heap.deflate1', 'Memory', 'heap per 4K page: deflate-1', 2.06, '×', 'input', dp=2),
    E('heap.live_mib', 'Memory', 'agent heap after full GC (live objects only)', None, 'MiB', 'input', dp=0,
      note="M read only before GC; this GC frees the UTF-16 copy of the file text and shrinks the heap as V8's "
           "memory reducer does when an agent idles"),
    E('heap.live_same_filled', 'Memory', 'heap after GC: same-filled pages', None, '%', 'input'),
    E('heap.live_zstd3', 'Memory', 'heap after GC per 4K page: zstd-3', None, '×', 'input', dp=2),
    E('heap.live_lzo', 'Memory', 'heap after GC per 4K page: lzo1x-1', None, '×', 'input', dp=2),

    # 4. Search
    E('search.entries', 'Search', 'entries under $HOME', 2233713, '', 'corpus', 'opt',
      why=lambda r: 'files come and go in $HOME; the timing rows below scale with this count'),
    E('search.find_ms', 'Search', '`find -iname` (warm cache)', 1425.5, 'ms', 'corpus', 'opt',
      why=lambda r: r.corpus('search.entries', 'entries') + '; ' + r.per('search.find_ms', 'search.entries', 'µs per entry', 1000)
      + r.disk_note('search.find_ms', 'find', 8)),
    E('search.rg_files_ms', 'Search', '`rg --files -g` (warm)', 200.1, 'ms', 'corpus',
      why=lambda r: r.corpus('search.entries', 'entries') + '; rg is multi-threaded, so other load on the CPU shows here first'
      + r.disk_note('search.rg_files_ms', 'rg', 8)),
    E('search.catalog_ms', 'Search', 'catalog query (name trigrams + parent ids)', 0.32, 'ms', 'corpus', 'opt',
      why=lambda r: f"{r.ctx('search.catalog_ms', 'hits')} hits now vs 2 at M (each hit rebuilds its path); " + r.corpus('search.entries', 'entries')),
    E('search.catalog_mib', 'Search', 'catalog size', 191, 'MiB', 'corpus', 'opt', dp=0,
      why=lambda r: r.corpus('search.entries', 'entries')),
    E('search.catalog_bytes_per_obj', 'Search', 'catalog bytes per object', 90, 'B', 'corpus', 'opt', dp=0,
      why=lambda r: 'average name length changes with the corpus'),
    E('search.catalog_update_us', 'Search', 'catalog insert, per object', 2, 'µs', 'perf', 'opt', dp=1),
    E('search.catalog_build_s', 'Search', 'catalog build (one-time)', 5.2, 's', 'corpus',
      why=lambda r: r.corpus('search.entries', 'entries')),
    E('search.exact_name_us', 'Search', 'exact-name lookup (hash map)', 0.05, 'µs', 'perf', 'opt', dp=3,
      why=lambda r: 'at 50 ns the timer call itself is most of the reading'),
    E('code.files', 'Search', 'code corpus: text files', 33693, '', 'corpus',
      why=lambda r: 'projects folder changed since M'),
    E('code.mib', 'Search', 'code corpus: size', 248, 'MiB', 'corpus', 'opt', dp=0,
      why=lambda r: r.corpus('code.files', 'files')),
    E('code.index_mib', 'Search', 'trigram index size (SQLite FTS5)', 754, 'MiB', 'corpus', dp=0,
      why=lambda r: r.corpus('code.mib', 'MiB of text', fmtv='{:,.0f}')),
    E('code.build_s', 'Search', 'trigram index build (one-time)', 19.2, 's', 'corpus',
      why=lambda r: r.corpus('code.mib', 'MiB of text', fmtv='{:,.0f}')),
] + [
    # m_files: files M found for the query; m_cold: M's first rg query, most likely run partly from disk
    E(f'code.{kind}_ms.{i}', 'Search', f'{label} `{q}`', m, 'ms', 'corpus', 'opt', query=q, m_files=mf,
      m_cold=kind == 'rg' and i == 0, why=(lambda r, k=f'code.{kind}_ms.{i}': r.code_why(k)))
    for kind, label, ms, files in (('rg', 'code search: rg', (173.6, 39.5, 38.7, 38.6, 37.3), (63, 63, 277, 7, 89)),
                                   ('index', 'code search: index', (2.50, 3.85, 5.31, 1.44, 2.32), (66, 79, 290, 7, 89)))
    for i, (q, m, mf) in enumerate(zip(('parseInt', 'retroachievements', 'TODO:', 'fn main', 'border-radius'), ms, files))
] + [
    # 5. Boot
    E('boot.total_s', 'Boot', 'boot total (systemd-analyze)', 25.171, 's', 'boot', 'opt', dp=3),
    E('boot.firmware_s', 'Boot', 'firmware', 11.263, 's', 'boot', 'opt', dp=3),
    E('boot.loader_s', 'Boot', 'boot loader', 1.726, 's', 'boot', 'opt', dp=3),
    E('boot.kernel_s', 'Boot', 'kernel + initramfs', 6.512, 's', 'boot', 'opt', dp=3),
    E('boot.userspace_s', 'Boot', 'userspace', 5.668, 's', 'boot', 'opt', dp=3),
    E('boot.graphical_s', 'Boot', 'graphical.target reached', 5.642, 's', 'boot', dp=3),
    E('boot.kexec_skips_s', 'Boot', 'firmware + loader (skipped by kexec)', 12.989, 's', 'boot', 'opt', dp=3),
    E('boot.chain.plymouth_quit_wait_s', 'Boot', 'critical chain: plymouth-quit-wait', 2.038, 's', 'boot', 'opt', dp=3),
    E('boot.chain.slowest', 'Boot', 'critical chain: slowest unit', 'plymouth-quit-wait.service +2.038s', '', 'boot'),
    E('boot.blame.ua_timer', 'Boot', 'blame: ua-timer', 3.946, 's', 'boot', 'opt', dp=3),
    E('boot.blame.apt_daily_upgrade', 'Boot', 'blame: apt-daily-upgrade', 1.375, 's', 'boot', 'opt', dp=3),
    E('boot.blame.snapd', 'Boot', 'blame: snapd + snapd.seeded', 1.625, 's', 'boot', 'opt', dp=3),
    E('boot.blame.fwupd_refresh', 'Boot', 'blame: fwupd-refresh', 0.527, 's', 'boot', 'opt', dp=3),
    E('boot.blame.man_db', 'Boot', 'blame: man-db', 0.285, 's', 'boot', 'opt', dp=3),
    E('boot.initrd_mb', 'Boot', 'initramfs size', 90.7, 'MB', 'config', 'opt'),
    E('boot.initrd_modules', 'Boot', 'initramfs: kernel modules', 1099, '', 'config', 'opt'),
    E('boot.initrd_modules_loaded', 'Boot', 'initramfs modules this PC has loaded', 52, '', 'config', 'opt',
      why=lambda r: f"{r.v('boot.modules_loaded')} modules loaded now vs 187 at M: devices and features used since boot add modules"),
    E('boot.initrd_modules_loaded_mb', 'Boot', 'initramfs modules loaded: size', 1.56, 'MB', 'config', 'opt', dp=2,
      why=lambda r: f"{r.v('boot.initrd_modules_loaded')} of the initramfs modules are loaded now vs 52 at M"),
    E('boot.initrd_firmware_mb', 'Boot', 'initramfs: firmware', 17.2, 'MB', 'config', 'opt'),
    E('boot.initrd_microcode_mb', 'Boot', 'initramfs: CPU microcode', 15.1, 'MB', 'config', 'opt'),
    E('boot.initrd_userspace_mb', 'Boot', 'initramfs: userspace (udev, plymouth, ...)', 63.3, 'MB', 'config', 'opt'),
    E('boot.modules_disk_files', 'Boot', 'modules on disk (files)', 6903, '', 'config', 'opt'),
    E('boot.modules_disk_mib', 'Boot', 'modules on disk (size)', 172, 'MiB', 'config', 'opt', dp=0),
    E('boot.modules_loaded', 'Boot', 'modules loaded (/proc/modules)', 187, '', 'state', 'opt',
      why=lambda r: 'grows as devices and features are used after boot'),
    E('boot.squashfs_mounts', 'Boot', 'squashfs (snap) mounts', 14, '', 'config', 'opt',
      why=lambda r: 'snap refreshes add and remove revisions'),

    # 6. Storage
    E('storage.fork_tree_gib', 'Storage', 'fork tree size (largest project)', 28, 'GiB', 'corpus', 'opt', dp=1),
    E('storage.fork_ms', 'Storage', 'overlay CoW fork, `mount -t overlay` as timed at M', 4.317, 'ms', 'perf', 'opt',
      why=lambda r: f"7 runs {r.ctx('storage.fork_ms', 'runs_ms')} ms; M was a single run; the time is mostly starting "
                    "mount(8) and date(1), see the syscall row"),
    E('storage.fork_syscall_ms', 'Storage', 'overlay CoW fork, mount(2) syscall only', None, 'ms', 'perf', dp=3),
    E('storage.git_worktree_ms', 'Storage', '`git worktree add` (baseline)', 79, 'ms', 'perf', 'opt', dp=0,
      why=lambda r: f"runs {r.ctx('storage.git_worktree_ms', 'runs_ms')} ms (M timed one); checkout "
                    f"{r.ctx('storage.git_worktree_ms', 'checkout_mib')} MiB, {r.ctx('storage.git_worktree_ms', 'tracked_files')} "
                    "files now vs 19 MiB, 1,770 files at M" + r.disk_note('storage.git_worktree_ms', 'git', 4)),
    E('storage.files', 'Storage', 'projects folder: files (unique inodes)', 1002224, '', 'corpus',
      why=lambda r: 'the projects folder changed since M'),
    E('storage.total_gib', 'Storage', 'projects folder: size', 70.52, 'GiB', 'corpus', 'opt', dp=2,
      why=lambda r: r.corpus('storage.files', 'files')),
    E('storage.dedup_gib', 'Storage', 'projects folder: duplicate bytes', 22.89, 'GiB', 'corpus', 'opt', dp=2,
      why=lambda r: r.corpus('storage.total_gib', 'GiB scanned', fmtv='{:,.2f}')),
    E('storage.dedup_pct', 'Storage', 'projects folder: duplicate share', 32.5, '%', 'corpus', 'opt',
      why=lambda r: r.corpus('storage.total_gib', 'GiB scanned', fmtv='{:,.2f}')),
    E('storage.dedup..claude/worktrees', 'Storage', 'duplicates in agent worktrees', 6.76, 'GiB', 'corpus', 'opt', dp=2,
      why=lambda r: 'agent worktrees are created and removed as sessions run'),
    E('storage.dedup.build', 'Storage', 'duplicates in build dirs', 7.24, 'GiB', 'corpus', 'opt', dp=2,
      why=lambda r: 'build dirs change with every build'),
    E('storage.dedup.other', 'Storage', 'duplicates elsewhere', 7.76, 'GiB', 'corpus', dp=2,
      why=lambda r: r.corpus('storage.total_gib', 'GiB scanned', fmtv='{:,.2f}')),
    E('storage.dedup..git', 'Storage', 'duplicates in .git', 0.89, 'GiB', 'corpus', dp=2,
      why=lambda r: r.corpus('storage.total_gib', 'GiB scanned', fmtv='{:,.2f}')),
    E('storage.dedup.node_modules', 'Storage', 'duplicates in node_modules', 0.12, 'GiB', 'corpus', dp=2,
      why=lambda r: r.corpus('storage.total_gib', 'GiB scanned', fmtv='{:,.2f}')),

    # 7. Tokens
    E('tok.sessions', 'Tokens', 'sessions (transcripts with model calls)', 201, '', 'corpus', 'opt',
      why=lambda r: 'every Claude Code session since M adds a transcript'),
    E('tok.calls', 'Tokens', 'model calls', 47700, '', 'corpus', 'opt', why=lambda r: r.corpus('tok.sessions', 'sessions')),
    E('tok.cache_read_share_input', 'Tokens', 'input read from cache', 98.7, '%', 'corpus', 'opt'),
    E('tok.cost.cache_read', 'Tokens', 'cost share: cache reads', 74.3, '%', 'corpus', 'opt'),
    E('tok.cost.cache_write', 'Tokens', 'cost share: cache writes', 11.9, '%', 'corpus'),
    E('tok.cost.output', 'Tokens', 'cost share: output', 13.8, '%', 'corpus'),
    E('tok.cost.input', 'Tokens', 'cost share: uncached input', 0.0, '%', 'corpus', abs_tol=0.5),
    E('tok.saving_vs_uncached', 'Tokens', 'saving vs no cache', 87, '%', 'corpus', 'opt', dp=1),
    E('tok.opening_ctx_median', 'Tokens', 'opening context, median', 70307, 'tok', 'corpus', 'opt'),
    E('tok.opening_ctx_share_of_reads', 'Tokens', 'opening context share of cache reads', 23.0, '%', 'corpus', 'opt'),
    E('tok.ctx_per_call_median', 'Tokens', 'context per call, median', 266161, 'tok', 'corpus', 'opt'),
    E('tok.calls_per_session_median', 'Tokens', 'calls per session, median', 68, '', 'corpus'),
    E('tok.step_cost', 'Tokens', 'cost per step (input-token equivalents)', 42223, 'tok', 'corpus', 'opt'),
    E('tok.readonly_calls', 'Tokens', 'steps that only read', 31.5, '%', 'corpus'),
    E('tok.followup_reads_calls', 'Tokens', 'read-only steps right after another', 16.1, '%', 'corpus', 'opt'),
    E('tok.followup_reads_cost', 'Tokens', 'their share of cost', 13.1, '%', 'corpus', 'opt'),
    E('tok.tool_results', 'Tokens', 'tool results', 51441, '', 'corpus', why=lambda r: r.corpus('tok.sessions', 'sessions')),
    E('tok.tool_result_median', 'Tokens', 'tool result, median', 209, 'tok', 'corpus', 'opt'),
    E('tok.tool_result_p90', 'Tokens', 'tool result, p90', 1455, 'tok', 'corpus'),
    E('tok.tool_result_p99', 'Tokens', 'tool result, p99', 5256, 'tok', 'corpus'),
    E('tok.tool_errors', 'Tokens', 'tool results that are errors', 1.8, '%', 'corpus', 'opt'),
    E('tok.repeat_reads', 'Tokens', 'repeat reads of an unchanged range', 2.0, '%', 'corpus', 'opt',
      why=lambda r: f"{r.ctx('tok.repeat_reads', 'rereads')} of {r.ctx('tok.repeat_reads', 'reads'):,} Read calls now vs "
                    f"68 of 3,321 at M: the {r.ctx('tok.repeat_reads', 'reads') - 3321:,} Read calls since M include "
                    f"{r.ctx('tok.repeat_reads', 'rereads') - 68} repeats"),
    E('tok.images', 'Tokens', 'screenshots and images', 3461, '', 'corpus', 'opt'),
    E('tok.image_tokens_std', 'Tokens', 'image tokens, standard tier', 2.92, 'Mtok', 'corpus', 'opt', dp=2),
    E('tok.image_tokens_hi', 'Tokens', 'image tokens, high-res tier', 3.85, 'Mtok', 'corpus', 'opt', dp=2),
    E('fmt.ls_human', 'Tokens', '100-file listing: `ls -la`', 2776, 'tok', 'config'),
    E('fmt.json_pretty', 'Tokens', '100-file listing: pretty JSON', 6283, 'tok', 'config', 'opt'),
    E('fmt.json_min', 'Tokens', '100-file listing: minified JSON', 4163, 'tok', 'config'),
    E('fmt.yaml', 'Tokens', '100-file listing: YAML', 4957, 'tok', 'config'),
    E('fmt.markdown', 'Tokens', '100-file listing: Markdown table', 3285, 'tok', 'config'),
    E('fmt.tsv', 'Tokens', '100-file listing: TSV', 2690, 'tok', 'config'),
    E('fmt.toon', 'Tokens', '100-file listing: table (TOON-style)', 2916, 'tok', 'config', 'opt'),
    E('fmt.names', 'Tokens', '100-file listing: names only', 401, 'tok', 'config', 'opt'),
    E('fmt.git_log', 'Tokens', '`git log -20`', 18928, 'tok', 'input', 'opt'),
    E('fmt.git_log_stat', 'Tokens', '`git log --stat -20`', 22251, 'tok', 'input'),
    E('fmt.git_log_oneline', 'Tokens', '`git log --oneline -20`', 530, 'tok', 'input', 'opt'),
    E('fmt.ps_aux', 'Tokens', '`ps aux`, top 30 by RSS', 9900, 'tok', 'state', 'opt',
      why=lambda r: 'command lines of whatever runs now; M had long Chrome and Electron command lines on top'),
    E('fmt.ps_fields', 'Tokens', '`ps -eo pid,rss,comm`, top 30', 286, 'tok', 'state', 'opt'),
    E('fmt.ip_addr', 'Tokens', '`ip addr`', 534, 'tok', 'state'),
    E('fmt.ip_br', 'Tokens', '`ip -br addr`', 124, 'tok', 'state'),
    E('fmt.df', 'Tokens', '`df -h`', 274, 'tok', 'state'),
    E('fmt.df_real', 'Tokens', '`df -h` real filesystems only', 73, 'tok', 'state'),
]
