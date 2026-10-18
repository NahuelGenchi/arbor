"""Join the section results with the M values and write one Markdown table, stamped with date and machine.

usage: report.py RESULTS.jsonl META.json OUT.md
Also writes OUT.json (every row with its M value, now value, delta and note) for later before/after diffs.
"""
import datetime
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from reference import ENTRIES, M_INFO, M_PRESSURE, PSS_WHY, TOL, TOOLKITS, toolkit_note  # noqa: E402

# Which section measures a key (run.sh's ONLY/SKIP leave sections out; their rows are left out of the table).
SECTION_OF = (('rt.', 'runtimes'), ('cell.py_', 'pycell'), ('cell.', 'cells'), ('mem.', 'memstate'),
              ('ram.', 'memstate'), ('heap.', 'heap'), ('search.', 'search'), ('code.', 'search'), ('boot.', 'boot'),
              ('storage.', 'storage'), ('tok.', 'transcripts'), ('fmt.', 'formats'), ('arbor.', 'arbor'),
              ('screen.', 'screen'), ('ui.mission', 'mission'), ('ui.', 'toolkits'))
DP = {'%': 1, 'MiB': 1, 'GiB': 1, 'MB': 1, 's': 1, '×': 2, 'Mtok': 2, 'B': 0, 'tok': 0, '': 0}


def fmt(v, unit, dp=None):
    if v is None:
        return '—'
    if isinstance(v, str):
        return v
    if dp is None:
        a = abs(v)
        if unit == 'ms':
            dp = 2 if a < 10 else 1 if a < 100 else 0
        elif unit == 'µs':
            dp = 2 if a < 1 else 1
        else:
            dp = DP.get(unit, 1)
    s = f'{v:,.{dp}f}'
    if unit in ('%', '×'):
        return s + unit
    return f'{s} {unit}' if unit else s


def load(path):
    recs = {}
    for line in open(path):
        line = line.strip()
        if not line.startswith('{'):
            continue
        try:
            r = json.loads(line)
        except ValueError:
            continue
        old = recs.get(r['key'])
        if old is None or old.get('value') is None or r.get('value') is not None:
            recs[r['key']] = r  # a measured value wins over an error record for the same key
    return recs


class Rows:
    """What a reference entry's why(r) can ask for."""

    def __init__(self, recs, meta):
        self.recs, self.meta = recs, meta
        self.ref = {e['key']: e for e in ENTRIES}

    def v(self, key):
        return (self.recs.get(key) or {}).get('value')

    def m(self, key):
        return self.ref[key]['m'] if key in self.ref else None

    def ctx(self, key, field, default=None):
        return ((self.recs.get(key) or {}).get('ctx') or {}).get(field, default)

    def corpus(self, key, noun, fmtv='{:,.0f}'):
        now, m = self.v(key), self.m(key)
        if now is None or not m:
            return f'{noun}: no comparison'
        return f'{fmtv.format(now)} {noun} now vs {fmtv.format(m)} at M ({now / m - 1:+.0%})'

    def per(self, key, by, label, scale=1.0):
        a, b, ma, mb = self.v(key), self.v(by), self.m(key), self.m(by)
        if not all((a, b, ma, mb)):
            return ''
        now, then = a / b * scale, ma / mb * scale
        return f'{now:.2f} {label} vs {then:.2f} at M ({now / then - 1:+.0%})'

    def disk_note(self, key, who, min_mib):
        """'; <who> read N MiB from disk ...' when a timed run read at least min_mib from storage, else ''."""
        d = self.ctx(key, 'disk_mib')
        worst = max(d) if isinstance(d, list) and d else d
        if not worst or worst < min_mib:
            return ''
        return (f"; {who} read {worst:,.0f} MiB from disk in a timed run: the files were not in the page cache "
                f"({self.meta.get('mem_available_gib', '?')} GiB RAM available at the start)")

    def code_why(self, key):
        q, ref = self.ctx(key, 'query'), self.ref[key]
        if q != ref.get('query'):
            return f"different query than M (`{q}` vs `{ref.get('query')}`)"
        cold = self.disk_note(key, 'rg', 16) if '.rg_' in key else ''
        note = f"{self.ctx(key, 'files')} files match now vs {ref.get('m_files')} at M" + cold
        if ref.get('m_cold') and (self.v(key) or 0) < ref['m'] and not cold:
            note += ("; M's first query most likely read part of the corpus from disk (0.9 GiB RAM was available; "
                     "its other four took 37–40 ms); here each query gets an untimed warm-up run first")
        return note


