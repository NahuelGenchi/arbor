"""Machine stamp and starting state for the report header (JSON on stdout)."""
import json
import os
import re
import socket
import subprocess
import time


def read(path, default=None):
    try:
        return open(path).read().strip()
    except OSError:
        return default


def cpus(path):
    """Count the CPUs in a list like '0-15,20'."""
    n = 0
    for part in (read(path) or '').split(','):
        if '-' in part:
            a, b = part.split('-')
            n += int(b) - int(a) + 1
        elif part:
            n += 1
    return n


def version(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        m = re.search(r'\d+\.\d+(\.\d+)?', out.stdout + out.stderr)
        return m.group(0) if m else '?'
    except (OSError, subprocess.TimeoutExpired):
        return None


def cpu_ticks():
    """CPU time every process has used so far, as {pid: (name, clock ticks)}."""
    d = {}
    for pid in os.listdir('/proc'):
        if pid.isdigit():
            try:
                st = open(f'/proc/{pid}/stat').read()
                f = st[st.rindex(')') + 2:].split()
                d[pid] = (st[st.index('(') + 1:st.rindex(')')], int(f[11]) + int(f[12]))
            except (OSError, ValueError):
                pass
    return d


def busiest(a, b, seconds, floor=20):
    """The three processes that used the most CPU between two cpu_ticks() readings, as 'name 800%' (percent of one
    core, summed by name), those at floor percent or more."""
    hz = os.sysconf('SC_CLK_TCK')
    use = {}
    for pid, (name, t) in b.items():
        if pid in a and a[pid][0] == name:
            use[name] = use.get(name, 0) + (t - a[pid][1]) / hz / seconds * 100
    top = sorted(use.items(), key=lambda x: -x[1])[:3]
    return ', '.join(f'{n} {u:.0f}%' for n, u in top if u >= floor)


def busy(seconds=1.0):
    """Processes using the most CPU over a short window, as 'name 800%' (percent of one core)."""
    a = cpu_ticks()
    t0 = time.time()
    time.sleep(seconds)
    return busiest(a, cpu_ticks(), time.time() - t0)


def main():
    mi = {k: int(v.split()[0]) * 1024 for k, v in (line.split(':', 1) for line in open('/proc/meminfo'))}
    model = re.search(r'^model name\s*:\s*(.+)$', read('/proc/cpuinfo', ''), re.M)
    threads = os.cpu_count()
    p_threads, e_cores = cpus('/sys/devices/cpu_core/cpus'), cpus('/sys/devices/cpu_atom/cpus')
    cores = f'{p_threads // 2}P+{e_cores}E cores, {threads} threads' if p_threads and e_cores else f'{threads} threads'
    osr = dict(re.findall(r'^(\w+)="?([^"\n]*)"?$', read('/etc/os-release', ''), re.M))
    dmi = lambda f: read(f'/sys/class/dmi/id/{f}', '?')  # noqa: E731
    home = os.path.expanduser('~')
    params = {k: os.environ[k].replace(home, '~') for k in (
        'PROJECTS_DIR', 'NAME_SEARCH_ROOT', 'NAME_QUERY', 'CODE_ROOT', 'CODE_QUERIES', 'DEDUP_ROOT', 'FORK_TREE',
        'GIT_LOG_REPO', 'GIT_LOG_REF', 'TRANSCRIPTS_DIR', 'HEAP_JSONL', 'HEAP_JSONL_BYTES', 'O200K_FILE',
        'DOCKER_IMAGE', 'LISTING_DIR', 'INITRD', 'ARBOR_BIN', 'SCREEN_STILL_S', 'FRAMES_S', 'FRAMES_CELLS',
        'FRAMES_SPIKE_MIB', 'TOOLKITS', 'TOOLKITS_PREBUILT', 'TOOLKITS_LAUNCHES', 'TOOLKITS_STILL_S',
        'TOOLKITS_SETTLE_S', 'FLUTTER', 'FLUTTER_PUB_CACHE') if os.environ.get(k)}
    tools = {'python': version(['python3', '--version']), 'node': version(['node', '--version']),
             'rustc': version(['rustc', '--version']), 'gcc': version(['gcc', '--version']),
             'rg': version(['rg', '--version']), 'docker': version(['docker', '--version']),
             'bwrap': version(['bwrap', '--version']), 'zstd': version(['zstd', '--version'])}
    print(json.dumps({
        'date': time.strftime('%Y-%m-%d %H:%M %Z'),
        'host': socket.gethostname(),
        'cpu': re.sub(r'\s+', ' ', model.group(1)).replace('(R)', '').replace('(TM)', '') if model else '?',
        'cores': cores,
        'ram_gib': round(mi['MemTotal'] / 2**30, 1),
        'board': f"{dmi('board_vendor')} {dmi('board_name')}",
        'bios': dmi('bios_version'),
        'os': osr.get('PRETTY_NAME', '?'),
        'kernel': os.uname().release,
        'booted': subprocess.run(['uptime', '-s'], capture_output=True, text=True).stdout.strip(),
        'governor': read('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor', '?'),
        'epp': read('/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference'),
        'load1': float(read('/proc/loadavg', '0').split()[0]),
        'busy': busy(),
        'mem_available_gib': round(mi['MemAvailable'] / 2**30, 1),
        'swap_used_gib': round((mi['SwapTotal'] - mi['SwapFree']) / 2**30, 1),
        'tools': ', '.join(f'{k} {v}' for k, v in tools.items() if v),
        'params': params,
    }))


if __name__ == '__main__':
    main()
