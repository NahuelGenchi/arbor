"""A/B of whole agent sessions: what `arbor look` changes in the tokens and steps an agent spends on a task, start to
finish, rather than in one answer.

usage: python3 bench/sessions/ab.py [--reps N] [--tasks find,explain,...] [--arms A,B]

Runs OpenCode, unchanged, on each task of tasks.py, in a world of its own, under arms that differ only in the world's
AGENTS.md and, for the tool arm, one tool: A says nothing about `arbor look`; B says to read with it, in the shell; C
says to find with grep first and then to make every read in one `arbor look` call; D says the same of `arbor look`
offered as a tool of its own, by look_mcp.py. Each task runs --reps times per arm (5 when unset), the arms in turns
(AB, then BA; ACD, CDA, then DAC). Every session starts afresh:

- a copy of the arm's repository (tasks.py) at the same path, and a new world over it;
- a new key with a home folder of its own (`--home`), which opens OpenCode's program, OpenCode's two hosts (its
  models' API and its list of models) and that world only. Its settings, there alone, allow only OpenCode's own
  provider, name the free model for every call, the background ones included, and turn off updates, language servers
  and formatters. In the tool arm they also start look_mcp.py, copied into that home, as a local MCP server whose
  tool the agent can call directly, not only from Code Mode. No login goes in: the free model needs none, so no plan's
  quota can be reached;
- `opencode models`, which fetches the list of models into that home, then
  `opencode run --standalone --auto -m MODEL PROMPT`, its input closed.

When it ends, or after AB_TIMEOUT seconds (900 when unset, counted a failure), the session is counted from OpenCode's
own records, its database in the key's home, read-only and from the session tables only, never the one with logins:
each model call (a step) with its input, cache-read, cache-write, output and reasoning tokens, the tools it called,
the commands it ran in the shell, the queries it gave the `arbor look` tool, the code it gave Code Mode, and its last
answer. The task's check then decides success, from that answer or from the files left in the world. A session that
OpenCode ends with an error of the provider's, such as the free model's rate limit, runs again, at most twice; the
attempts stay in the records. When the provider ends every attempt, the run stops, since the rate limit lasts a
while: `--resume` goes on later, running again that session, any other whose every attempt the provider ended, and
those the run did not reach. Then the key is taken back, which deletes its home, and the world is thrown away.

The run stops if a step names another model or records a cost other than 0, or if MODEL is not one of the free ones
(its name ends in `-free`). arbor runs with ARBOR_HOME in the temp folder, so none of the person's worlds, keys or
ledger are touched, and with ARBOR_NO_CGROUP=1, so the runs neither need nor start `arbor serve`. The binary is
ARBOR_BIN, or a release build of prototype/ in the temp folder, made as the suite's arbor section makes it.

Writes RESULTS_DIR/sessions_<date>_<host>.jsonl, one record per session, and summary.py's report of it next to it as
.md. The records hold the agents' last answers, which are about this public repository, with the temp folder's path
replaced by `.`. Everything else lives in one temp folder, removed at the end, also on Ctrl-C.

Environment: OPENCODE (~/.opencode/bin/opencode), AB_MODEL (opencode/deepseek-v4.1-flash-free), AB_TIMEOUT (900),
AB_GAP (seconds to wait after each session, 0 when unset: the free model's rate limit ends sessions after about 120
in an hour), ARBOR_BIN, RESULTS_DIR (bench/results), BENCH_TMP or TMPDIR (/tmp).
"""
import argparse
import datetime
import json
import os
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(HERE)
ROOT = os.path.dirname(BENCH)
sys.path.insert(0, HERE)
import summary  # noqa: E402
import tasks as T  # noqa: E402

OPENCODE = os.environ.get('OPENCODE') or os.path.expanduser('~/.opencode/bin/opencode')
MODEL = os.environ.get('AB_MODEL') or 'opencode/deepseek-v4.1-flash-free'
TIMEOUT = float(os.environ.get('AB_TIMEOUT') or 900)
GAP = float(os.environ.get('AB_GAP') or 0)
RESULTS = os.environ.get('RESULTS_DIR') or os.path.join(BENCH, 'results')
HOSTS = ['opencode.ai', 'models.opencode.ai']
SETTINGS = {'model': MODEL, 'small_model': MODEL, 'enabled_providers': [MODEL.split('/')[0]], 'autoupdate': False,
            'lsp': False, 'formatter': False}