def perf_note(rows, e, rec):
    parts = []
    suite = e['src'] == 'suite'
    blocks = (rec.get('ctx') or {}).get('blocks')
    if blocks and isinstance(e['m'], (int, float)):
        lo, hi = min(blocks), max(blocks)
        rng = f'{fmt(lo, e["unit"], e.get("dp"))}–{fmt(hi, e["unit"], e.get("dp"))}'
        rng = f'{e.get("blocks_label", "block medians")} {rng}'
        if lo * (1 - TOL) <= e['m'] <= hi * (1 + TOL):
            parts.append(f'{"the reference" if suite else "M"} is within run-to-run noise: {rng}')
        else:
            parts.append(rng)
    meta = rows.meta
    if meta.get('epp') and meta['epp'] != M_INFO['epp']:
        parts.append(f"CPU energy preference is {meta['epp']} (M: {M_INFO['epp']})")
    now, m = rec.get('value'), e['m']
    if isinstance(now, (int, float)) and isinstance(m, (int, float)) and now < m * (1 - TOL):
        # load here cannot make a run faster
        parts.append(f"faster than the reference run, which started at a load average of {e.get('ref_load')}" if suite
                     else M_PRESSURE)
    elif meta.get('load1') is not None:
        parts.append(f"load average {meta['load1']} at start" +
                     (f" (busy: {meta['busy']} of a core)" if meta.get('busy') else ''))
    return '; '.join(parts)


def same_time(a, b, slack=5):
    """`uptime -s` is computed from the clock minus uptime, so it can move by a second between calls."""
    try:
        fa, fb = (datetime.datetime.strptime(x, '%Y-%m-%d %H:%M:%S') for x in (a, b))
        return abs((fa - fb).total_seconds()) <= slack
    except ValueError:
        return a == b


