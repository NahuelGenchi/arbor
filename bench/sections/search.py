"""4. Search: names over $NAME_SEARCH_ROOT and code under $CODE_ROOT.

Names: `find -iname` and `rg --files -g` (one warm-up run each, then the median of 3) against a catalog in the shape
of Arbor's: one SQLite row per object (id, parent id, name) with an FTS5 trigram index on names only. A query
returns ids and rebuilds each hit's path through the parent chain. Also: per-object size of the catalog, cost of one
incremental insert, and an exact-name lookup in a hash map.
Code: `rg -l -F <literal>` against an FTS5 trigram index over the same text files (files <= 1 MB, no NUL in the
first 8 KiB, as listed by `rg --files`), median of 3 and 5 runs. The databases live in the temp dir (tmpfs).
"""
import collections
import os
import shutil
import sqlite3
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import WORK, child_disk_mib, emit, env, fail, timed  # noqa: E402

NAME_KEYS = ['search.find_ms', 'search.rg_files_ms', 'search.catalog_ms', 'search.catalog_bytes_per_obj',
             'search.catalog_mib', 'search.catalog_update_us', 'search.catalog_build_s', 'search.exact_name_us',
             'search.entries']


def listed(argv, disk):
    """Run a lister; -> lines of output. Appends the MiB it read from storage (a cache miss) to disk."""
    d0 = child_disk_mib()
    n = subprocess.run(argv, capture_output=True).stdout.count(b'\n')
    disk.append(round(child_disk_mib() - d0, 1))
    return n


def name_search(root, q, rg):
    find_cmd = ['find', root, '-xdev', '-iname', f'*{q}*']
    subprocess.run(find_cmd, capture_output=True)  # warm the dentry/inode cache
    disk = []
    t_find, hits_find, runs = timed(lambda: listed(find_cmd, disk), 3)
    emit('search.find_ms', t_find, {'hits': hits_find, 'runs': [round(x) for x in runs], 'disk_mib': disk})
    if rg:
        rg_cmd = [rg, '--files', '--no-ignore', '--hidden', '-g', f'*{q}*', '--iglob', f'*{q}*', root]
        subprocess.run(rg_cmd, capture_output=True)
        disk = []
        t_rg, hits_rg, runs = timed(lambda: listed(rg_cmd, disk), 3)
        emit('search.rg_files_ms', t_rg, {'hits': hits_rg, 'runs': [round(x) for x in runs], 'disk_mib': disk})
    else:
        fail('search.rg_files_ms', 'rg not found')

    # Catalog: one row per object, trigram index on the name only.
    t0 = time.perf_counter()
    ids = {root: 1}
    rows = [(1, 0, root)]
    nid = 1
    by_name = collections.defaultdict(list)
    for dp, dns, fns in os.walk(root):
        pid = ids.pop(dp)
        for n in dns:
            nid += 1
            ids[dp + '/' + n] = nid
            rows.append((nid, pid, n))
            by_name[n].append(nid)
        for n in fns:
            nid += 1
            rows.append((nid, pid, n))
            by_name[n].append(nid)
    walk_s = time.perf_counter() - t0
    emit('search.entries', len(rows), {'walk_s': round(walk_s, 1)})

    t_hash, _, _ = timed(lambda: by_name.get('index.html'), 1001)
    emit('search.exact_name_us', t_hash * 1000)
    del by_name, ids

    db = os.path.join(WORK, 'names.db')
    con = sqlite3.connect(db)
    con.execute('pragma journal_mode=off')
    con.execute('pragma synchronous=off')
    con.execute('create table node(id integer primary key, parent int, name text)')
    con.execute("create virtual table nx using fts5(name, content='node', content_rowid='id', "
                "tokenize='trigram case_sensitive 0')")
    t0 = time.perf_counter()
    con.executemany('insert into node values (?,?,?)', rows)
    con.execute('insert into nx(rowid,name) select id,name from node')
    con.commit()
    emit('search.catalog_build_s', time.perf_counter() - t0)
    n_rows = len(rows)
    del rows
    con.execute('vacuum')
    size = os.path.getsize(db)
    emit('search.catalog_mib', size / 2**20)
    emit('search.catalog_bytes_per_obj', size / n_rows)

    def path(i):
        parts = []
        while i:
            p, n = con.execute('select parent,name from node where id=?', (i,)).fetchone()
            parts.append(n)
            i = p
        return '/'.join(reversed(parts))

    def query():
        return [path(r[0]) for r in con.execute('select rowid from nx where name like ?', (f'%{q}%',))]
    t_q, hits, _ = timed(query, 9)
    emit('search.catalog_ms', t_q, {'hits': len(hits)})

    t0 = time.perf_counter()
    for k in range(1000):
        nid += 1
        con.execute('insert into node values (?,?,?)', (nid, 1, f'new-file-{k}.txt'))
        con.execute('insert into nx(rowid,name) values (?,?)', (nid, f'new-file-{k}.txt'))
    con.commit()
    emit('search.catalog_update_us', (time.perf_counter() - t0) * 1e6 / 1000)
    con.close()
    os.remove(db)


