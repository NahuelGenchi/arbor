"""M values: what the 16 Oct 2026 investigation measured on this PC, in the order the report lists them.

src 'opt' = the number is quoted in OPTIMIZATION.md (some rounded there); 'run' = it appears only in that
investigation's raw output, kept here so the rows next to the quoted ones have a baseline too. m=None marks a
metric this suite adds (no M value). src 'suite' = not an M value: a row added after the investigation, whose m is
the suite's own reference from a later run on this PC (ref_date, and ref_load, the load average at its start).

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


# The arbor rows' reference: the last of five `ONLY=arbor` runs in a row from an ordinary context (a systemd --user
# unit), the one closest to their medians, with arbor's state on tmpfs and `arbor serve` started by the section.
ARBOR_REF = {'ref_date': '17 Oct 2026', 'ref_load': 1.18}
# The rows of the network a key opens came later, from five more runs made the same way: the one closest to their
# medians. Their load averages at the start were 0.51, 0.62, 1.37, 1.74 (ref_load) and 1.89; in all five, arbor.run_ms
# read within 3% of its reference and arbor.run_cgroup_ms within 6%.
ARBOR_NET_REF = {'ref_date': '17 Oct 2026', 'ref_load': 1.74}
# The `arbor look` rows came after those, from five runs of the section with ARBOR_BIN set to a release build, made the
# same way in an A/B with the build before them: the one closest to the median of arbor.look_ms. The token rows read
# the same in all five, and the rows before them within 2% of the other build's, but for keep and undo.
ARBOR_LOOK_REF = {'ref_date': '17 Oct 2026', 'ref_load': 2.02}
# The rows of a large file read in parts came last, from five more runs made the same way, in an A/B with the build
# before them: the one closest to the median of arbor.look_ms. Their load averages at the start were 0.32, 0.41, 0.49,
# 0.50 (ref_load) and 0.55. The token rows read the same in all five; the others within 3% of the other build's
# medians, but for arbor.ask_ms, the difference of two close medians (0.48–0.64 ms here, 0.57–0.80 ms there), and the
# binary, 0.07 MiB larger. The reference of `arbor look` alone, which now describes outline and lines, is from them.
ARBOR_OUTLINE_REF = {'ref_date': '17 Oct 2026', 'ref_load': 0.5}
# The references of bench/sessions, which run.sh does not run: A/Bs of whole OpenCode sessions with and without
# `arbor look`, per arm, its report's ratios against A over all tasks: (ratio of medians, 95% interval), and the
# sessions that passed their check in A and in the arm. sessions/summary.py prints each under a new run's ratios. B is
# from the first A/B (sessions/ab.py --reps 10), 119 of its 120 sessions (the free model's rate limit ended every
# attempt of the last). C and D are from the second (#74, --arms A,C,D --reps 10), 156 of its 180 sessions: the rate
# limit stopped it there, and it was recorded without the other 24. It resumed after 26 sessions at load 0.56.
SESSIONS_REF = {
    'B': {'ref_date': '17 Oct 2026', 'ref_load': 0.47, 'sessions': 119, 'ok': ('58/60', '57/59'),
          'weighted': (1.01, 0.85, 1.15), 'steps': (1.27, 1.19, 1.52), 'input': (0.82, 0.65, 1.00),
          'output': (1.18, 0.99, 1.43), 'tool text': (0.61, 0.54, 0.76)},
    'C': {'ref_date': '17 Oct 2026', 'ref_load': 6.60, 'sessions': 156, 'ok': ('47/52', '50/52'),
          'weighted': (1.05, 0.86, 1.23), 'steps': (1.02, 0.89, 1.14), 'input': (1.11, 0.86, 1.36),
          'output': (0.99, 0.84, 1.12), 'tool text': (1.10, 0.83, 1.28)},
    'D': {'ref_date': '17 Oct 2026', 'ref_load': 6.60, 'sessions': 156, 'ok': ('47/52', '49/52'),
          'weighted': (1.04, 0.84, 1.19), 'steps': (0.98, 0.87, 1.12), 'input': (1.06, 0.76, 1.28),
          'output': (1.03, 0.93, 1.20), 'tool text': (0.93, 0.75, 1.13)},
}


def cycles(r, key):
    runs = r.ctx(key, 'runs_ms')
    return f'its {len(runs)} cycles took {min(runs):.1f}–{max(runs):.1f} ms' if runs else ''


def parts(r):
    """The rows of a file read in parts move with the reader in the binary measured, or with the files it reads."""
    files, ref = r.ctx('arbor.outline_pct', 'files') or {}, r.ctx('arbor.outline_pct', 'ref')
    return (f'the binary measured outlines files differently, or reads others: the {len(files)} largest code files of '
            f'commit {ref}')


def hops(r):
    """A connection passes through a few processes, each of them run where the scheduler puts it."""
    why = 'a connection lasts a fraction of a millisecond, in the client, the server and, through the proxy, arbor'
    if 'E cores' in str(r.meta.get('cores', '')):
        why += '; this CPU mixes P- and E-cores, and each of them runs slower on an E-core'
    return why


def placement(r, *parts):
    """A row timed on one arbor process at a time moves with the kind of core the scheduler puts it on."""
    if 'E cores' in str(r.meta.get('cores', '')):
        parts += ('this CPU mixes P- and E-cores, and one arbor process runs slower on an E-core',)
    return '; '.join(p for p in parts if p)


# The screen rows (sections/screen.py), the baseline the Mission Control interface is compared against. The still
# screen's CPU is the report's M number; the other rows came after M, and their reference is the suite's own: the
# medians of four `ONLY=screen` runs in a row from an ordinary context (a systemd --user unit), with nobody at the PC.
# Their load averages at the start were 0.45 (ref_load), 1.06, 6.55 and 6.11 (the last two left over from the cells
# of the run before); over the four, the compositor woke 91–214 times a second, 1 to 4 of about 1,200 refreshes
# showed no new frame idle, and 2.8–13.3% under load, so the wakeups and the missed frames move well beyond ±15%
# from run to run.
SCREEN_REF = {'ref_date': '17 Oct 2026', 'ref_load': 0.45}
MISSED = {'dp': 2, 'abs_tol': 0.1,
          'abs_why': 'within 1 frame in 1,000 of the reference, the precision the target in docs/INTERFACE.md is set in'}


def still_why(r, key):
    """What the screen and the rest of the machine did during the still-screen window."""
    gpu, rest, busy = (r.ctx(key, f) for f in ('gpu_ms', 'rest_cpu_pct', 'busy'))
    drew = ("the driver does not report the compositor's GPU time" if gpu is None else
            'the compositor drew nothing in the window (no GPU time)' if not gpu else
            f'the compositor drew frames in the window, so something on the screen changed ({gpu:,.0f} ms of GPU time)')
    by_thread = r.ctx(key, 'by_thread')
    return ((f'by thread: {by_thread}; ' if by_thread else '') + f"{drew}; the rest of the machine used {rest:.0f}% "
            "of a core" + (f' (busy: {busy})' if busy else '') + f"; load average {r.ctx(key, 'load1')} at its start")


def toolkit_note(c, metric):
    """What a toolkit row's record says beside its value (sections/toolkits.py): c is the row's ctx."""
    def first(*fields):
        return next((c[f] for f in fields if c.get(f) is not None), None)
    if metric == 'first_frame_ms':
        runs = c.get('launches_ms') or []
        return (f"launches {min(runs):.0f}–{max(runs):.0f} ms; loaded {c.get('loaded_ms')} ms, connected "
                f"{c.get('connected_ms')} ms, first frame committed {c.get('committed_ms')} ms after the start; "
                f"load average {c.get('load1')}") if runs else ''
    if metric.startswith(('rss_', 'pss_')):
        gpu = (f"; on the GPU {c.get('gpu_vram_mib')} MiB VRAM, {c.get('gpu_gtt_mib')} MiB GTT"
               if c.get('gpu_vram_mib') is not None else '')
        drv = f"; drivers {', '.join(c['drivers'])}" if c.get('drivers') else ''
        return (f"{c.get('procs')} process{'es' if c.get('procs') != 1 else ''}; PSS {c.get('pss_anon_mib')} MiB "
                f"anonymous, {c.get('pss_file_mib')} MiB file, {c.get('pss_shmem_mib')} MiB shared" + gpu + drv)
    if metric == 'still_frames_s':
        return (f"{c['frames']} frames shown in the {c.get('window_s')} s window; load average {c.get('load1')}"
                if c.get('frames') is not None else '')
    if metric.startswith('still_'):
        gpu = c.get('gpu_ms')
        drew = ('' if gpu is None else '; it drew nothing (no GPU time)' if not gpu else
                f'; it drew frames ({gpu:,.0f} ms of GPU time)')
        return (f"by thread: {c.get('by_thread') or 'none'}{drew}; the compositor used {c.get('compositor_cpu_pct')}% "
                f"and woke {c.get('compositor_wakeups_s')} times a second; load average {c.get('load1')}")
    if metric == 'frame_work_p99_ms':
        return (f"{c['frames']:,} frames: mean {c.get('mean_ms')} ms, median {c.get('median_ms')} ms, the most "
                f"{c.get('worst_ms')} ms") if c.get('frames') else ''
    if metric.startswith(('missed_', 'frame_')):
        if not c.get('frames'):
            return ''
        load = (f"; {c['cells']} busy cells and a {c['spike_mib']:,} MiB spike"
                + (f", {c['missed_during_spike']} missed while it touched its memory"
                   if c.get('missed_during_spike') is not None else '') if c.get('cells') is not None else '')
        gpu = f", {c['gpu_ms_per_frame']} ms of GPU time a frame" if c.get('gpu_ms_per_frame') is not None else ''
        return (f"{c['frames']:,} frames at {c.get('refresh_hz')} Hz, {c.get('missed')} missed, the slowest "
                f"{c.get('worst_ms')} ms; {first('cpu_pct')}% of a core in all{gpu}; by thread: "
                f"{c.get('threads')}{load}; load average {c.get('load1')}")
    if metric == 'atspi_objects':
        if not c.get('registered'):
            return 'not on the accessibility bus'
        if c.get('expected'):
            missing = f" (missing: {', '.join(c['missing'])})" if c.get('missing') else ''
            return (f"{c.get('toolkit')}; readable: {c.get('expected_found')} of the {c.get('expected')} words looked "
                    f"for{missing}; roles: {c.get('roles')}")
        seen = [w for w, ok in (('the heading', c.get('heading')), ('the card', c.get('card'))) if ok]
        return (f"{c.get('toolkit')}; readable: {', '.join(seen) or 'neither the heading nor the card'}, "
                f"{c.get('row_titles')} row titles and {c.get('row_words')} rows' words of 300; roles: {c.get('roles')}")
    return ''


def frames_why(r, key):
    """How the test window's frames went, and what ran beside them."""
    c = lambda f: r.ctx(key, f)  # noqa: E731
    parts = [f"{c('missed')} of {c('refreshes'):,} refreshes at {c('refresh_hz')} Hz showed no new frame, and the "
             f"slowest frame took {c('worst_ms')} ms"]
    if c('cells') is not None:
        spike = f"touched in {c('spike_s')} s" if c('spike_s') is not None else 'still being touched at the end'
        during = c('missed_during_spike')
        parts.append(f"{c('cells')} busy cells and a {c('spike_mib'):,} MiB spike, {spike}"
                     + (f', {during} of the missed frames while it touched its memory' if during is not None else ''))
    if c('focus') is False:
        parts.append('the test window did not have the focus at the end, so something may have covered it')
    parts.append(f"load average {c('load1')} at its start" + (f" (busy: {c('busy')})" if c('busy') else ''))
    return '; '.join(parts)


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
    # Through bwrap, which runs where AppArmor blocks unshare (Ubuntu 24.04+): a different method, so its own rows.
    # Reference: `ONLY="cells storage"` run from an ordinary context (a systemd --user unit), 17 Oct 2026.
    E('storage.fork_bwrap_ms', 'Storage', 'overlay CoW fork through bwrap, `true` started in it', 1.99, 'ms', 'perf',
      'suite', ref_date='17 Oct 2026', ref_load=10.76),
    E('storage.fork_bwrap_overlay_ms', 'Storage', "overlay CoW fork through bwrap, the overlay's share", 0.099, 'ms',
      'perf', 'suite', dp=3, ref_date='17 Oct 2026', ref_load=10.76, blocks_label='block differences',
      why=lambda r: f"the difference of two close medians: {r.v('storage.fork_bwrap_ms'):.2f} ms with the overlay, "
                    f"{r.ctx('storage.fork_bwrap_overlay_ms', 'sandbox_ms'):.2f} ms without"),
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

    # 8. The arbor prototype, built from this checkout (sections/arbor.py). It came after M, so its rows have no M
    # value; their reference is the suite's own (ARBOR_REF).
    E('arbor.binary_mib', 'Prototype', 'the arbor binary, release build', 1.23, 'MiB', 'config', 'suite', dp=2,
      why=lambda r: "the prototype's code, its dependencies or its toolchain changed since the reference", **ARBOR_REF),
    E('arbor.new_ms', 'Prototype', 'make a world: `arbor new`', 2.01, 'ms', 'perf', 'suite', **ARBOR_REF,
      why=placement),
    E('arbor.run_ms', 'Prototype', 'start a sealed command: `arbor run W -- true`, no cgroup', 4.79, 'ms', 'perf',
      'suite', **ARBOR_REF),
    E('arbor.run_cgroup_ms', 'Prototype', 'start a sealed command in a cgroup of its own', 5.13, 'ms', 'perf', 'suite',
      **ARBOR_REF),
    E('arbor.run_net_ms', 'Prototype', 'the same under a key that opens a host (`--net`)', 5.72, 'ms', 'perf', 'suite',
      **ARBOR_NET_REF),
    E('arbor.ask_ms', 'Prototype', "ask for an effect from inside: its share of the command's time", 0.61, 'ms',
      'perf', 'suite', dp=2, blocks_label='block differences', **ARBOR_REF,
      why=lambda r: f"the difference of two close medians: {r.ctx('arbor.ask_ms', 'with_ask_ms'):.2f} ms with the "
                    f"ask, {r.ctx('arbor.ask_ms', 'run_ms'):.2f} ms without; each ask reads every effect in the "
                    "outbox (0 to 128 here), so later blocks read more"),
    E('arbor.release_ms', 'Prototype', 'release an effect (`true`), with its receipt', 1.47, 'ms', 'perf', 'suite',
      **ARBOR_REF),
    E('arbor.tunnel_ms', 'Prototype', 'a connection from inside a world, through its proxy', 0.149, 'ms', 'perf',
      'suite', dp=3, **ARBOR_NET_REF, why=hops),
    E('arbor.tunnel_direct_ms', 'Prototype', 'the same connection made directly, outside any world', 0.0268, 'ms',
      'perf', 'suite', dp=3, **ARBOR_NET_REF, why=hops),
    E('arbor.keep_ms', 'Prototype', 'keep a world that added 1,000 files', 6.22, 'ms', 'perf', 'suite', **ARBOR_REF,
      why=lambda r: placement(r, cycles(r, 'arbor.keep_ms'))),
    E('arbor.undo_ms', 'Prototype', 'undo that keep', 4.09, 'ms', 'perf', 'suite', **ARBOR_REF,
      why=lambda r: placement(r, cycles(r, 'arbor.undo_ms'))),
    E('arbor.serve_rss_mib', 'Prototype', 'idle RSS: `arbor serve`', 2.66, 'MiB', 'rss', 'suite', dp=2, **ARBOR_REF,
      why=lambda r: rss_why(r, 'arbor.serve_rss_mib')),
    E('arbor.serve_pss_mib', 'Prototype', 'idle PSS: `arbor serve`', 0.87, 'MiB', 'pss', 'suite', dp=2, **ARBOR_REF),
    E('arbor.diff100_tokens', 'Tokens', '`arbor diff`, 100 files changed', 700, 'tok', 'input', 'suite', **ARBOR_REF,
      why=lambda r: '`arbor diff` lists the same 100 changes differently than at the reference'),
    E('arbor.look_files_tokens', 'Tokens', '`arbor look files`: the 100-file listing, names only', 422, 'tok',
      'config', 'suite', **ARBOR_LOOK_REF,
      why=lambda r: f"{r.ctx('arbor.look_files_tokens', 'dir')} holds other names than at the reference; `ls -la` of "
                    f"them: {r.ctx('arbor.look_files_tokens', 'ls_la')} tokens"),
    E('arbor.look_log_tokens', 'Tokens', '`arbor look log`: the 20 commits of `git log -20`', 579, 'tok', 'input',
      'suite', **ARBOR_LOOK_REF,
      why=lambda r: f"another repository or commit; `git log -20` of them: {r.ctx('arbor.look_log_tokens', 'git_log')} "
                    'tokens'),
    E('arbor.look_procs_tokens', 'Tokens', '`arbor look procs`: the 30 using the most memory', 325, 'tok', 'state',
      'suite', **ARBOR_LOOK_REF,
      why=lambda r: f"the names of whatever runs now; `ps aux` of the same 30: "
                    f"{r.ctx('arbor.look_procs_tokens', 'ps_aux')} tokens"),
    E('arbor.look_status_tokens', 'Tokens', '`arbor look status`: 15 changes in a repository', 128, 'tok', 'input',
      'suite', **ARBOR_LOOK_REF, why=lambda r: '`arbor look status` lists the same 15 changes differently'),
    E('arbor.git_status_tokens', 'Tokens', '`git status` of the same 15 changes', 244, 'tok', 'input', 'suite',
      **ARBOR_LOOK_REF, why=lambda r: "this git version's `git status` words the same changes differently"),
    E('arbor.look_about_tokens', 'Tokens', '`arbor look` alone: all it answers, described', 413, 'tok', 'input',
      'suite', **ARBOR_OUTLINE_REF, why=lambda r: 'the binary measured describes `arbor look` in other words'),
    E('arbor.look_ms', 'Prototype', 'one `arbor look` call: status, log and files', 2.70, 'ms', 'perf', 'suite', dp=2,
      **ARBOR_LOOK_REF,
      why=lambda r: placement(r, f"`git status`, `git log -20` and `ls -la` took "
                                 f"{r.ctx('arbor.look_ms', 'usual_ms'):.2f} ms one after another")),
    E('arbor.outline_pct', 'Tokens', "a large code file's outline, `arbor look outline`: share of the file's tokens",
      4.2, '%', 'input', 'suite', **ARBOR_OUTLINE_REF, why=parts),
    E('arbor.outline_read_pct', 'Tokens', "that outline and two of its functions, `arbor look lines`: share of the "
      "file's tokens", 8.48, '%', 'input', 'suite', **ARBOR_OUTLINE_REF, why=parts),

    # 9. The screen
    E('screen.still_cpu_pct', 'Screen', 'still screen, no input: compositor CPU, share of one core', 2.1, '%', 'state',
      'opt', why=lambda r: still_why(r, 'screen.still_cpu_pct')),
    E('screen.still_wakeups_s', 'Screen', 'still screen, no input: compositor wakeups', 130, '/s', 'state', 'suite',
      dp=0, **SCREEN_REF, why=lambda r: still_why(r, 'screen.still_wakeups_s')),
    E('screen.missed_idle_pct', 'Screen', 'test window: frames missed, machine idle', 0.13, '%', 'state', 'suite',
      **MISSED, **SCREEN_REF, why=lambda r: frames_why(r, 'screen.missed_idle_pct')),
    E('screen.frame_p99_idle_ms', 'Screen', 'test window: slowest 1% of frame times, machine idle', 16.7, 'ms', 'state',
      'suite', dp=1, **SCREEN_REF, why=lambda r: frames_why(r, 'screen.frame_p99_idle_ms')),
    E('screen.missed_load_pct', 'Screen', 'test window: frames missed, a busy cell per CPU thread and a memory spike',
      8.35, '%', 'state', 'suite', **MISSED, **SCREEN_REF, why=lambda r: frames_why(r, 'screen.missed_load_pct')),
    E('screen.frame_p99_load_ms', 'Screen', 'test window: slowest 1% of frame times, under that load', 33.4, 'ms',
      'state', 'suite', dp=1, **SCREEN_REF, why=lambda r: frames_why(r, 'screen.frame_p99_load_ms')),
]

