"""The tasks of the whole-session A/B (ab.py), the arms' AGENTS.md and tool, and each task's check.

Every session works on this repository at COMMIT, exported with `git archive` into a new git repository of one commit,
in which the bugs in PLANTS are already in place, `CLAUDE.md` is left out and `AGENTS.md` is the arm's. With one commit,
`git log` and `git diff` give no hint of the plants. Both arms' repositories have the same path, so the folder name
tells the agent nothing either.

Each task needs reads of the code: find where something is defined, gather facts from a large file, work out what a
function does with given inputs, or find a bug from its symptom and fix it. A check decides each session's success
from the agent's last answer or from the files it left in the world. They were written before the runs, and a run
never changes them.
"""
import re

COMMIT = '38e62945848af949f48ca3af32688cd56a9bc29d'  # main on 17 Oct 2026, after #67

# Both arms: what the repository is, and to answer only what is asked.
COMMON = """\
# Agent instructions

This is the repository of arbor, a proposal for an operating system rebuilt for AI agents: a report of measurements
(`OPTIMIZATION.md`), the suite that makes them (`bench/`, in Python, JavaScript, shell and C), and a first prototype
in Rust (`prototype/arbor/`).

- Do the task you are given. Change files only when it asks for a change, and only the ones it needs.
- Do not commit, and do not run the suite or the prototype's tests.
"""

# Arm B only: read with `arbor look`.
LOOK = """
## Reading the code

Read with `arbor look`, in the shell, instead of reading whole files:

- `arbor look outline FILE` lists the definitions in a source file and the lines each spans; `arbor look lines FILE
  A-B` then prints only lines A to B. Read a large file this way, not whole.
- `arbor look files DIR` lists what a folder holds.
- Ask for several reads in one call, joined by `+`: `arbor look outline a.rs + outline b.rs + lines c.py 40-80`.
- `arbor look` alone describes every query and its options.
"""

# Arms C and D (#74), written before their runs: B's agent took more steps, walking folders where A's used grep,
# reading an outline and its lines in two calls, asking `arbor look` what it answers. C and D say to find with grep
# first and then make every read in one call, and describe the queries in full. C runs `arbor look` in the shell; D
# calls it as a tool of its own (TOOL), not through the shell, and finds the queries in the tool's description. Before
# the runs, a smoke test's agent in D sent `grep` to `arbor look` as a query: both now name the grep tool, and say that
# `arbor look` has no search.
QUERIES = """\
- `outline FILE`: the definitions in a Rust, Python, JavaScript, TypeScript or shell file, or a Markdown file's
  headings, each with the lines it spans, its comments and attributes included. `lines FILE LINE-END` then reads one
  whole.
- `lines FILE [A[-B]]`: lines A to B of a file, from 1 and to its end when left out.
- `files [DIR]`: what a folder holds.
- `status`, and `log [REV] [PATH]...`: what changed in the git repository, and its commits.

There is no query that searches: find with the grep tool. After a query, `-n ROWS` says how many rows it gives (0
for all). Each answer starts with a line `# QUERY`, which says what was cut or why the query failed.
"""

C_LOOK = """
## Reading the code

Read with `arbor look`, in the shell, in as few steps as you can:

- Find where things are with the grep tool first, rather than by listing folders.
- Then make every read the task needs in one call, joined by `+`:
  `arbor look outline src/a.rs + lines src/a.rs 120-160 + lines src/b.py 40-80`. Read a large file by its outline
  and the lines you need, not whole.

Its queries:

""" + QUERIES

D_LOOK = """
## Reading the code

Read with the `arbor_look` tool, in as few steps as you can:

- Find where things are with the grep tool first, rather than by listing folders.
- Then make every read the task needs in one `arbor_look` call, one query per item:
  `["outline src/a.rs", "lines src/a.rs 120-160", "lines src/b.py 40-80"]`. Read a large file by its outline and
  the lines you need, not whole.

Its queries are in the tool's description.
"""