def explain(rows, e, rec):
    kind, ctx = e['kind'], rec.get('ctx') or {}
    specific = ''
    if e.get('why'):
        try:
            specific = e['why'](rows)
        except Exception as ex:  # a missing companion value must not break the report
            specific = f'(no detail: {ex})'
    if kind == 'perf':
        return '; '.join(p for p in (specific, perf_note(rows, e, rec)) if p)
    if kind == 'rss':
        return specific or 'resident pages of a fresh process follow the page cache'
    if kind == 'pss':
        n = ctx.get('same_program_procs')
        rss = e['key'].replace('.pss.', '.rss.')
        rss_off = rss != e['key'] and rows.v(rss) and rows.m(rss) and abs(rows.v(rss) / rows.m(rss) - 1) > TOL
        sharers = ('' if n is None else f'{n} other processes run the same program now' if n else
                   'no other process runs this program now, so PSS follows RSS (see the RSS row)' if rss_off else
                   "no other process runs this program now; the rest is its share of the libraries every process "
                   "maps (libc, ld.so), which moves with how many processes map them")
        return '; '.join(p for p in (PSS_WHY, specific, sharers) if p)
    if kind == 'boot':
        booted = ctx.get('booted') or rows.meta.get('booted')
        if booted and not same_time(booted, M_INFO['boot']):
            return f"different boot: this one started {booted}; M described the boot of {M_INFO['boot']}"
        if (ctx.get('last_run') or '') > M_INFO['blame_read']:
            return (f"same boot, but a timer ran the unit again at {ctx['last_run']}, after M read blame "
                    f"({M_INFO['blame_read']}); blame reports the latest run")
        return specific or 'same boot as M; systemd reported a different value'
    if kind == 'input':
        if e['key'].startswith('heap.'):
            mib, lines, node = ctx.get('source_mib'), ctx.get('lines'), ctx.get('node')
            src = f'source transcript {mib} MiB, {lines:,} lines' if lines else 'source transcript'
            if not (mib == 121 and lines == 5021):
                return f'{src} (M: 121 MiB, 5,021 lines): a different input than M'
            if node != M_INFO['node']:
                return f"same input; node {node} now vs {M_INFO['node']} at M: V8 versions size and place objects differently"
            return 'same input and node version as M; no known cause'
        if e['key'].startswith('fmt.git'):
            ref = ctx.get('ref') or ''
            return (f"repo {ctx.get('repo')} at {ref}" +
                    ('' if ref.startswith(M_INFO['git_ref'][:7]) else f" (M: {M_INFO['git_ref']}); a different starting commit"))
        return specific
    if kind == 'config':
        if e['key'].startswith('boot.initrd'):
            b = rows.ctx('boot.initrd_mb', 'bytes')
            if b and b != M_INFO['initrd_bytes']:
                return (f"the initramfs was rebuilt: {b:,} bytes now vs {M_INFO['initrd_bytes']:,} at M "
                        f"({rows.ctx('boot.initrd_mb', 'file')}); " + specific).rstrip('; ')
        if e['key'].startswith('fmt.'):
            return specific or f"{os.environ.get('LISTING_DIR', '/usr/bin')} changed since M (package updates change names, sizes and dates)"
        return specific or 'system configuration changed since M'
    if kind == 'corpus' and e['unit'] in ('ms', 's', 'µs'):  # timing over a data set: CPU load matters too
        return '; '.join(p for p in (specific or 'the data set changed since M', perf_note(rows, e, rec)) if p)
    return specific or {'corpus': 'the data set changed since M', 'state': 'live reading of the running system'}[kind]


def evaluate(rows, e, rec):
    """-> (status icon, delta text, note)"""
    if rec is None:
        return '❌', '', 'not measured (section did not run; see the run log)'
    now, m = rec.get('value'), e['m']
    if now is None:
        return '❌', '', 'failed: ' + (rec.get('err') or 'no value')
    if m is None:
        return '🆕', '', e.get('note') or 'no M value; first baseline'
    if isinstance(m, str) or isinstance(now, str):
        return ('✅', '', '') if str(now) == str(m) else ('⚠️', 'differs', explain(rows, e, rec))
    in_abs = 'abs_tol' in e and abs(now - m) <= e['abs_tol']
    if m == 0:
        delta, ok, note = f'{now - m:+.1f} {e["unit"]}'.strip(), in_abs, ''
    else:
        rel = now / m - 1
        delta = f'{rel:+.0%}' if abs(rel) >= 0.005 else '±0%'
        ok = abs(rel) <= e.get('tol', TOL) or in_abs
        note = '' if abs(rel) <= e.get('tol', TOL) else (
            e.get('abs_why') or f"within {e.get('abs_tol')} {e['unit']} of M, the precision M was printed with")
    return ('✅', delta, note) if ok else ('⚠️', delta, explain(rows, e, rec) or 'outside ±15%; no specific cause found')