# 10. The toolkits of issue #8 (sections/toolkits.py, run only with ONLY=toolkits): the same test program in each
# candidate toolkit, measured the same way. The reference is the suite's own (docs/INTERFACE.md, The toolkits
# measured): each row's median over the clean samples of five `ONLY=toolkits` runs on 17 Oct 2026, from an ordinary
# context (a systemd --user unit), 3 or 4 a row. A sample is clean when its rows passed the section's checks and began
# at a load average under 1; memory and first frame count from the launches whose still screen was clean. The runs
# started at load averages 0.46, 0.52, 0.02 (ref_load: the run of 21:33, clean for every toolkit), 0.32 and 0.57.
# Under load, the missed frames ranged from 1.3% to 10.8% for GTK 4 over its three samples, so that row moves well
# beyond ±15% from run to run, as the screen section's does.
# gtk4rs, the C program written again through gtk4-rs (#9), has its reference from the `ONLY="toolkits mission"` run
# of GTK_RS_REF, its first, from an ordinary context, next to the C program measured again in that run: one sample a
# row, the first frame a median of 5 launches.
TOOLKITS = (('gtk4', 'GTK 4'), ('gtk4rs', 'GTK 4, gtk4-rs'), ('qtquick', 'Qt Quick'), ('slint', 'Slint'),
            ('iced', 'iced'), ('flutter', 'Flutter'))