CODE_KEYS = ['code.files', 'code.mib', 'code.index_mib', 'code.build_s']


def code_search(root, queries, rg):
    files = subprocess.run([rg, '--files', root], capture_output=True, text=True).stdout.split('\n')
    docs, tot = [], 0
    for f in files:
        if not f:
            continue
        try:
            if os.path.getsize(f) > 1_000_000:
                continue
            b = open(f, 'rb').read()
            if b'\0' in b[:8192]:
                continue
            docs.append((f, b.decode('utf-8', 'replace')))
            tot += len(b)
        except OSError:
            pass
    emit('code.files', len(docs))
    emit('code.mib', tot / 2**20)
    db = os.path.join(WORK, 'code.db')
    con = sqlite3.connect(db)
    con.execute('pragma journal_mode=off')
    con.execute('pragma synchronous=off')
    con.execute("create virtual table c using fts5(path unindexed, body, tokenize='trigram')")
    t0 = time.perf_counter()
    con.executemany('insert into c values (?,?)', docs)
    con.commit()
    emit('code.build_s', time.perf_counter() - t0)
    del docs
    con.execute('vacuum')
    size = os.path.getsize(db)
    emit('code.index_mib', size / 2**20, {'x_text': round(size / tot, 2) if tot else None})
    for i, q in enumerate(queries):
        argv, disk = [rg, '-l', '-F', q, root], []
        subprocess.run(argv, capture_output=True)  # warm-up, so every query starts from a warm page cache

        def rg_once():
            d0 = child_disk_mib()
            n = subprocess.run(argv, capture_output=True).stdout.count(b'\n')
            disk.append(child_disk_mib() - d0)
            return n
        t_rg, hits_rg, _ = timed(rg_once, 3)
        # 3 blocks of 7 (M: median of 5): a few-ms query moves a lot with load, so the spread goes into the note.
        blocks = [timed(lambda: len(con.execute('select path from c where body match ?',
                                                ('"' + q.replace('"', '""') + '"',)).fetchall()), 7) for _ in range(3)]
        t_ix, hits_ix = statistics.median(b[0] for b in blocks), blocks[-1][1]
        emit(f'code.rg_ms.{i}', t_rg, {'query': q, 'files': hits_rg, 'disk_mib': round(statistics.median(disk), 1)})
        emit(f'code.index_ms.{i}', t_ix, {'query': q, 'files': hits_ix, 'speedup': round(t_rg / t_ix) if t_ix else None,
                                          'blocks': [round(b[0], 3) for b in blocks]})
    con.close()
    os.remove(db)


def main():
    rg = shutil.which('rg')
    try:
        name_search(os.path.abspath(env('NAME_SEARCH_ROOT', os.path.expanduser('~'))), env('NAME_QUERY', 'invoice'), rg)
    except Exception as e:
        fail(NAME_KEYS, e)
    queries = env('CODE_QUERIES', 'parseInt|retroachievements|TODO:|fn main|border-radius').split('|')
    if not rg:
        return fail(CODE_KEYS + [f'code.rg_ms.{i}' for i in range(len(queries))], 'rg not found')
    try:
        code_search(os.path.abspath(env('CODE_ROOT') or os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', '..')),
                    queries, rg)
    except Exception as e:
        fail(CODE_KEYS, e)


if __name__ == '__main__':
    main()