def note_ctx(e, rec):
    """Short context shown on rows that pass, when it helps reading them."""
    ctx = rec.get('ctx') or {} if rec else {}
    k = e['key']
    if k in ('search.find_ms', 'search.rg_files_ms', 'search.catalog_ms'):
        return f"{ctx.get('hits')} hits"
    if k.startswith('code.') and 'files' in ctx and 'query' in ctx:
        return f"{ctx['files']} files" + (f", {ctx['speedup']}× faster than rg" if ctx.get('speedup') else '')
    if k == 'cell.py_fork_ms' and ctx.get('speedup'):
        return f"{ctx['speedup']}× faster than fresh"
    if k in ('storage.fork_ms', 'storage.fork_bwrap_ms'):
        return f"tree {ctx.get('tree')}; a write in the fork reached the lower tree: {ctx.get('write_reached_lower')}"
    if k == 'storage.fork_bwrap_overlay_ms' and ctx.get('sandbox_ms'):
        return f"the same sandbox without the overlay: {ctx['sandbox_ms']:.2f} ms"
    if k == 'heap.lzo' and ctx.get('roundtrip_checked'):
        return f"kernel uses lzo-rle (LZO1X-1 + zero runs); round trips checked {ctx['roundtrip_checked']}"
    if k.startswith('heap.') and ctx.get('mib_per_s'):
        return f"{ctx['mib_per_s']} MiB/s single-thread"
    if k == 'ram.claude_code_mib' and ctx.get('procs') is not None:
        return f"{ctx['procs']} sessions, {ctx.get('per_process_mib', '–')} MiB each"
    if k.startswith('ram.') and ctx.get('procs') is not None:
        return f"{ctx['procs']} processes"
    if k == 'storage.dedup_pct' and ctx.get('scan_s') is not None:
        return f"{ctx.get('hashed_files'):,} files hashed in {ctx['scan_s']} s"
    if k == 'fmt.git_log' and ctx.get('repo'):
        return f"{ctx['repo']} at {ctx.get('ref')}"
    if k == 'arbor.ask_ms' and ctx.get('run_ms'):
        return f"{ctx.get('with_ask_ms'):.2f} ms with the ask, {ctx['run_ms']:.2f} ms without"
    if k.startswith('arbor.serve_') and ctx.get('service'):
        return f"arbor serve {ctx['service']}"
    if k == 'screen.still_cpu_pct' and ctx.get('compositor'):
        return (f"{ctx['compositor']}; GPU time {ctx.get('gpu_ms')} ms; the rest of the machine used "
                f"{ctx.get('rest_cpu_pct')}% of a core")
    if k == 'screen.still_wakeups_s' and ctx.get('by_thread'):
        return f"by thread: {ctx['by_thread']}"
    if k.startswith(('screen.missed_', 'screen.frame_')) and ctx.get('frames'):
        load = f"; {ctx['cells']} cells, a {ctx['spike_mib']:,} MiB spike" if ctx.get('cells') is not None else ''
        return (f"{ctx['frames']:,} frames at {ctx.get('refresh_hz')} Hz, {ctx.get('missed')} missed, the slowest "
                f"{ctx.get('worst_ms')} ms; {ctx.get('to_screen_ms')} ms from commit to screen (median){load}")
    if k.startswith('ui.'):
        return toolkit_note(ctx, k.rsplit('.', 1)[1])
    return ''