TOOLKITS_REF = {'ref_date': '17 Oct 2026', 'ref_load': 0.02}
GTK_RS_REF = {'ref_date': '18 Oct 2026', 'ref_load': 0.4}
TOOLKIT_M = {  # in the order of TOOLKIT_ROWS
    'gtk4': (170, 139.9, 117.5, 0.0, 0.1, 162.7, 139.9, 0.25, 16.7, 0.97, 2.17, 33.3, 127),
    'gtk4rs': (159, 140.9, 60.7, 0.02, 0.1, 140.9, 60.6, 0.33, 16.7, 0.83, 10.26, 33.3, 127),
    'qtquick': (147, 162.0, 138.1, 0.0, 0.05, 165.4, 141.5, 0.25, 16.7, 0.66, 11.43, 33.3, 109),
    'slint': (93, 93.8, 80.1, 0.02, 2.0, 93.8, 80.1, 0.33, 16.7, 0.92, 83.33, 133.3, 69),
    'iced': (99, 95.1, 83.9, 0.02, 4.0, 95.2, 84.0, 0.25, 16.7, 1.64, 16.35, 33.3, 0),
    'flutter': (199, 174.1, 139.3, 0.0, 0.3, 154.6, 131.6, 0.25, 16.7, 2.59, 4.25, 33.3, 42),
}
STILL = {'abs_tol': 0.05, 'abs_why': 'within 3 clock ticks (10 ms each) of the reference over the 60 s window'}
TOOLKIT_ROWS = (
    ('first_frame_ms', 'from its start to its first frame on screen', 'ms', {'dp': 0}),
    ('rss_start_mib', 'memory 2.5 s after its start: RSS', 'MiB', {}),
    ('pss_start_mib', 'memory 2.5 s after its start: PSS', 'MiB', {}),
    ('still_cpu_pct', 'still screen, no input: CPU, share of one core', '%', dict(STILL, dp=2)),
    ('still_wakeups_s', 'still screen, no input: wakeups', '/s',
     {'dp': 2, 'abs_tol': 0.1, 'abs_why': 'within 6 wakeups a minute of the reference'}),
    ('rss_idle_mib', 'memory after a minute of still screen: RSS', 'MiB', {}),
    ('pss_idle_mib', 'memory after a minute of still screen: PSS', 'MiB', {}),
    ('missed_idle_pct', 'animation: frames missed, machine idle', '%', MISSED),
    ('frame_p99_idle_ms', 'animation: slowest 1% of frame times, machine idle', 'ms', {'dp': 1}),
    ('frame_work_p99_ms', 'animation: work per frame (CPU time), slowest 1%', 'ms', {'dp': 2}),
    ('missed_load_pct', 'animation: frames missed, a busy cell per CPU thread and a memory spike', '%', MISSED),
    ('frame_p99_load_ms', 'animation: slowest 1% of frame times, under that load', 'ms', {'dp': 1}),
    ('atspi_objects', 'objects on the accessibility bus (AT-SPI)', '', {'abs_tol': 0, 'abs_why': ''}),
)