# The tool arm's MCP server, as OpenCode names it, and so the tool: `arbor_look`
MCP = 'arbor'
LOOK_TOOL = f"{MCP}_{T.TOOL['name']}"
# look_mcp.py and its tool, in the key's home; the server runs in the cell, in the agent's folder
MCP_DIR = '.config/opencode'
MCP_COMMAND = ['sh', '-c', f'exec python3 -I -B "$HOME/{MCP_DIR}/look_mcp.py" "$HOME/{MCP_DIR}/look_tool.json"']
WORLD = 'task'
RETRIES = 2
CHILD = None  # the session running now, stopped if the run is interrupted


def sh(argv, **kw):
    kw.setdefault('stdout', subprocess.PIPE)
    kw.setdefault('stderr', subprocess.PIPE)
    return subprocess.run(argv, **kw)


class Arbor:
    def __init__(self, binary, home):
        self.bin, self.env = binary, dict(os.environ, ARBOR_HOME=home, ARBOR_NO_CGROUP='1')

    def __call__(self, *args, check=True, **kw):
        r = sh([self.bin, *args], env=self.env, **kw)
        if check and r.returncode != 0:
            err = (r.stderr or b'').decode(errors='replace').strip().splitlines()
            raise RuntimeError(f'`arbor {" ".join(args[:3])}` exited with {r.returncode}: {err[-1] if err else ""}')
        return r


class World:
    """What a check may ask of the world a session left."""

    def __init__(self, arbor):
        self.arbor = arbor

    def read(self, path):
        r = self.arbor('run', WORLD, '--', 'cat', path, check=False)
        return r.stdout.decode(errors='replace') if r.returncode == 0 else ''

    def original(self, path):
        return sh(['git', '-C', ROOT, 'show', f'{T.COMMIT}:{path}'], check=True).stdout.decode()

    def python(self, code):
        r = self.arbor('run', WORLD, '--', 'python3', '-B', '-c', code, check=False)
        return r.returncode == 0, (r.stdout + r.stderr).decode(errors='replace')


def build(work):
    """ARBOR_BIN, or the suite's release build of prototype/ in the temp folder, apart from the task's folder."""
    os.environ['BENCH_WORK'] = os.path.join(work, 'build')
    os.makedirs(os.environ['BENCH_WORK'])
    sys.path.insert(0, os.path.join(BENCH, 'sections'))
    import arbor as section  # noqa: E402  (the suite's arbor section: its build() is reused as is)
    path, err = section.build()
    if err:
        sys.exit(f'ab.py: {err}')
    return path


def repository(arm, dest):
    """This repository at T.COMMIT, with the plants and the arm's AGENTS.md, as a new git repository of one commit."""
    os.makedirs(dest)
    tar = sh(['git', '-C', ROOT, 'archive', T.COMMIT], check=True).stdout
    sh(['tar', '-x', '-C', dest], input=tar, check=True)
    os.remove(os.path.join(dest, 'CLAUDE.md'))
    with open(os.path.join(dest, 'AGENTS.md'), 'w') as f:
        f.write(T.ARMS[arm])
    for path, old, new in T.PLANTS:
        p = os.path.join(dest, path)
        text = open(p).read()
        if text.count(old) != 1:
            sys.exit(f'ab.py: the plant in {path} does not match {T.COMMIT[:7]}')
        with open(p, 'w') as f:
            f.write(text.replace(old, new))
    when = sh(['git', '-C', ROOT, 'show', '-s', '--format=%cI', T.COMMIT], check=True).stdout.decode().strip()
    env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='arbor',
               GIT_AUTHOR_EMAIL='arbor@example.invalid', GIT_COMMITTER_NAME='arbor',
               GIT_COMMITTER_EMAIL='arbor@example.invalid', GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
    for argv in (['init', '-q', '-b', 'main'], ['add', '-A'], ['commit', '-q', '-m', f'arbor at {T.COMMIT[:7]}']):
        sh(['git', '-C', dest, *argv], env=env, check=True)


