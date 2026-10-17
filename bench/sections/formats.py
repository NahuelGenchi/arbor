"""7b. Output formats in o200k tokens: one 100-file listing printed eight ways, git log with and without
projection, ps/ip/df in full vs the asked fields.

Listing: the first 100 names (code-point order) in $LISTING_DIR with type, mode, owner, size and mtime; `ls -la`
of the same directory (head -101) is the human-text baseline. git log runs on $GIT_LOG_REPO at $GIT_LOG_REF, which
by default is the commit the M run saw, so the count does not drift as the repo grows. Commands run with the
locale of the M run ($FORMATS_LOCALE, en_US.UTF-8): ls order and dates depend on it.
"""
import json
import os
import pwd
import stat
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import BENCH_DIR, WORK, emit, env, fail  # noqa: E402

LOC = dict(os.environ, LC_ALL=env('FORMATS_LOCALE', 'en_US.UTF-8'))


def sh(cmd):
    return subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True, env=LOC).stdout


def ls_rows(d, n=100):
    rows = []
    for name in sorted(os.listdir(d))[:n]:
        st = os.lstat(os.path.join(d, name))
        try:
            owner = pwd.getpwuid(st.st_uid).pw_name
        except KeyError:
            owner = str(st.st_uid)
        rows.append({'name': name,
                     'type': 'link' if stat.S_ISLNK(st.st_mode) else ('dir' if stat.S_ISDIR(st.st_mode) else 'file'),
                     'mode': stat.filemode(st.st_mode), 'owner': owner, 'size': st.st_size,
                     'modified': time.strftime('%Y-%m-%dT%H:%M', time.localtime(st.st_mtime))})
    return rows


def yaml(rows):
    out = []
    for r in rows:
        first = True
        for k, v in r.items():
            out.append(('- ' if first else '  ') + f'{k}: {v}')
            first = False
    return '\n'.join(out)


def tsv(rows):
    ks = list(rows[0])
    return '\t'.join(ks) + '\n' + '\n'.join('\t'.join(str(r[k]) for k in ks) for r in rows)


def md(rows):
    ks = list(rows[0])
    return ('| ' + ' | '.join(ks) + ' |\n|' + '---|' * len(ks) + '\n'
            + '\n'.join('| ' + ' | '.join(str(r[k]) for k in ks) + ' |' for r in rows))


def toon(rows, name='files'):
    ks = list(rows[0])
    return f'{name}[{len(rows)}]{{{",".join(ks)}}}:\n' + '\n'.join('  ' + ','.join(str(r[k]) for k in ks) for r in rows)


def git_target():
    """(repo, ref): $GIT_LOG_REPO if set, else the repo under $PROJECTS_DIR that holds $GIT_LOG_REF,
    else the most recently committed repo there at HEAD."""
    ref, repo = env('GIT_LOG_REF', 'HEAD'), env('GIT_LOG_REPO')
    if repo:
        return repo, ref
    root = env('PROJECTS_DIR')
    if not root or not os.path.isdir(root):
        return None, ref
    repos = [os.path.join(root, d) for d in sorted(os.listdir(root)) if os.path.isdir(os.path.join(root, d, '.git'))]
    for r in repos:
        if subprocess.run(['git', '-C', r, 'cat-file', '-e', f'{ref}^{{commit}}'], capture_output=True).returncode == 0:
            return r, ref
    latest = []
    for r in repos:
        t = subprocess.run(['git', '-C', r, 'log', '-1', '--format=%ct'], capture_output=True, text=True).stdout.strip()
        if t:
            latest.append((int(t), r))
    return (max(latest)[1], 'HEAD') if latest else (None, ref)


def main():
    v = {}
    d = env('LISTING_DIR', '/usr/bin')
    rows = ls_rows(d, 100)
    v['fmt.ls_human'] = sh(f'ls -la {d} | head -101')
    v['fmt.json_pretty'] = json.dumps(rows, indent=2)
    v['fmt.json_min'] = json.dumps(rows, separators=(',', ':'))
    v['fmt.yaml'] = yaml(rows)
    v['fmt.markdown'] = md(rows)
    v['fmt.tsv'] = tsv(rows)
    v['fmt.toon'] = toon(rows)
    v['fmt.names'] = '\n'.join(r['name'] for r in rows)
    repo, ref = git_target()
    if repo:
        for k, extra in (('fmt.git_log', []), ('fmt.git_log_stat', ['--stat']), ('fmt.git_log_oneline', ['--oneline'])):
            v[k] = subprocess.run(['git', '-C', repo, 'log', *extra, '-20', ref], capture_output=True, text=True,
                                  env=LOC).stdout
    v['fmt.ps_aux'] = sh('ps aux --sort=-rss | head -31')
    v['fmt.ps_fields'] = sh('ps -eo pid,rss,comm --sort=-rss | head -31')
    v['fmt.ip_addr'] = sh(['ip', 'addr'])
    v['fmt.ip_br'] = sh(['ip', '-br', 'addr'])
    v['fmt.df'] = sh(['df', '-h'])
    v['fmt.df_real'] = sh(['df', '-h', '-x', 'squashfs', '-x', 'tmpfs', '-x', 'devtmpfs', '-x', 'efivarfs', '-x', 'overlay'])
    src = os.path.join(WORK, 'formats.json')
    json.dump(v, open(src, 'w'))
    r = subprocess.run(['node', os.path.join(BENCH_DIR, 'lib', 'o200k.js'), '--json', src], capture_output=True, text=True)
    os.remove(src)
    if r.returncode != 0:
        return fail(list(v), 'o200k counter failed: ' + r.stderr)
    counts = json.loads(r.stdout)
    git_ctx = {'repo': os.path.basename(repo) if repo else None, 'ref': ref[:12]}
    for k, n in counts.items():
        emit(k, n, dict(git_ctx, chars=len(v[k])) if k.startswith('fmt.git') else {'chars': len(v[k])})
    if not repo:
        fail(['fmt.git_log', 'fmt.git_log_oneline'], 'no git repository found (set GIT_LOG_REPO)')


if __name__ == '__main__':
    main()