def toolkit_why(r, key):
    return toolkit_note((r.recs.get(key) or {}).get('ctx') or {}, key.rsplit('.', 1)[1])


ENTRIES += [E(f'ui.{t}.{metric}', 'Toolkits', f'{name}: {label}', TOOLKIT_M[t][i], unit, 'state', 'suite', **kw,
              **(GTK_RS_REF if t == 'gtk4rs' else TOOLKITS_REF), why=lambda r, k=f'ui.{t}.{metric}': toolkit_why(r, k))
            for t, name in TOOLKITS for i, (metric, label, unit, kw) in enumerate(TOOLKIT_ROWS)]

# 11. Mission Control, the first app of #9 (sections/mission.py, run only with ONLY=mission), measured as the toolkits
# are, in two states of arbor made by the section: `mission`, four tries and two decisions with nothing running, also
# drawn by GTK's cairo renderer (`mission_cairo`), and `mission_busy`, the same with 300 more tries, a sleeping room in
# each, while the ledger moves. The reference is the suite's own, from the runs of MISSION_REF (docs/INTERFACE.md, The
# first app, measured), from an ordinary context: one sample a row, the first frame a median of 5 launches. The first
# two states are from #9's run at 13:02, whose cairo rows began while the load average still fell from the loaded run
# before them (7.0 at the first launch), with no other program using a CPU thread; the section has waited for the load
# to settle there since. `mission_busy` is from #78's run at 14:17, once the window read only what new steps name.
MISSION = (('mission', 'Mission Control, nothing running'), ('mission_cairo', 'Mission Control, cairo renderer'),
           ('mission_busy', 'Mission Control, 300 rooms'))
