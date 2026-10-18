"""The report of a whole-session A/B (ab.py): per task and arm, the medians and spread of what the sessions spent, and
what each arm changed against arm A, and each later arm against an earlier one, with its uncertainty.

usage: python3 bench/sessions/summary.py RESULTS.jsonl   (prints Markdown)

Only each slot's last final attempt counts (ab.py runs a session again after an error of the provider's, and
`--resume` runs a slot again whose every attempt the provider ended). Every session the agent finished counts, failed
or not, since a failed session was paid for too; one that the provider ended is left out, and named. The weighted
cost is the suite's (bench/sections/transcripts.js): input 1, cache write 1.25, cache read 0.1, output 5, where output
is what OpenCode records as output plus reasoning, which it keeps apart and the provider bills as output.

B/A is the ratio of two arms' medians for one task. Its 95% interval is a bootstrap: 4,000 times, each arm's sessions
of the task are drawn again with replacement, and the 2.5th and 97.5th percentiles of the ratio are kept. All tasks is
the geometric mean of the tasks' ratios, bootstrapped the same way, task by task. P(B<A) is the share of pairs of one
A and one B session of the task in which B spent less (ties count half). Tool text is the characters the tools
returned to the agent in a session. The reference row, for comparing a new run with an earlier one, is the arm's
ratios against A in the run that SESSIONS_REF in lib/reference.py records for it.
"""
import json
import math
import os
import random
import statistics
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'lib'))
import reference  # noqa: E402

DRAWS = 4000


def weighted(r):
    return r.get('input', 0) + 1.25 * r.get('cache_write', 0) + 0.1 * r.get('cache_read', 0) + 5 * (
        r.get('output', 0) + r.get('reasoning', 0))


METRICS = [  # (label, value of a record)
    ('weighted', weighted),
    ('steps', lambda r: r.get('steps', 0)),
    ('input', lambda r: r.get('input', 0)),
    ('cache read', lambda r: r.get('cache_read', 0)),
    ('output', lambda r: r.get('output', 0) + r.get('reasoning', 0)),
    ('last context', lambda r: r.get('context_last', 0)),
    ('tool text', lambda r: sum((r.get('tool_chars') or {}).values())),
    ('wall s', lambda r: r.get('wall_s', 0)),
]
RATIOS = ('weighted', 'steps', 'input', 'output', 'tool text')  # the columns of B against A


def provider_error(rec):
    """A session OpenCode ended because of the provider, not the agent: an error and no answer."""
    return not rec.get('timed_out') and (rec.get('outcome') not in (None, 'succeeded') or
                                         bool(rec.get('errors') and not rec.get('answer')))


def load(path):
    """(meta, end, the sessions that count, the slots the provider ended): each slot's last final attempt."""
    meta, end, slots, counted = {}, {}, {}, False
    for line in open(path):
        r = json.loads(line)
        if r.get('meta'):
            meta = r
        elif r.get('meta_resume'):
            meta.setdefault('resumed', []).append(r)
            counted = False
        elif r.get('meta_end'):
            if counted or not end:  # the end of the last part that added a session
                end = r
        elif r.get('final'):
            slots[r['rep'], r['task'], r['arm']] = r
            counted = counted or not provider_error(r)
            if meta.get('resumed'):
                meta['resumed'][-1]['added'] = meta['resumed'][-1].get('added', 0) + (not provider_error(r))
    recs = sorted(slots.values(), key=lambda r: r['n'])
    return meta, end, [r for r in recs if not provider_error(r)], [r for r in recs if provider_error(r)]


def load_avg(m):
    load = m.get('load')
    return '?' if load is None else f"{load[0] if isinstance(load, list) else load:.2f}"


def ratio(a, b):
    ma, mb = statistics.median(a), statistics.median(b)
    return mb / ma if ma else math.nan


def boot(groups, rng):
    """groups: [(A values, B values)] per task -> (geometric mean of the B/A ratios, its 95% interval)."""
    def gm(rs):
        rs = [r for r in rs if r > 0 and math.isfinite(r)]
        return math.exp(sum(map(math.log, rs)) / len(rs)) if rs else math.nan
    point = gm([ratio(a, b) for a, b in groups])
    draws = sorted(gm([ratio(rng.choices(a, k=len(a)), rng.choices(b, k=len(b))) for a, b in groups])
                   for _ in range(DRAWS))
    return point, draws[int(0.025 * DRAWS)], draws[int(0.975 * DRAWS) - 1]


