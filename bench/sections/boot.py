"""5. Boot: systemd-analyze totals, critical chain and blame; initramfs contents; modules on disk.

The initramfs is read without unpacking it to disk: the uncompressed early cpio archives (newc headers) are walked
in place, then the compressed main archive is piped through the zstd CLI (gzip, xz and lz4 are tried by magic) and
walked the same way. Every file is classed as microcode, firmware, kernel module or other (udev, plymouth, ...);
module names are compared with /proc/modules.
"""
import collections
import fnmatch
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'lib'))
from common import emit, env, fail  # noqa: E402

BLAME = {'ua-timer.service': 'boot.blame.ua_timer', 'apt-daily-upgrade.service': 'boot.blame.apt_daily_upgrade',
         'fwupd-refresh.service': 'boot.blame.fwupd_refresh', 'man-db.service': 'boot.blame.man_db'}


def span_s(text):
    """systemd time span ('1min 2.345s', '862ms', '11.263s', '2h 1min') -> seconds."""
    total, units = 0.0, {'h': 3600, 'min': 60, 's': 1, 'ms': 1e-3, 'us': 1e-6, 'µs': 1e-6}
    for num, unit in re.findall(r'([\d.]+)\s*(h|min|ms|us|µs|s)\b', text):
        total += float(num) * units[unit]
    return total


def analyze():
    sa = shutil.which('systemd-analyze')
    if not sa:
        return fail('boot.total_s', 'systemd-analyze not found')
    out = subprocess.run([sa], capture_output=True, text=True).stdout
    m = re.search(r'Startup finished in (.*?)= ([^\n]+)', out)
    if not m:
        return fail('boot.total_s', 'boot not finished or not reported: ' + out.strip()[:200])
    boot_ctx = {'booted': subprocess.run(['uptime', '-s'], capture_output=True, text=True).stdout.strip()}
    phases = {ph: span_s(t) for t, ph in re.findall(r'([\d.]+(?:min|ms|s)(?: [\d.]+(?:ms|s))?) \((\w+)\)', m.group(1))}
    emit('boot.total_s', span_s(m.group(2)), boot_ctx)
    for ph in ('firmware', 'loader', 'kernel', 'initrd', 'userspace'):
        if ph in phases:
            emit(f'boot.{ph}_s', phases[ph], boot_ctx)
    if 'firmware' in phases and 'loader' in phases:
        emit('boot.kexec_skips_s', phases['firmware'] + phases['loader'], boot_ctx)
    g = re.search(r'graphical.target reached after (.+?) in userspace', out)
    if g:
        emit('boot.graphical_s', span_s(g.group(1)), boot_ctx)

    chain = subprocess.run([sa, 'critical-chain'], capture_output=True, text=True).stdout
    items = []
    for line in chain.splitlines():
        cm = re.search(r'([\w@.\\-]+\.(?:service|target|mount|socket|device|swap))\s+@\S+(?:\s+\+(\S+))?', line)
        if cm and cm.group(2):
            items.append((cm.group(1), span_s(cm.group(2))))
    pq = [t for u, t in items if u == 'plymouth-quit-wait.service']
    if pq:
        emit('boot.chain.plymouth_quit_wait_s', pq[0], dict(boot_ctx, on_critical_chain=True))
    else:
        fail('boot.chain.plymouth_quit_wait_s', 'plymouth-quit-wait.service is not on the critical chain')
    if items:
        u, t = max(items, key=lambda x: x[1])
        emit('boot.chain.slowest', f'{u} +{t:.3f}s', boot_ctx)

    blame = {}
    for line in subprocess.run([sa, 'blame'], capture_output=True, text=True).stdout.splitlines():
        bm = re.match(r'\s*(.+?)\s+(\S+\.(?:service|mount|device|swap|socket|timer|target))\s*$', line)
        if bm:
            blame[bm.group(2)] = span_s(bm.group(1))
    for unit, key in BLAME.items():
        if unit in blame:
            emit(key, blame[unit], dict(boot_ctx, last_run=last_run(unit)))
        else:
            fail(key, f'{unit} did not run at this boot')
    snapd = [blame[u] for u in ('snapd.service', 'snapd.seeded.service') if u in blame]
    if snapd:
        emit('boot.blame.snapd', sum(snapd), dict(boot_ctx, units='snapd + snapd.seeded'))
    else:
        fail('boot.blame.snapd', 'snapd did not run at this boot')


def newc(d, off, files, regular_only):
    """Walk one newc cpio archive at d[off:]; append (name, size); return the offset after its trailer.
    The M run counted every entry with data in the early archives (`cpio` newc walk) but every regular file, empty
    ones included, in the main archive (`cpio -itv`); regular_only keeps those counts comparable."""
    while True:
        h = d[off:off + 110]
        if h[:6] not in (b'070701', b'070702'):
            raise ValueError(f'bad cpio header at {off}')
        fs, ns = int(h[54:62], 16), int(h[94:102], 16)
        name = d[off + 110:off + 110 + ns - 1].decode(errors='replace')
        off = (off + 110 + ns + 3) & ~3
        off = (off + fs + 3) & ~3
        if name == 'TRAILER!!!':
            return off
        if (int(h[14:22], 16) & 0o170000 == 0o100000) if regular_only else fs:
            files.append((name, fs))