MISSION_REF = {'mission': {'ref_date': '18 Oct 2026', 'ref_load': 0.15},
               'mission_cairo': {'ref_date': '18 Oct 2026', 'ref_load': 0.15},
               'mission_busy': {'ref_date': '18 Oct 2026', 'ref_load': 0.31}}
MISSION_ROWS = TOOLKIT_ROWS + (('still_frames_s', 'still screen, no input: frames shown', '/s',
                                {'dp': 2, 'abs_tol': 0.05, 'abs_why': 'within 3 frames a minute of the reference'}),)
MISSION_M = {  # in the order of MISSION_ROWS
    'mission': (174, 140.6, 60.1, 0.0, 0.1, 140.6, 60.3, 0.42, 16.7, 0.58, 15.1, 33.3, 173, 0.0),
    'mission_cairo': (126, 53.9, 23.7, 0.0, 0.03, 53.9, 23.7, 0.42, 16.7, 0.73, 13.09, 33.3, 173, 0.0),
    'mission_busy': (200, 160.9, 83.5, 0.33, 12.5, 161.8, 84.3, 0.17, 16.7, 8.98, 4.92, 33.4, 371, 1.0),
}
ENTRIES += [E(f'ui.{t}.{metric}', 'Mission Control', f'{name}: {label}', MISSION_M[t][i], unit, 'state', 'suite',
              **kw, **MISSION_REF[t], why=lambda r, k=f'ui.{t}.{metric}': toolkit_why(r, k))
            for t, name in MISSION for i, (metric, label, unit, kw) in enumerate(MISSION_ROWS)]
