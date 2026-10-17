"""3a. Memory state snapshot: swap and zram configuration and usage, available RAM, RAM by process group.

These are readings of the live system, not benchmarks: they depend on what runs and what is swapped out at the
moment. Taken first, before the suite's own heavy sections. Process RSS comes from /proc/<pid>/status (readable for
every process); PSS from smaps_rollup is only readable for this user's processes.
"""
import collections
import glob
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import emit, fail  # noqa: E402

GIB = 1024 ** 3


def meminfo():
    d = {}
    for line in open('/proc/meminfo'):
        k, v = line.split(':', 1)
        d[k] = int(v.split()[0]) * 1024
    return d


def swaps():
    rows = []
    for line in open('/proc/swaps').read().splitlines()[1:]:
        name, typ, size, used, prio = line.split()[:5]
        rows.append({'name': name, 'type': typ, 'size': int(size) * 1024, 'used': int(used) * 1024})
    return rows


def zram():
    out = []
    for z in sorted(glob.glob('/sys/block/zram*')):
        try:
            algo = re.search(r'\[([^\]]+)\]', open(f'{z}/comp_algorithm').read()).group(1)
            mm = [int(x) for x in open(f'{z}/mm_stat').read().split()]
            out.append({'dev': '/dev/' + os.path.basename(z), 'algo': algo,
                        'disksize': int(open(f'{z}/disksize').read()), 'orig': mm[0], 'compr': mm[1], 'total': mm[2]})
        except (OSError, AttributeError, IndexError):
            pass
    return out


def swap_state():
    mi, sw, zr = meminfo(), swaps(), zram()
    zdevs = {z['dev'] for z in zr}
    disk = [s for s in sw if s['name'] not in zdevs]
    used = mi['SwapTotal'] - mi['SwapFree']
    emit('mem.swapfile_gib', sum(s['size'] for s in disk) / GIB, {'devices': [s['name'] for s in disk]})
    emit('mem.zram_gib', sum(z['disksize'] for z in zr) / GIB)
    emit('mem.zram_algo', ','.join(z['algo'] for z in zr) or 'none')
    emit('mem.swap_used_gib', used / GIB)
    emit('mem.swap_disk_used_gib', sum(s['used'] for s in disk) / GIB)
    emit('mem.available_gib', mi['MemAvailable'] / GIB, {'total_gib': round(mi['MemTotal'] / GIB, 1)})
    orig, total = sum(z['orig'] for z in zr), sum(z['total'] for z in zr)
    if total:
        emit('mem.zram_ratio', orig / total, {'orig_gib': round(orig / GIB, 2), 'stored_gib': round(total / GIB, 2)})
    else:
        fail('mem.zram_ratio', 'zram holds no data right now' if zr else 'no zram device')


def group(name, cmd):
    """Process grouping of the M snapshot (same rules)."""
    c = cmd.lower()
    if 'claude' in c and ('electron' in c or '/opt/claude' in c or 'claude-desktop' in c or '--type=' in c):
        return 'electron'
    if re.search(r'(^|/)claude( |$)', cmd) or 'claude-code' in c or '@anthropic-ai' in c:
        return 'claude_code'
    if 'chrome' in c or 'chromium' in c:
        return 'chrome'
    if (name in ('gnome-shell', 'Xwayland', 'mutter-x11-fram', 'gnome-session-b', 'gnome-session-c', 'gsd-xsettings')
            or name.startswith('gsd-') or 'gnome' in name or name.startswith('xdg-')
            or name in ('at-spi2-registr', 'at-spi-bus-laun', 'ibus-daemon', 'ibus-x11', 'ibus-extension-',
                        'ibus-engine-sim', 'ibus-portal', 'ibus-memconf', 'evolution-sourc', 'evolution-calen',
                        'evolution-addre', 'evolution-alarm', 'goa-daemon', 'goa-identity-se', 'gvfsd',
                        'tracker-miner-f', 'localsearch-3', 'tinysparql', 'pipewire', 'pipewire-pulse', 'wireplumber',
                        'dconf-service', 'nautilus', 'ding')
            or name.startswith(('gvfs', 'localsearch', 'tracker'))):
        return 'gnome'
    return None


def ram_groups():
    g = collections.defaultdict(list)
    for pid in os.listdir('/proc'):
        if not pid.isdigit():
            continue
        try:
            st = open(f'/proc/{pid}/status').read()
            name = re.search(r'^Name:\s*(.*)$', st, re.M).group(1)
            m = re.search(r'^VmRSS:\s*(\d+)', st, re.M)
            rss = int(m.group(1)) / 1024 if m else 0
            cmd = open(f'/proc/{pid}/cmdline', 'rb').read().replace(b'\0', b' ').decode(errors='replace')[:90]
        except (OSError, AttributeError):
            continue
        k = group(name, cmd)
        if k:
            g[k].append(rss)
    for k in ('claude_code', 'electron', 'chrome', 'gnome'):
        v = g.get(k, [])
        ctx = {'procs': len(v)}
        if k == 'claude_code' and v:
            ctx['per_process_mib'] = f'{min(v):.0f}–{max(v):.0f}'
        emit(f'ram.{k}_mib', sum(v), ctx)


if __name__ == '__main__':
    try:
        swap_state()
    except Exception as e:
        fail(['mem.swapfile_gib', 'mem.zram_gib', 'mem.swap_used_gib', 'mem.available_gib', 'mem.zram_ratio'], e)
    ram_groups()