def main(results, meta_path, out):
    recs = load(results)
    meta = json.load(open(meta_path)) if os.path.exists(meta_path) else {}
    meta['duration_s'] = os.environ.get('BENCH_DURATION_S', meta.get('duration_s', '?'))
    rows = Rows(recs, meta)
    table, data = [], []
    counts = {'✅': 0, '⚠️': 0, '🆕': 0, '❌': 0}
    known = {e['key'] for e in ENTRIES}
    suite_dates = set()
    extra = [dict(key=k, sec=k.split('.')[0], label=k, m=None, unit='', kind='perf', src='run')
             for k in recs if k not in known]
    skipped = os.environ.get('BENCH_SKIPPED', '').split()
    # The toolkits section measures only the toolkits TOOLKITS names: the others' rows are left out, like a section's.
    named = os.environ.get('TOOLKITS', '').split() or [t for t, _ in TOOLKITS]
    left_out = [t for t, _ in TOOLKITS if t not in named] if 'toolkits' not in skipped else []
    for e in ENTRIES + extra:
        rec = recs.get(e['key'])
        if rec is None and (next((s for p, s in SECTION_OF if e['key'].startswith(p)), None) in skipped
                            or e['key'].startswith('ui.') and e['key'].split('.')[1] in left_out):
            continue
        icon, delta, note = evaluate(rows, e, rec)
        if icon == '✅' and not note:
            note = note_ctx(e, rec)
        counts[icon] += 1
        now = rec.get('value') if rec else None
        mark = {'run': ' †', 'suite': ' ‡'}.get(e['src'], '') if e['m'] is not None else ''
        if mark == ' ‡':
            suite_dates.add(e['ref_date'])
        mtxt = fmt(e['m'], e['unit'], e.get('dp')) + mark
        cell = lambda s: str(s).replace('|', '\\|').replace('\n', ' ')  # noqa: E731
        table.append(f"| {e['sec']} | {cell(e['label'])} | {cell(mtxt)} | {cell(fmt(now, e['unit'], e.get('dp')))} | "
                     f"{delta} | {icon} | {cell(note)} |")
        data.append({'key': e['key'], 'section': e['sec'], 'metric': e['label'], 'unit': e['unit'], 'm': e['m'],
                     'm_source': e['src'], 'now': now, 'delta': delta, 'status': icon, 'note': note,
                     'ctx': (rec or {}).get('ctx')})
    m = meta
    head = [
        f"# Arbor baseline: {m.get('date', '?')} on {m.get('host', '?')}",
        '',
        f"**Machine:** {m.get('cpu', '?')} ({m.get('cores', '?')}), {m.get('ram_gib', '?')} GiB RAM, "
        f"{m.get('board', '?')} (BIOS {m.get('bios', '?')}), {m.get('os', '?')}, Linux {m.get('kernel', '?')}. "
        f"Booted {m.get('booted', '?')}.",
        f"**Run:** {m.get('date', '?')}, {m.get('duration_s', '?')} s; CPU governor {m.get('governor', '?')}, energy "
        f"preference {m.get('epp', '?')}; load average {m.get('load1', '?')} at start"
        f"{' (busy: ' + m['busy'] + ' of a core)' if m.get('busy') else ''}; {m.get('mem_available_gib', '?')} GiB "
        f"RAM available, {m.get('swap_used_gib', '?')} GiB swap in use at start. Tools: {m.get('tools', '?')}."
        + (f" Sections not run (ONLY/SKIP): {', '.join(skipped)}." if skipped else '')
        + (f" Toolkits not run (TOOLKITS): {', '.join(left_out)}." if left_out else ''),
        f"**Reference:** M = measured on this PC on {M_INFO['date']} for OPTIMIZATION.md; † = from that run's raw output, "
        "not quoted in OPTIMIZATION.md"
        + (f"; ‡ = not M: the suite's own reference, from its run of {', '.join(sorted(suite_dates))}"
           if suite_dates else '')
        + ". ✅ within ±15% of M (or within the precision M was printed with) · ⚠️ outside, "
        "cause in the note · 🆕 no M value · ❌ failed.",
        f"**Result:** {counts['✅']} within ±15%, {counts['⚠️']} outside with the cause noted, {counts['🆕']} new, "
        f"{counts['❌']} failed.",
        '',
        '| Area | Metric | M | Now | Δ | | Note |',
        '|---|---|--:|--:|--:|:-:|---|',
    ]
    params = m.get('params') or {}
    tail = ['', '**Parameters:** ' + '; '.join(f'{k}=`{v}`' for k, v in params.items())] if params else []
    with open(out, 'w') as f:
        f.write('\n'.join(head + table + tail) + '\n')
    with open(os.path.splitext(out)[0] + '.json', 'w') as f:
        json.dump({'meta': meta, 'm_info': M_INFO, 'rows': data}, f, indent=1, default=str)
    print(f"{counts['✅']} within ±15%, {counts['⚠️']} outside (explained), {counts['🆕']} new, {counts['❌']} failed")
    return counts


if __name__ == '__main__':
    main(*sys.argv[1:4])