# Its buttons, pressed by the window itself in two of those states (`arbor-mission --bench`, sections/mission.py): each
# row is the median of its presses, from the press (the button's clicked signal) to the presentation time of the frame
# that shows its result, from the run of MISSION_ACT_REF on the build with #82, #83 and #84, from an ordinary context.
# A result shows on a refresh, so a press moves by up to a frame (16.7 ms at 60 Hz) from one run to the next.
# That run's still-screen rows with 300 rooms failed (another window was drawing), so MISSION_M keeps #78's for them.
MISSION_ACT_REF = {'mission': {'ref_date': '18 Oct 2026', 'ref_load': 0.28},
                   'mission_busy': {'ref_date': '18 Oct 2026', 'ref_load': 0.52}}
MISSION_ACTS = (  # metric, label, (nothing running, 300 rooms)
    ('confirm_ms', 'Release or Refuse: from the press to its confirmation on screen', (24, 25)),
    ('release_ms', 'Release now: from the press to the receipt of `arbor release` on screen', (39, 47)),
    ('refuse_ms', 'Refuse now: from the press to the receipt of `arbor refuse` on screen', (42, 45)),
    ('changes_ms', 'See changes: from the press to the files and their lines on screen', (39, 34)),
    ('keep_ms', 'Keep: from the press to "Kept" on screen, once `arbor keep` has run', (35, 49)),
    ('undo_ms', 'Undo the keep: from the press to its line on screen, once `arbor undo` has run', (39, 47)),
    ('start_ms', 'Start an agent: from the press to "Started" on screen, once its try and key are made', (45, 44)),
)