ARMS = {'A': COMMON, 'B': COMMON + LOOK, 'C': COMMON + C_LOOK, 'D': COMMON + D_LOOK}

# The tool look_mcp.py offers in arm D, as the MCP server `arbor`, so that OpenCode names it `arbor_look`.
TOOL = {
    'name': 'look',
    'description': 'Reads the repository as short tables, several reads in one call: give every read the task needs '
                   'as one list. Each item is one query:\n\n' + QUERIES,
    'inputSchema': {'type': 'object', 'required': ['queries'], 'properties': {'queries': {
        'type': 'array', 'items': {'type': 'string'}, 'description': 'The queries, one per item, e.g. '
        '["outline src/a.rs", "lines src/a.rs 120-160", "files src"]'}}},
}
TOOL_ARMS = {'D'}  # the arms whose key's home also holds TOOL

# The bugs the fix tasks start from: (path, original text, planted text). Each original text appears once in its file.
PLANTS = [
    ('prototype/arbor/src/key.rs', 'is_some_and(|m| m.len() == 2)', 'is_some_and(|m| m.len() <= 2)'),
    ('prototype/arbor/src/main.rs', 'if n == 1 { "" } else { "s" }', 'if n <= 1 { "" } else { "s" }'),
    ('bench/sections/arbor.py', "return lines[-1] if lines else 'nothing on stderr'",
     "return lines[0] if lines else 'nothing on stderr'"),
]


def _plain(text):
    """The answer without Markdown marks and digit separators."""
    return re.sub(r'(?<=\d)[,_](?=\d)', '', re.sub(r'[`*]', '', text or ''))


def _block(text, start, python):
    """(lines before, the block, lines after) of the definition whose first line matches `start`, or None. A Rust
    block ends at the first `}` in column 0; a Python one before the next line in column 0, blank lines left out."""
    lines = text.splitlines()
    first = next((i for i, l in enumerate(lines) if re.match(start, l)), None)
    if first is None:
        return None
    end = first + 1
    if python:
        while end < len(lines) and (not lines[end].strip() or lines[end][0] in ' \t'):
            end += 1
        while end > first + 1 and not lines[end - 1].strip():
            end -= 1
    else:
        while end < len(lines) and lines[end - 1] != '}':
            end += 1
    return lines[:first], lines[first:end], lines[end:]


def _rest_same(now, orig, start, python):
    """True when everything outside the block is as in the original, but for trailing spaces."""
    a, b = _block(now, start, python), _block(orig, start, python)
    if a is None or b is None:
        return False, None
    strip = lambda ls: [l.rstrip() for l in ls]  # noqa: E731
    return strip(a[0]) == strip(b[0]) and strip(a[2]) == strip(b[2]), '\n'.join(a[1])


def check_find(answer, world):
    a = _plain(answer)
    found = {'path': bool(re.search(r'prototype/arbor/src/net\.rs', a)), 'name': bool(re.search(r'\bis_local\b', a)),
             'lines': bool(re.search(r'\b23[4-7]\s*(?:-|–|—|to|\.\.)\s*268\b', a))}
    return all(found.values()), ', '.join(k for k, v in found.items() if not v) or 'right'


KEPT = {'PATH', 'HOME', 'USER', 'LOGNAME', 'SHELL', 'TERM', 'COLORTERM', 'NO_COLOR', 'LANG', 'LANGUAGE', 'TZ', 'EDITOR',
        'VISUAL', 'PAGER'}


def check_list(answer, world):
    names = set(re.findall(r'\b[A-Z][A-Z0-9_]+\*?', _plain(answer)))
    missing = KEPT - names
    lc = any(x.startswith('LC_') for x in names)
    extra = {x for x in names - KEPT if not x.startswith('LC_')}
    ok = not missing and lc and not extra
    return ok, 'right' if ok else '; '.join(p for p in (
        missing and 'missing ' + ' '.join(sorted(missing)), not lc and 'missing LC_*',
        extra and 'extra ' + ' '.join(sorted(extra))) if p)