def count(db, folder):
    """One session's numbers from OpenCode's database: its sessions and their messages, nothing else."""
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    try:
        sessions = con.execute('SELECT id, parent_id, cost FROM session_v2 ORDER BY time_created').fetchall()
        rows = con.execute('SELECT session_id, type, data FROM session_message ORDER BY session_id, seq').fetchall()
    finally:
        con.close()
    main = next((s[0] for s in sessions if s[1] is None), None)
    rec = {'sessions': len(sessions), 'steps': 0, 'input': 0, 'output': 0, 'reasoning': 0, 'cache_read': 0,
           'cache_write': 0, 'cost': sum(s[2] or 0 for s in sessions), 'models': [], 'tools': {}, 'tool_chars': {},
           'arbor_look': 0, 'commands': [], 'errors': [], 'finish': {}, 'context_last': 0, 'context_max': 0,
           'outcome': None, 'answer': '', 'first_ms': None, 'last_ms': None}
    for sid, kind, data in rows:
        d = json.loads(data)
        t = (d.get('time') or {}).get('created')
        if t:
            rec['first_ms'] = min(rec['first_ms'] or t, t)
            rec['last_ms'] = max(rec['last_ms'] or t, t)
        if kind == 'idle' and sid == main:
            rec['outcome'] = d.get('outcome')
        if kind != 'assistant':
            continue
        rec['steps'] += 1
        tok = d.get('tokens') or {}
        cache = tok.get('cache') or {}
        for k, v in (('input', tok.get('input')), ('output', tok.get('output')), ('reasoning', tok.get('reasoning')),
                     ('cache_read', cache.get('read')), ('cache_write', cache.get('write'))):
            rec[k] += v or 0
        ctx = (tok.get('input') or 0) + (cache.get('read') or 0) + (cache.get('write') or 0)
        rec['context_max'] = max(rec['context_max'], ctx)
        rec['cost'] += d.get('cost') or 0
        m = d.get('model') or {}
        name = f"{m.get('providerID')}/{m.get('id')}"
        if name not in rec['models']:
            rec['models'].append(name)
        f = d.get('finish') or 'none'
        rec['finish'][f] = rec['finish'].get(f, 0) + 1
        if d.get('error'):
            rec['errors'].append(str(d['error'])[:300])
        text = []
        for part in d.get('content') or []:
            if part.get('type') == 'text':
                text.append(part.get('text') or '')
            if part.get('type') != 'tool':
                continue
            tool = part.get('name') or '?'
            rec['tools'][tool] = rec['tools'].get(tool, 0) + 1
            state = part.get('state') or {}
            out = sum(len(c.get('text') or '') for c in state.get('content') or [] if isinstance(c, dict))
            rec['tool_chars'][tool] = rec['tool_chars'].get(tool, 0) + out
            given = state.get('input') or {}
            if tool in ('shell', 'bash'):  # OpenCode 2 calls its shell tool `shell`
                command = str(given.get('command') or ' '.join(map(str, given.values()))).replace(folder, '.')
                rec['commands'].append(command[:300])
                rec['arbor_look'] += 'arbor look' in command
            elif tool in (LOOK_TOOL, T.TOOL['name']):
                rec['commands'].append(f'{tool} {json.dumps(given.get("queries"))}'.replace(folder, '.')[:300])
                rec['arbor_look'] += 1
            elif tool == 'execute':  # Code Mode, whose code may call the shell or the tool as `tools.arbor.look`
                code = str(given.get('code') or '')
                rec['commands'].append(f'execute {code}'.replace(folder, '.')[:300])
                rec['arbor_look'] += code.count('arbor look') + code.count(f'.{MCP}.{T.TOOL["name"]}(')
        if sid == main:
            rec['context_last'] = ctx
            if text:
                rec['answer'] = '\n'.join(text).replace(folder, '.')
    return rec