def last_run(unit):
    """When the unit last started: blame reports a unit's latest run, and timers rerun these after boot."""
    out = subprocess.run(['systemctl', 'show', '-p', 'ExecMainStartTimestamp', '--value', unit],
                         capture_output=True, text=True).stdout  # 'Fri 2026-10-16 17:21:01 -03'
    m = re.search(r'\d{4}-\d\d-\d\d \d\d:\d\d:\d\d', out)
    return m.group(0) if m else None


DECOMP = [(b'\x28\xb5\x2f\xfd', ['zstd', '-dc']), (b'\x1f\x8b', ['gzip', '-dc']),
          (b'\xfd7zXZ\x00', ['xz', '-dc']), (b'\x02\x21\x4c\x18', ['lz4', '-dc'])]


def initramfs():
    keys = ['boot.initrd_mb', 'boot.initrd_modules', 'boot.initrd_modules_loaded', 'boot.initrd_firmware_mb',
            'boot.initrd_microcode_mb', 'boot.initrd_userspace_mb']
    path = env('INITRD', '/boot/initrd.img')
    try:
        d = open(path, 'rb').read()
    except OSError as e:
        return fail(keys, e)
    ctx = {'file': os.path.realpath(path), 'bytes': len(d)}
    files, off = [], 0
    while off < len(d) and d[off:off + 6] in (b'070701', b'070702'):
        off = newc(d, off, files, False)
        while off < len(d) and d[off] == 0:
            off += 1
    early_bytes = off
    main_note = None
    if off < len(d):
        tool = next((cmd for magic, cmd in DECOMP if d[off:off + len(magic)] == magic), None)
        if tool and shutil.which(tool[0]):
            raw = subprocess.run(tool, input=d[off:], capture_output=True).stdout
            moff = 0
            while moff < len(raw) and raw[moff:moff + 6] in (b'070701', b'070702'):
                moff = newc(raw, moff, files, True)
                while moff < len(raw) and raw[moff] == 0:
                    moff += 1
            ctx['main_unpacked_mb'] = round(len(raw) / 1e6, 1)
        else:
            main_note = f'main archive format {d[off:off + 4].hex()} not readable here'
    tot, cnt, mods = collections.Counter(), collections.Counter(), {}
    for name, fs in files:
        if '/microcode/' in name:
            k = 'microcode'
        elif '/firmware/' in name:
            k = 'firmware'
        elif re.search(r'\.ko(\.zst|\.xz|\.gz)?$', name):
            k = 'modules'
            mods[re.sub(r'\.ko(\.zst|\.xz|\.gz)?$', '', name.rsplit('/', 1)[1]).replace('-', '_')] = fs
        else:
            k = 'other'
        tot[k] += fs
        cnt[k] += 1
    loaded = {line.split()[0] for line in open('/proc/modules')}
    used = mods.keys() & loaded
    ctx['early_mb'] = round(early_bytes / 1e6, 1)
    emit('boot.initrd_mb', len(d) / 1e6, ctx)
    emit('boot.initrd_modules', len(mods), {'mb': round(sum(mods.values()) / 1e6, 1)})
    emit('boot.initrd_modules_loaded', len(used), {'mb': round(sum(mods[m] for m in used) / 1e6, 2)})
    emit('boot.initrd_modules_loaded_mb', sum(mods[m] for m in used) / 1e6)
    emit('boot.initrd_firmware_mb', tot['firmware'] / 1e6, {'files': cnt['firmware']})
    emit('boot.initrd_microcode_mb', tot['microcode'] / 1e6, {'files': cnt['microcode']})
    if main_note:
        fail('boot.initrd_userspace_mb', main_note)
    else:
        emit('boot.initrd_userspace_mb', tot['other'] / 1e6, {'files': cnt['other']})


def modules_on_disk():
    rel = os.uname().release
    root = f'/lib/modules/{rel}'
    n = 0
    for dp, dns, fns in os.walk(root):
        n += sum(1 for f in fns if fnmatch.fnmatch(f, '*.ko*'))
    du = subprocess.run(['du', '-sk', root], capture_output=True, text=True).stdout.split()
    loaded = sum(1 for _ in open('/proc/modules'))
    emit('boot.modules_disk_files', n, {'kernel': rel})
    if du:
        emit('boot.modules_disk_mib', int(du[0]) / 1024)
    emit('boot.modules_loaded', loaded, {'share_pct': round(100 * loaded / n, 1) if n else None})
    squash = sum(1 for line in open('/proc/self/mountinfo') if ' - squashfs ' in line)
    emit('boot.squashfs_mounts', squash)


if __name__ == '__main__':
    for fn in (analyze, initramfs, modules_on_disk):
        try:
            fn()
        except Exception as e:
            fail(f'boot.{fn.__name__}', e)