SIZES = ['805306368', '2684354560', 'refused', '3072']


def check_explain(answer, world):
    got = re.findall(r'\b\d{4,}\b|\brefused\b', _plain(answer).lower())[-4:]
    return got == SIZES, 'right' if got == SIZES else f'read {" ".join(got) or "nothing"}'


def _fix_rust(world, path, name, right):
    """Right when the function `name` matches one of the patterns in `right` and the rest of the file is unchanged."""
    same, body = _rest_same(world.read(path), world.original(path), rf'(pub )?fn {name}\b', python=False)
    fixed = bool(body and '<=' not in body and any(re.search(r, body) for r in right))
    return same and fixed, 'right' if same and fixed else (f'{name} not fixed' if same else f'changed more than {name}')


def check_fix_clock(answer, world):
    return _fix_rust(world, 'prototype/arbor/src/key.rs', 'm_ok', [r'len\(\)\s*==\s*2\b'])


def check_fix_plural(answer, world):
    return _fix_rust(world, 'prototype/arbor/src/main.rs', 'count', [
        r'n\s*==\s*1\s*\{\s*""\s*\}\s*else\s*\{\s*"s"\s*\}', r'n\s*!=\s*1\s*\{\s*"s"\s*\}\s*else\s*\{\s*""\s*\}'])


LAST_LINE = """\
import sys
sys.path.insert(0, 'bench/sections')
import arbor
cases = [(b'one\\ntwo\\nthree\\n', 'three'), (b'\\n  \\n', 'nothing on stderr'), (b'only', 'only')]
bad = [(i, arbor.last_line(i)) for i, want in cases if arbor.last_line(i) != want]
print(bad or 'ok')
"""


def check_fix_python(answer, world):
    path, start = 'bench/sections/arbor.py', r'def last_line\b'
    same, _ = _rest_same(world.read(path), world.original(path), start, python=True)
    ok, out = world.python(LAST_LINE)
    works = ok and out.strip() == 'ok'
    return same and works, 'right' if same and works else (
        'changed more than last_line' if not same else 'last_line still wrong: ' + out.strip()[-120:])


TASKS = {
    'find': {
        'kind': 'find a definition',
        'prompt': 'Which function in the prototype tells whether an IP address belongs to this computer or to a local '
                  'network? Reply with only its file, its name and the lines it spans, as `PATH NAME FIRST-LAST`.',
        'check': check_find,
    },
    'list': {
        'kind': 'gather facts from a large file',
        'prompt': 'Which variables of its own environment does `arbor run` pass into a world when the key adds none? '
                  'Reply with only their names, one per line.',
        'check': check_list,
    },
    'explain': {
        'kind': 'work out what a function does',
        'prompt': 'How does `arbor run --mem` read each of these sizes: `768M`, `2.5GiB`, `0.5G`, `3k`? Reply with '
                  'only four lines, one per size in this order: the number of bytes it reads, in digits, or `refused` '
                  'if arbor refuses that size.',
        'check': check_explain,
    },
    'fix-clock': {
        'kind': 'fix a bug in Rust from its symptom',
        'prompt': '`arbor key new notes --until 18:5` makes a key that ends at 18:05, but arbor should refuse it: '
                  'minutes must have two digits, as clocks show them. Find the cause and fix it, changing nothing '
                  'else.',
        'check': check_fix_clock,
    },
    'fix-plural': {
        'kind': 'fix a bug in Rust from its symptom',
        'prompt': 'When no command runs under it, `arbor key take-back dev` says `stopped 0 command`, where it should '
                  'say `stopped 0 commands`. Find the cause and fix it, changing nothing else.',
        'check': check_fix_plural,
    },
    'fix-python': {
        'kind': 'fix a bug in Python from its symptom',
        'prompt': "When the suite's `arbor` section cannot build the prototype, the error it records quotes the first "
                  "line of cargo's errors, where it should quote the last. Find the cause and fix it, changing nothing "
                  "else.",
        'check': check_fix_python,
    },
}