def press_why(r, key):
    ms, load = r.ctx(key, 'presses_ms'), r.ctx(key, 'load1')
    return f"presses of {', '.join(f'{x:.0f}' for x in ms or [])} ms; load average {load}" if ms else ''


ENTRIES += [E(f'ui.{t}.{metric}', 'Mission Control', f'{name}: {label}', ms[i], 'ms', 'perf', 'suite', dp=0,
              abs_tol=16.7, abs_why='within one refresh at 60 Hz of the reference', **MISSION_ACT_REF[t],
              why=lambda r, k=f'ui.{t}.{metric}': press_why(r, k))
            for i, (t, name) in enumerate((MISSION[0], MISSION[2])) for metric, label, ms in MISSION_ACTS]

# 12. The five goals of 18 Oct 2026 (sections/goals.py, run only with ONLY=goals, issue #85): today's Ubuntu against arbor
# over a whole stretch of work. The reference is the suite's own, from an ordinary context: the session rows are the
# medians of the 3 measured runs of each arm (after one warm-up) of the run at 20:54 (GOALS_SESSION_REF, GOALS_PARTS=session,
# issue #90), with the CPU under 1% busy before every run, but package energy, which comes with the power rows from the run
# at 19:59 (GOALS_REF): energy was read through RAPL, which the owner opened to ordinary users for that run only (#85 says
# why it is closed by default). An earlier run at 18:52 read within 10% of the 19:59 one on every row but the memory and
# swap ones and arbor's five tries (4.7 s there, 4.1 s here). The safety and ease rows are from that earlier run
# (GOALS_SAFE_REF), the still desktop from the run in GOALS_STILL_REF. Arbor's wall time swings between runs on the same
# CPU time (4.9, 12.0 and 12.8 s): a build in a world links, renames and rewrites files of the folder below, and overlayfs
# copies each one up and fsyncs it, about 1,640 disk cache flushes in a run against the copies' 30 (#90). They took 1.4 s
# in the fast run and 5.2-5.5 s in the slow ones; every arbor run follows the copies' 4.4 GiB of writes. The rows' ctx has them.
GOALS_REF = {'ref_date': '18 Oct 2026', 'ref_load': 3.5}
GOALS_SESSION_REF = {'ref_date': '18 Oct 2026', 'ref_load': 0.57}
GOALS_SAFE_REF = {'ref_date': '18 Oct 2026', 'ref_load': 3.3}
# The still desktop's run started at a load average of 5.84, right after the mission section's, and its blocks at 0.41-0.53.
# Someone used the PC during it: of the two blocks of today, the first saw input in all three attempts, so today has one
# clean block of 150 s, and arbor two. `arbor serve` was already running when it started, so it ran in both arms: the
# arms differ by Mission Control, open on the section's state.
GOALS_STILL_REF = {'ref_date': '18 Oct 2026', 'ref_load': 5.84}
GOAL_ARMS = (('worktree', 'today, git worktrees'), ('copy', 'today, copies'), ('arbor', 'arbor, worlds'))
SESSION_ROWS = (  # metric, label, unit, kind, extra
    ('mem_peak_mib', 'memory and swap in use: peak above the start', 'MiB', 'state', {'tol': 0.3}),
    ('mem_mean_mib', 'memory and swap in use: mean above the start', 'MiB', 'state', {'tol': 1.0}),
    ('swap_mib', 'swapped out (zram and disk)', 'MiB', 'state',
     {'tol': 1.0, 'abs_tol': 64, 'abs_why': 'within 64 MiB of the reference: swap follows what else holds memory'}),
    ('disk_mib', 'written to disk', 'MiB', 'state', {}),
    ('taken_mib', 'disk taken by the three tries', 'MiB', 'state', {}),
    ('wall_s', 'wall time', 's', 'perf', {}),
    ('cpu_s', 'CPU time, all cores', 's', 'perf', {}),
    ('energy_j', 'package energy (RAPL)', 'J', 'perf', {}),
)
SESSION_M = {  # in the order of SESSION_ROWS
    'worktree': (4402.6, 1695.7, 8.1, 3312.6, 3314.6, 32.34, 267.8, 4235.6),
    'copy': (1179.8, 364.3, 0.0, 4443.4, 3358.6, 8.61, 45.02, 936.6),
    'arbor': (490.2, 107.6, 0.0, 2209.1, 1296.6, 11.99, 17.18, 647.8),
}
ENTRIES += [E(f'goal.session.{a}.{metric}', 'Goals', f'session, {name}: {label}', SESSION_M[a][i], unit, kind, 'suite',
              **kw, **(GOALS_REF if metric == 'energy_j' else GOALS_SESSION_REF), blocks_label='runs')
            for a, name in GOAL_ARMS for i, (metric, label, unit, kind, kw) in enumerate(SESSION_ROWS)]