def session(arbor, work, task, arm, n):
    """Runs one session and returns its record."""
    global CHILD
    folder = os.path.join(work, 'arbor')
    if os.path.exists(folder):
        shutil.rmtree(folder)
    shutil.copytree(os.path.join(work, f'repo-{arm}'), folder, symlinks=True)
    key = f'k{n:03d}'
    arbor('new', WORLD, '--from', folder)
    args = ['key', 'new', key, '--read', os.path.dirname(OPENCODE), '--home', '--world', WORLD, '--for', '2h']
    for h in HOSTS:
        args += ['--net', h]
    arbor(*args)
    settings, files = dict(SETTINGS), {}
    if arm in T.TOOL_ARMS:
        settings['mcp'] = {MCP: {'type': 'local', 'command': MCP_COMMAND, 'codemode': False}}
        with open(os.path.join(HERE, 'look_mcp.py'), 'rb') as f:
            files = {'look_mcp.py': f.read(), 'look_tool.json': json.dumps(T.TOOL).encode()}
    for name, data in {'opencode.json': json.dumps(settings).encode(), **files}.items():
        arbor('run', WORLD, '--key', key, '--', 'sh', '-c', f'mkdir -p ~/{MCP_DIR} && cat > ~/{MCP_DIR}/{name}',
              input=data)
    for attempt in range(5):  # a new home fetches the list of models in the background, and it can miss for a while
        listed = arbor('run', WORLD, '--key', key, '--', OPENCODE, 'models', '--standalone', check=False)
        if MODEL in listed.stdout.decode(errors='replace').split():
            break
        time.sleep(5 * (attempt + 1))
    else:
        raise RuntimeError(f'{MODEL} is not among the models OpenCode lists')
    rec = {'n': n, 'task': task, 'arm': arm, 'started': datetime.datetime.now().isoformat(timespec='seconds'),
           'load': os.getloadavg()[0]}
    prompt = T.TASKS[task]['prompt']
    t0 = time.monotonic()
    argv = [arbor.bin, 'run', WORLD, '--key', key, '--', OPENCODE, 'run', '--standalone', '--auto', '--title', 'task',
            '-m', MODEL, prompt]
    p = CHILD = subprocess.Popen(argv, env=arbor.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.PIPE)
    try:
        _, err = p.communicate(timeout=TIMEOUT)
        rec['timed_out'] = False
    except subprocess.TimeoutExpired:
        stop(arbor)
        _, err = p.communicate()
        rec['timed_out'] = True
    CHILD = None
    rec['wall_s'] = round(time.monotonic() - t0, 1)
    rec['exit'] = p.returncode
    # arbor's own line naming the hosts the key kept out; OpenCode's lines on stderr may quote other text of arbor's
    said = re.findall(rf"^arbor: key '{key}' does not open (.+?), so the command could not reach",
                      err.decode(errors='replace'), re.M)
    rec['refused_hosts'] = sorted({h for line in said for h in re.split(r', | and ', line)})
    db = os.path.join(work, 'home', 'homes', key, '.local', 'share', 'opencode', 'opencode.db')
    rec.update(count(db, folder) if os.path.exists(db) else {'steps': 0, 'errors': ['no database']})
    ok, why = T.TASKS[task]['check'](rec.get('answer', ''), World(arbor))
    rec['success'], rec['why'] = bool(ok and not rec['timed_out']), why
    diff = arbor('diff', WORLD, check=False).stdout.decode(errors='replace').splitlines()
    rec['changed'] = [ln for ln in diff if ln.strip() and ' .git/' not in ln][:20]  # git touches its own objects
    arbor('key', 'take-back', key, check=False)
    arbor('discard', WORLD, check=False)
    shutil.rmtree(folder)
    return rec


