"""The report of a whole-session A/B (ab.py): per task and arm, the medians and spread of what the sessions spent, and
what arm B changed against arm A, with its uncertainty.

usage: python3 bench/sessions/summary.py RESULTS.jsonl   (prints Markdown)

Only each slot's final attempt counts (ab.py runs a session again after an error of the provider's). Every session
counts, failed or not, since a failed session was paid for too. The weighted cost is the suite's
(bench/sections/transcripts.js): input 1, cache write 1.25, cache read 0.1, output 5, where output is what OpenCode
records as output plus reasoning, which it keeps apart and the provider bills as output.

B/A is the ratio of the arms' medians for one task. Its 95% interval is a bootstrap: 4,000 times, each arm's sessions
of the task are drawn again with replacement, and the 2.5th and 97.5th percentiles of the ratio are kept. All tasks is
the geometric mean of the tasks' ratios, bootstrapped the same way, task by task. P(B<A) is the share of pairs of one
A and one B session of the task in which B spent less (ties count half). The reference row is the run that
SESSIONS_REF in lib/reference.py records, for comparing a new run with it.
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
    ('wall s', lambda r: r.get('wall_s', 0)),
]


def load(path):
    meta, end, recs = {}, {}, []
    for line in open(path):
        r = json.loads(line)
        if r.get('meta'):
            meta = r
        elif r.get('meta_resume'):
            meta.setdefault('resumed', []).append(r)
        elif r.get('meta_end'):
            end = r
        elif r.get('final'):
            recs.append(r)
    return meta, end, recs


def load_avg(m):
    return f"{m['load'][0]:.2f}" if m.get('load') else '?'


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
    meta, end, recs = load(path)
    tasks = [t for t in meta.get('tasks', []) if any(r['task'] == t for r in recs)]
    arms = meta.get('arms', ['A', 'B'])
    rng = random.Random(1)
    built = meta.get('arbor') or '?'
    out = [f"# arbor look over whole sessions: {meta.get('date', '?')[:10]}", '',
           f"{meta.get('opencode', 'OpenCode ?')} on `{meta.get('model')}`, tasks on this repository at "
           f"{meta.get('commit', '?')[:7]}, arbor "
           f"{'from ARBOR_BIN' if built == 'ARBOR_BIN' else 'built at ' + built[:7]}, {len(recs)} sessions "
           f"({meta.get('reps')} per task and arm). Load average {load_avg(meta)} at the start "
           f"({meta.get('date', '?')[11:16]}), {load_avg(end)} at the end ({end.get('date', '?')[11:16]}). A free "
           "remote model's answers vary from run to run, and the model behind the name can change, so a new run will "
           "not repeat these numbers exactly." + ''.join(
               f" The run stopped after session {r['after']} and went on at {r['date'][11:16]} (load average "
               f"{load_avg(r)}) with `--resume`." for r in meta.get('resumed', [])), '',
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
    if len(arms) == 2:
        a_arm, b_arm = arms if arms[0] == 'A' else arms[::-1]
        out += ['', f'{b_arm} against {a_arm}: the ratio of medians, its 95% bootstrap interval, and '
                f'P({b_arm}<{a_arm}).', '', '| task | weighted | steps | input | output | ok A | ok B |',
                '| :--- |' + ' ---: |' * 6]
        groups = {m: [] for m, _ in METRICS[:5]}
        for t in tasks + ['all tasks']:
            if t == 'all tasks':
                cells = []
                for m in ('weighted', 'steps', 'input', 'output'):
                    p, lo, hi = boot(groups[m], rng)
                    cells.append(f'{p:.2f} ({lo:.2f}–{hi:.2f})')
                ra = [r for r in recs if r['arm'] == a_arm]
                rb = [r for r in recs if r['arm'] == b_arm]
            else:
                ra = [r for r in recs if r['task'] == t and r['arm'] == a_arm]
                rb = [r for r in recs if r['task'] == t and r['arm'] == b_arm]
                if not ra or not rb:
                    continue
                cells = []
                for m, f in METRICS[:5]:
                    a, b = [f(r) for r in ra], [f(r) for r in rb]
                    groups[m].append((a, b))
                    if m == 'cache read':
                        continue
                    p, lo, hi = boot([(a, b)], rng)
                    cells.append(f'{p:.2f} ({lo:.2f}–{hi:.2f}), {below(a, b):.2f}')
            ok = [f"{sum(bool(r.get('success')) for r in rs)}/{len(rs)}" for rs in (ra, rb)]
            out.append(f'| {t} | ' + ' | '.join(cells) + f' | {ok[0]} | {ok[1]} |')
        ref = getattr(reference, 'SESSIONS_REF', None)
        if ref:
            out.append(f"| *reference, {ref['ref_date']}* | " + ' | '.join(
                '{:.2f} ({:.2f}–{:.2f})'.format(*ref[m]) for m in ('weighted', 'steps', 'input', 'output')) +
                f" | {ref['ok'][0]} | {ref['ok'][1]} |")
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