COUNT = {'abs_tol': 0, 'abs_why': ''}
ENTRIES += [E(f'goal.power.{a}.tries5_s', 'Goals', f'power, {name}: 5 tries of a change at once, keep 1', m, 's',
              'perf', 'suite', **GOALS_REF, blocks_label='runs')
            for (a, name), m in zip(GOAL_ARMS, (6.60, 7.94, 4.12))]
ENTRIES += [E(f'goal.power.{a}.{metric}', 'Goals', f'power, {name}: {label}', m, unit, kind, 'suite', **kw, **GOALS_REF)
            for a, name, ms in (('worktree', 'today, git worktrees', (3, 1, 4, 2.32)),
                                ('arbor', 'arbor, worlds', (4, 0, 5, 7.65)))
            for (metric, label, unit, kind, kw), m in zip((
                ('kept', 'undo test: kinds of change in the folder the keep brought (of 4)', '', 'input', COUNT),
                ('escaped', 'undo test: changes that reached outside the try unkept', '', 'input', COUNT),
                ('put_back', 'undo test: places as before once the try is undone (of 5)', '', 'input', COUNT),
                ('undo_ms', 'undo test: the undo, timed (one run)', 'ms', 'perf', {'tol': 0.5})), ms)]
ENTRIES += [E(f'goal.safety.{a}.reached', 'Goals', f'safety, {name}: probes that reached (of 13)', m, '', 'input',
              'suite', **COUNT, **GOALS_SAFE_REF)
            for a, name, m in (('today', 'as you', 13), ('arbor', 'in a cell under an agent key', 0))]
ENTRIES += [E(key, 'Goals', f'ease: {label}', m, '', 'input', 'suite', **COUNT, **GOALS_SAFE_REF) for key, label, m in (
    ('goal.ease.start.today', 'start an agent safely, today', 'no stock way'),
    ('goal.ease.start.arbor', 'start an agent safely, arbor: commands', 3),
    ('goal.ease.approve.today', 'approve one effect, today', 'nothing waits'),
    ('goal.ease.approve.arbor', 'approve one effect, arbor: commands', 2),
    ('goal.ease.approve.mission', 'approve one effect, Mission Control', 'cannot act yet'),
    ('goal.ease.keep.worktree', 'keep a try, git worktrees: commands', 3),
    ('goal.ease.keep.copy', 'keep a try, copies: commands', 3),
    ('goal.ease.keep.arbor', 'keep a try, arbor: commands', 1),
    ('goal.ease.undo.worktree', 'undo a kept try, git worktrees: commands', 1),
    ('goal.ease.undo.arbor', 'undo a kept try, arbor: commands', 1),
    ('goal.ease.discard4.worktree', 'throw 4 tries away, git worktrees: commands', 8),
    ('goal.ease.discard4.copy', 'throw 4 tries away, copies: commands', 4),
    ('goal.ease.discard4.arbor', 'throw 4 tries away, arbor: commands', 4))]
STILL_ROWS = (('wakeups_s', 'interrupts, all CPUs', '/s', {'dp': 0}), ('busy_pct', 'CPU busy, all cores', '%', {'dp': 2}),
              ('deep_pct', 'time in the deepest idle state', '%', {'dp': 1}),
              ('power_w', 'package power (RAPL)', 'W', {'dp': 2}))
STILL_M = {'today': (2985, 0.50, 94.1, 22.29), 'arbor': (2895, 0.48, 94.4, 22.06)}  # in the order of STILL_ROWS
ENTRIES += [E(f'goal.still.{a}.{metric}', 'Goals', f'still desktop, {name}: {label}', STILL_M[a][i], unit, 'state',
              'suite', **kw, **GOALS_STILL_REF, blocks_label='blocks')
            for a, name in (('today', 'today'), ('arbor', 'with arbor serve and Mission Control'))
            for i, (metric, label, unit, kw) in enumerate(STILL_ROWS)]