def below(a, b):
    """P(B < A) over all pairs, ties half."""
    pairs = [(x, y) for x in a for y in b]
    return sum(1 if y < x else 0.5 if y == x else 0 for x, y in pairs) / len(pairs) if pairs else math.nan


def n(v):
    if isinstance(v, float) and not v.is_integer() and abs(v) < 100:
        return f'{v:,.1f}'
    return f'{round(v):,}'


def spread(vs):
    return f'{n(statistics.median(vs))} ({n(min(vs))}–{n(max(vs))})'


def report(path):
    meta, end, recs, lost = load(path)
    tasks = [t for t in meta.get('tasks', []) if any(r['task'] == t for r in recs)]
    arms = meta.get('arms', ['A', 'B'])
    rng = random.Random(1)
    built = meta.get('arbor') or '?'
    ran = {(r['rep'], r['task'], r['arm']) for r in recs + lost}
    unrun = {}  # (round, task): the arms of its slots that never ran, as when a run is recorded before its end
    for rep in range(meta.get('reps') or 0):
        for t in meta.get('tasks', []):
            for arm in arms:
                if (rep, t, arm) not in ran:
                    unrun.setdefault((rep, t), []).append(arm)
    if end:
        ended = f"{load_avg(end)} at the end ({end.get('date', '?')[11:16]})"
    else:  # recorded before its end: the last session that counts
        last = max(recs, key=lambda r: r['n'], default={})
        ended = f"{load_avg(last)} at the last session that counts ({last.get('started', '?')[11:16]})"
    out = [f"# arbor look over whole sessions: {meta.get('date', '?')[:10]}", '',
           f"{meta.get('opencode', 'OpenCode ?')} on `{meta.get('model')}`, tasks on this repository at "
           f"{meta.get('commit', '?')[:7]}, arbor "
           f"{'from ARBOR_BIN' if built == 'ARBOR_BIN' else 'built at ' + built[:7]}, {len(recs)} sessions "
           f"({meta.get('reps')} per task and arm{' planned, not all of which ran' if unrun else ''}). Load average "
           f"{load_avg(meta)} at the start ({meta.get('date', '?')[11:16]}), {ended}. A free "
           "remote model's answers vary from run to run, and the model behind the name can change, so a new run will "
           "not repeat these numbers exactly." + ''.join(
               f" The run stopped after session {r['after']} and went on at {r['date'][11:16]} (load average "
               f"{load_avg(r)}) with `--resume`" + ('.' if r.get('added') else ', which added no session.')
               for r in meta.get('resumed', [])), '',
           'Median (min–max) per session. Weighted: input 1, cache write 1.25, cache read 0.1, output 5.', '',
           '| task | arm | ok | ' + ' | '.join(m for m, _ in METRICS) + ' | arbor look | read |',
           '| :--- | :-: | :-: |' + ' ---: |' * (len(METRICS) + 2)]
    for t in tasks:
        for arm in arms:
            rs = [r for r in recs if r['task'] == t and r['arm'] == arm]
            if not rs:
                continue
            cells = [spread([f(r) for r in rs]) for _, f in METRICS]
            look = spread([r.get('arbor_look', 0) for r in rs])
            reads = spread([(r.get('tools') or {}).get('read', 0) for r in rs])
            ok = f"{sum(bool(r.get('success')) for r in rs)}/{len(rs)}"
            out.append(f'| {t} | {arm} | {ok} | ' + ' | '.join(cells) + f' | {look} | {reads} |')
    base = 'A' if 'A' in arms else arms[0]
    others = [x for x in arms if x != base]
    pairs = [(base, x) for x in others] + [(x, y) for i, x in enumerate(others) for y in others[i + 1:]]
    for a_arm, b_arm in pairs:
        out += ['', f'{b_arm} against {a_arm}: the ratio of medians, its 95% bootstrap interval, and '
                f'P({b_arm}<{a_arm}).', '', '| task | ' + ' | '.join(RATIOS) + f' | ok {a_arm} | ok {b_arm} |',
                '| :--- |' + ' ---: |' * (len(RATIOS) + 2)]
        groups = {m: [] for m in RATIOS}
        for t in tasks:
            ra = [r for r in recs if r['task'] == t and r['arm'] == a_arm]
            rb = [r for r in recs if r['task'] == t and r['arm'] == b_arm]
            if not ra or not rb:
                continue
            cells = []
            for m, f in METRICS:
                if m in RATIOS:
                    a, b = [f(r) for r in ra], [f(r) for r in rb]
                    groups[m].append((a, b))
                    p, lo, hi = boot([(a, b)], rng)
                    cells.append(f'{p:.2f} ({lo:.2f}–{hi:.2f}), {below(a, b):.2f}')
            out.append(f'| {t} | ' + ' | '.join(cells) + ' | ' + ' | '.join(
                f"{sum(bool(r.get('success')) for r in rs)}/{len(rs)}" for rs in (ra, rb)) + ' |')
        cells = ['{:.2f} ({:.2f}–{:.2f})'.format(*boot(groups[m], rng)) for m, _ in METRICS if m in RATIOS]
        out.append('| all tasks | ' + ' | '.join(cells) + ' | ' + ' | '.join(
            f"{sum(bool(r.get('success')) for r in recs if r['arm'] == x)}/{sum(r['arm'] == x for r in recs)}"
            for x in (a_arm, b_arm)) + ' |')
        ref = getattr(reference, 'SESSIONS_REF', {}).get(b_arm) if a_arm == 'A' else None
        if ref:
            out.append(f"| *reference, {ref['ref_date']}* | " + ' | '.join(
                '{:.2f} ({:.2f}–{:.2f})'.format(*ref[m]) for m in RATIOS) + f" | {ref['ok'][0]} | {ref['ok'][1]} |")
    tools = sorted({k for r in recs for k in (r.get('tools') or {})})
    if tools:
        out += ['', 'Tool calls and the characters they returned, median per session.', '',
                '| task | arm | ' + ' | '.join(tools) + ' |', '| :--- | :-: |' + ' ---: |' * len(tools)]
        for t in tasks:
            for arm in arms:
                rs = [r for r in recs if r['task'] == t and r['arm'] == arm]
                if rs:
                    out.append(f'| {t} | {arm} | ' + ' | '.join(
                        f"{n(statistics.median((r.get('tools') or {}).get(k, 0) for r in rs))} / "
                        f"{n(statistics.median((r.get('tool_chars') or {}).get(k, 0) for r in rs))}"
                        for k in tools) + ' |')
    retried = sum(1 for line in open(path) if '"final": false' in line)
    timed = sum(1 for r in recs if r.get('timed_out'))
    out += ['', f'{retried} attempts ended with an error of the provider and ran again; {timed} sessions ran out of '
            f"time ({meta.get('timeout_s', '?')} s) and count as failed."]
    if lost:
        out.append(f"{len(lost)} {'session is' if len(lost) == 1 else 'sessions are'} left out: the provider ended "
                   "every attempt ("
                   + ', '.join(f"{r['task']} {r['arm']} round {r['rep']}" for r in lost) + ').')
    if unrun:
        count = sum(map(len, unrun.values()))
        out.append(f"{count} {'session' if count == 1 else 'sessions'} of the plan never ran ("
                   + ', '.join(f"{t} {''.join(a)} round {rep}" for (rep, t), a in unrun.items()) + ').')
    return '\n'.join(out) + '\n'


def arbor_commit(root):
    import subprocess
    r = subprocess.run(['git', '-C', root, 'rev-parse', 'HEAD'], capture_output=True, text=True)
    dirty = subprocess.run(['git', '-C', root, 'status', '--porcelain', '--', 'prototype'], capture_output=True,
                           text=True).stdout.strip()
    return r.stdout.strip() + (' (prototype/ changed)' if dirty else '')


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit(__doc__.split('\n\n')[1])
    sys.stdout.write(report(sys.argv[1]))