def stop(arbor):
    """Stops the command running in the world, as `arbor stop` does: its cell, found with `arbor ps`."""
    for line in arbor('ps', check=False).stdout.decode(errors='replace').splitlines()[1:]:
        cols = line.split()
        if len(cols) > 1 and cols[1] == WORLD:
            arbor('stop', cols[0], check=False)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--reps', type=int, default=5)
    ap.add_argument('--tasks', default=','.join(T.TASKS))
    ap.add_argument('--arms', default='A,B')
    ap.add_argument('--resume', metavar='JSONL', help="an earlier run's records: its sessions are kept, and the run "
                    'goes on with its settings, running the sessions it did not finish, or that the provider ended')
    a = ap.parse_args()
    names, arms, done, n = a.tasks.split(','), a.arms.split(','), set(), 0
    if a.resume:
        with open(a.resume) as f:
            old = [json.loads(line) for line in f]
        meta = next(r for r in old if r.get('meta'))
        if (meta['commit'], meta['model']) != (T.COMMIT, MODEL):
            sys.exit(f'ab.py: {a.resume} ran on another commit or model')
        names, arms, a.reps = meta['tasks'], meta['arms'], meta['reps']
        done = {(r['rep'], r['task'], r['arm']) for r in old if r.get('final') and not summary.provider_error(r)}
        n = max((r.get('n', 0) for r in old), default=0)
    if not MODEL.endswith('-free'):
        sys.exit(f'ab.py: {MODEL} is not one of the free models; this A/B spends nothing')
    if any(t not in T.TASKS for t in names) or any(x not in T.ARMS for x in arms):
        sys.exit(f'ab.py: tasks are {", ".join(T.TASKS)}; arms are {", ".join(T.ARMS)}')
    base = os.environ.get('BENCH_TMP') or os.environ.get('TMPDIR') or '/tmp'
    work = tempfile.mkdtemp(prefix='arbor-ab.', dir=base)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    os.makedirs(RESULTS, exist_ok=True)
    stamp = f"sessions_{datetime.datetime.now():%Y-%m-%d_%H%M}_{socket.gethostname().split('.')[0]}"
    out = a.resume or os.path.join(RESULTS, stamp + '.jsonl')
    try:
        arbor = Arbor(build(work), os.path.join(work, 'home'))
        for arm in arms:
            repository(arm, os.path.join(work, f'repo-{arm}'))
        version = sh([OPENCODE, '--version']).stdout.decode().strip()
        meta = {'meta': True, 'date': datetime.datetime.now().isoformat(timespec='seconds'), 'model': MODEL,
                'opencode': version, 'commit': T.COMMIT, 'reps': a.reps, 'tasks': names, 'arms': arms,
                'timeout_s': TIMEOUT, 'load': os.getloadavg(),
                'arbor': 'ARBOR_BIN' if os.environ.get('ARBOR_BIN') else summary.arbor_commit(ROOT)}
        if a.resume:
            meta = {'meta_resume': True, 'after': n, **{k: meta[k] for k in ('date', 'opencode', 'load', 'arbor')}}
        with open(out, 'a' if a.resume else 'w') as f:
            f.write(json.dumps(meta) + '\n')
        for rep in range(a.reps):
            for i, task in enumerate(names):
                turn = (rep + i) % len(arms)
                for arm in arms[turn:] + arms[:turn]:
                    if (rep, task, arm) in done:
                        continue
                    for attempt in range(RETRIES + 1):
                        n += 1
                        rec = session(arbor, work, task, arm, n)
                        rec.update(rep=rep, attempt=attempt, final=True)
                        bad = [m for m in rec.get('models', []) if m != MODEL]
                        if bad or rec.get('cost'):
                            with open(out, 'a') as f:
                                f.write(json.dumps(rec) + '\n')
                            sys.exit(f'ab.py: session {n} used {bad or MODEL} at a cost of {rec.get("cost")}; stopped')
                        again = summary.provider_error(rec) and attempt < RETRIES
                        rec['final'] = not again
                        with open(out, 'a') as f:
                            f.write(json.dumps(rec) + '\n')
                        print(f"{n:3d} {task:10s} {arm} rep {rep} steps {rec.get('steps', 0):3d} "
                              f"look {rec.get('arbor_look', 0):2d} {'ok ' if rec['success'] else 'no '} "
                              f"{rec['wall_s']:6.1f}s {rec['why']}", flush=True)
                        time.sleep(GAP)
                        if not again:
                            break
                    if summary.provider_error(rec):
                        sys.exit(f'ab.py: the provider ended every attempt of session {n} '
                                 f'({(rec.get("errors") or ["?"])[0][:100]}); go on later with --resume {out}')
        meta_end = {'meta_end': True, 'date': datetime.datetime.now().isoformat(timespec='seconds'),
                    'load': os.getloadavg()}
        with open(out, 'a') as f:
            f.write(json.dumps(meta_end) + '\n')
        md = summary.report(out)
        with open(out[:-len('.jsonl')] + '.md', 'w') as f:
            f.write(md)
        print(md)
    finally:
        if CHILD and CHILD.poll() is None:
            stop(arbor)
            CHILD.wait()
        sh(['chmod', '-R', 'u+rwX', work])  # overlay work folders are made without permissions
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    main()
