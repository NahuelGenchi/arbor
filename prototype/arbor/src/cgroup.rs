//! Cells in their own cgroup, without root. `arbor serve` is a systemd user service that holds a delegated subtree,
//! `arbor.slice/arbor.service`, and does nothing else. arbor itself makes each cell's cgroup in it, writes the
//! limits, starts the cell's first process inside it, reads what the cell used, and removes it: no daemon sits
//! between a command and its cell.
//!
//! `arbor.slice` sits beside the desktop's `app.slice`, so turning on the CPU controller for cells changes nothing
//! for the desktop's apps, and under contention the cells together share the CPU with them instead of starving
//! them.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{Context, Error, Result};

const ROOT: &str = "/sys/fs/cgroup";
const CONTROLLERS: [&str; 3] = ["cpu", "memory", "pids"];
/// The CPU controller's accounting period, in microseconds.
const PERIOD_US: u64 = 100_000;
/// Set on arbor when it runs itself again in a user scope, so it does so only once.
pub const SCOPE_MARK: &str = "ARBOR_SCOPE";

/// The cgroup path of this process (`0::/…` in `/proc/self/cgroup`).
fn own_cgroup() -> Option<String> {
    let text = fs::read_to_string("/proc/self/cgroup").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::to_string)
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// True when this process sits under the user's systemd manager. Only then may it move a child into a cell's
/// cgroup: the kernel requires write access to the closest cgroup holding both.
fn inside_user_manager() -> bool {
    let marker = format!("/user@{}.service/", uid());
    own_cgroup().is_some_and(|c| c.contains(&marker))
}

/// True when the user's systemd manager can be asked to start `arbor serve` or a scope.
fn user_manager_reachable() -> bool {
    env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| Path::new(&d).join("systemd/private").exists())
}

/// `ARBOR_NO_CGROUP=1` runs cells as they run where there is no user manager: to compare the two, or to rule
/// cgroups out.
fn switched_off() -> bool {
    env::var_os("ARBOR_NO_CGROUP").is_some_and(|v| !v.is_empty())
}

/// The user manager's cgroup folder.
fn user_manager() -> PathBuf {
    let marker = format!("/user@{}.service", uid());
    if let Some(own) = own_cgroup()
        && let Some(at) = own.find(&marker)
    {
        return Path::new(ROOT).join(own[..at + marker.len()].trim_start_matches('/'));
    }
    PathBuf::from(format!(
        "{ROOT}/user.slice/user-{0}.slice/user@{0}.service",
        uid()
    ))
}

/// The folder that holds every cell's cgroup.
pub fn cells_dir() -> PathBuf {
    user_manager().join("arbor.slice/arbor.service/cells")
}

/// Why a cell cannot have a cgroup in this session, or `None` when it can.
pub fn unavailable() -> Option<&'static str> {
    if switched_off() {
        Some("ARBOR_NO_CGROUP is set")
    } else if !inside_user_manager() {
        Some("this session runs outside the systemd user manager")
    } else if !cells_dir().join("cgroup.procs").exists() && !user_manager_reachable() {
        Some("the systemd user manager cannot be reached from this session")
    } else {
        None
    }
}

/// From outside the user's systemd manager, as in an SSH login, a cell cannot start in a cgroup of its own, so
/// arbor runs itself again in a scope the manager owns (systemd asks the system manager to move it there).
/// Returns only when it does not. Only commands with limits that need a cgroup do this: a plain command runs
/// as before rather than depend on it.
pub fn enter_user_manager() {
    if switched_off()
        || inside_user_manager()
        || !user_manager_reachable()
        || env::var_os(SCOPE_MARK).is_some()
    {
        return;
    }
    let Ok(exe) = env::current_exe() else { return };
    // `exec` returns only when systemd-run cannot start; the cell then runs without a cgroup.
    let _ = Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "--"])
        .arg(exe)
        .args(env::args_os().skip(1))
        .env(SCOPE_MARK, "1")
        .exec();
}

/// Starts `arbor serve` unless it already runs, and returns the cells folder.
pub fn ensure_service() -> Result<PathBuf> {
    let cells = cells_dir();
    if cells.join("cgroup.procs").exists() {
        return Ok(cells);
    }
    let exe = env::current_exe()?;
    let out = Command::new("systemd-run")
        .args([
            "--user",
            "--quiet",
            "--unit=arbor",
            "--slice=arbor.slice",
            "--collect",
            "--service-type=notify",
            "--description=arbor: holds the cgroups of running cells",
            "--property=Delegate=cpu memory pids",
            "--property=DelegateSubgroup=hold",
            "--",
        ])
        .arg(&exe)
        .arg("serve")
        .stdin(Stdio::null())
        .output()
        .ctx(|| "cannot run systemd-run".to_string())?;
    // Another arbor may have started it a moment ago; give it time to say it is ready.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !cells.join("cgroup.procs").exists() {
        if Instant::now() > deadline {
            return Err(Error::new(format!(
                "cannot start `arbor serve` ({}); `systemctl --user stop arbor` resets it",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(cells)
}

/// Tells systemd the service is ready (`sd_notify`), when it asked to be told.
fn notify(state: &str) -> io::Result<()> {
    let Some(path) = env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let sock = UnixDatagram::unbound()?;
    let addr = match path.as_bytes().strip_prefix(b"@") {
        Some(name) => SocketAddr::from_abstract_name(name)?,
        None => SocketAddr::from_pathname(&path)?,
    };
    sock.send_to_addr(state.as_bytes(), &addr)?;
    Ok(())
}

/// `arbor serve`, run by systemd as `arbor.service` inside its `hold` subgroup: turns on the controllers for the
/// unit's subtree, makes `cells`, tells systemd it is ready, then sleeps until it is stopped.
pub fn serve() -> Result<i32> {
    let own = own_cgroup().ok_or_else(|| Error::new("cannot read /proc/self/cgroup"))?;
    let hold = Path::new(ROOT).join(own.trim_start_matches('/'));
    let unit = hold
        .parent()
        .filter(|_| hold.ends_with("hold"))
        .ok_or_else(|| {
            Error::usage("`arbor serve` is started by arbor itself, through systemd-run")
        })?;
    let available = fs::read_to_string(unit.join("cgroup.controllers"))?;
    let enable = CONTROLLERS
        .iter()
        .filter(|c| available.split_whitespace().any(|a| a == **c))
        .map(|c| format!("+{c}"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(unit.join("cgroup.subtree_control"), &enable)
        .ctx(|| "cannot turn on the controllers for cells".to_string())?;
    let cells = unit.join("cells");
    match fs::create_dir(&cells) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.into()),
        _ => {}
    }
    fs::write(cells.join("cgroup.subtree_control"), &enable)?;
    notify("READY=1")?;
    loop {
        unsafe { libc::pause() };
    }
}

/// What a cell may use. `None` is no limit.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Memory in bytes, page cache and the cell's `/tmp` included; swap is off.
    pub mem: Option<u64>,
    /// CPUs: 0.5 is half of one, 2 is two.
    pub cpu: Option<f64>,
    /// Processes and threads at once.
    pub pids: Option<u64>,
    /// Wall time.
    pub time: Option<Duration>,
}

/// The command-line flags that set limits.
pub const FLAGS: [&str; 4] = ["mem", "cpu", "pids", "time"];

impl Limits {
    /// Sets the limit a command-line flag names: `mem`, `cpu`, `pids` or `time`.
    pub fn set(&mut self, flag: &str, value: &str) -> Result<()> {
        match flag {
            "mem" => self.mem = Some(parse_size(value)?),
            "cpu" => self.cpu = Some(parse_cpus(value)?),
            "pids" => self.pids = Some(parse_count(value)?),
            "time" => self.time = Some(parse_duration(value)?),
            _ => return Err(Error::usage(format!("there is no limit --{flag}"))),
        }
        Ok(())
    }

    /// True when a limit only a cgroup can hold is set.
    pub fn need_cgroup(&self) -> bool {
        self.mem.is_some() || self.cpu.is_some() || self.pids.is_some()
    }

    /// `mem 512MiB, cpu 0.5, pids 64, time 10m`, or empty.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(m) = self.mem {
            parts.push(format!("mem {}", size(m)));
        }
        if let Some(c) = self.cpu {
            parts.push(format!("cpu {c}"));
        }
        if let Some(p) = self.pids {
            parts.push(format!("pids {p}"));
        }
        if let Some(t) = self.time {
            parts.push(format!("time {}", duration(t)));
        }
        parts.join(", ")
    }
}

/// Reads `512M`, `2G`, `1.5GiB` or a plain number of bytes. K, M, G and T are powers of 1024.
pub fn parse_size(text: &str) -> Result<u64> {
    let t = text.trim();
    let t = t
        .strip_suffix("iB")
        .or_else(|| t.strip_suffix('B'))
        .unwrap_or(t);
    let (num, unit) = match t.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((at, _)) => (&t[..at], &t[at..]),
        None => (t, ""),
    };
    let scale: u64 = match unit.to_ascii_uppercase().as_str() {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => 0,
    };
    match num.parse::<f64>() {
        Ok(n) if scale > 0 && n >= 1.0 && n.is_finite() => Ok((n * scale as f64) as u64),
        _ => Err(Error::usage(format!(
            "'{text}' is not a size: use for example 512M or 2G"
        ))),
    }
}

/// Reads a number of CPUs, such as `0.5` or `2`.
pub fn parse_cpus(text: &str) -> Result<f64> {
    match text.trim().parse::<f64>() {
        Ok(n) if (0.01..=4096.0).contains(&n) => Ok(n),
        _ => Err(Error::usage(format!(
            "'{text}' is not a number of CPUs: use for example 0.5 or 2"
        ))),
    }
}

/// Reads `90s`, `10m`, `2h`, `500ms` or a plain number of seconds.
pub fn parse_duration(text: &str) -> Result<Duration> {
    let t = text.trim();
    let (num, scale) = if let Some(n) = t.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60.0)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3600.0)
    } else {
        (t, 1.0)
    };
    match num.parse::<f64>() {
        Ok(n) if n > 0.0 && (n * scale) < 1e9 => Ok(Duration::from_secs_f64(n * scale)),
        _ => Err(Error::usage(format!(
            "'{text}' is not a duration: use for example 90s, 10m or 2h"
        ))),
    }
}

/// Reads a number of processes, at least 1.
pub fn parse_count(text: &str) -> Result<u64> {
    match text.trim().parse::<u64>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(Error::usage(format!(
            "'{text}' is not a number of processes: use for example 64"
        ))),
    }
}

/// `512MiB`, `1.5GiB`, `100KiB`.
pub fn size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GiB", 1 << 30), ("MiB", 1 << 20), ("KiB", 1 << 10)];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            let n = bytes as f64 / scale as f64;
            return if n >= 100.0 || (n * 10.0).round() % 10.0 == 0.0 {
                format!("{n:.0}{unit}")
            } else {
                format!("{n:.1}{unit}")
            };
        }
    }
    format!("{bytes}B")
}

/// `250ms`, `90s`, `10m`, `1h 30m`.
pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0 => format!("{}ms", d.as_millis()),
        1..60 => format!("{s}s"),
        60..3600 if s.is_multiple_of(60) => format!("{}m", s / 60),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ if s.is_multiple_of(3600) => format!("{}h", s / 3600),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// A new cell's name: 8 random hex digits.
pub fn new_id() -> String {
    let mut b = [0u8; 4];
    unsafe { libc::getrandom(b.as_mut_ptr().cast(), b.len(), 0) };
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What a cell used, from its cgroup.
#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub cpu_us: u64,
    /// Peak memory in bytes (Linux 5.19 and later).
    pub mem_peak: Option<u64>,
    /// Peak number of processes and threads, where the kernel keeps it.
    pub pids_peak: Option<u64>,
    /// Processes the kernel killed because the cell ran out of memory.
    pub oom_kills: u64,
    /// Times a new process was refused because of the process limit.
    pub pids_refused: u64,
}

/// A cell's cgroup, killed and removed when dropped.
pub struct Group {
    id: String,
    dir: PathBuf,
    /// The folder, open: a new process can start in it (`sys::fork_into`).
    folder: File,
    /// Its `cgroup.procs`, open: a process can move itself in where it could not start there.
    procs: File,
}

impl Group {
    /// Makes a cell's cgroup in `cells` with its limits written.
    pub fn create(cells: &Path, limits: &Limits) -> Result<Group> {
        let enabled = fs::read_to_string(cells.join("cgroup.subtree_control")).unwrap_or_default();
        let wanted = [
            (limits.mem.is_some(), "memory"),
            (limits.cpu.is_some(), "cpu"),
            (limits.pids.is_some(), "pids"),
        ];
        for (asked, controller) in wanted {
            if asked && !enabled.split_whitespace().any(|c| c == controller) {
                return Err(Error::refused(format!(
                    "this computer does not let users limit {controller}: the systemd user manager has no \
                     {controller} controller to hand on"
                )));
            }
        }
        let (id, dir) = loop {
            let id = new_id();
            let dir = cells.join(&id);
            match fs::create_dir(&dir) {
                Ok(()) => break (id, dir),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).ctx(|| format!("cannot create {}", dir.display())),
            }
        };
        let opened = File::open(&dir).and_then(|folder| {
            let procs = OpenOptions::new()
                .write(true)
                .open(dir.join("cgroup.procs"))?;
            Ok((folder, procs))
        });
        let (folder, procs) = match opened {
            Ok(files) => files,
            Err(e) => {
                remove_dir(&dir);
                return Err(e.into());
            }
        };
        let group = Group {
            id,
            dir,
            folder,
            procs,
        };
        let set = |file: &str, value: String| {
            fs::write(group.dir.join(file), value).ctx(|| format!("cannot set the cell's {file}"))
        };
        if let Some(mem) = limits.mem {
            set("memory.max", mem.to_string())?;
            // Without swap accounting there is no such file, and no swap to turn off.
            match fs::write(group.dir.join("memory.swap.max"), "0") {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
            set("memory.oom.group", "1".into())?;
        }
        if let Some(cpu) = limits.cpu {
            let quota = ((cpu * PERIOD_US as f64) as u64).max(1000);
            set("cpu.max", format!("{quota} {PERIOD_US}"))?;
        }
        if let Some(pids) = limits.pids {
            set("pids.max", pids.to_string())?;
        }
        Ok(group)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The open folder, to start the cell's first process in it.
    pub fn folder(&self) -> BorrowedFd<'_> {
        self.folder.as_fd()
    }

    /// The open `cgroup.procs`, for the cell's first process to move itself in with `join` where it could not
    /// start there.
    pub fn procs(&self) -> RawFd {
        self.procs.as_raw_fd()
    }

    /// Kills every process in the cell at once.
    pub fn kill(&self) {
        kill_dir(&self.dir);
    }

    pub fn usage(&self) -> Usage {
        read_usage(&self.dir)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.kill();
        remove_dir(&self.dir);
    }
}

/// Moves the calling process into the cgroup whose `cgroup.procs` is open as `fd`. Safe between fork and exec.
pub fn join(fd: RawFd) -> io::Result<()> {
    if unsafe { libc::write(fd, b"0".as_ptr().cast(), 1) } == 1 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// One `key value` line of a cgroup file.
fn field(dir: &Path, file: &str, key: &str) -> Option<u64> {
    fs::read_to_string(dir.join(file)).ok().and_then(|t| {
        t.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok())
    })
}

fn number(dir: &Path, file: &str) -> Option<u64> {
    fs::read_to_string(dir.join(file))
        .ok()
        .and_then(|t| t.trim().parse().ok())
}

pub fn read_usage(dir: &Path) -> Usage {
    Usage {
        cpu_us: field(dir, "cpu.stat", "usage_usec").unwrap_or(0),
        mem_peak: number(dir, "memory.peak"),
        pids_peak: number(dir, "pids.peak"),
        oom_kills: field(dir, "memory.events", "oom_kill").unwrap_or(0),
        pids_refused: field(dir, "pids.events", "max").unwrap_or(0),
    }
}

/// Memory in use now by the cell whose cgroup is `dir`.
pub fn memory_now(dir: &Path) -> Option<u64> {
    number(dir, "memory.current")
}

/// Kills every process in the cgroup `dir`.
pub fn kill_dir(dir: &Path) {
    let _ = fs::write(dir.join("cgroup.kill"), "1");
}

/// Removes an empty cgroup. Its last processes may still be on their way out, so it tries for a moment.
pub fn remove_dir(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Err(e) = fs::remove_dir(dir) {
        if e.kind() == io::ErrorKind::NotFound || Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sizes_cpus_counts_and_durations() {
        assert_eq!(parse_size("512M").unwrap(), 512 << 20);
        assert_eq!(parse_size("2G").unwrap(), 2 << 30);
        assert_eq!(parse_size("1.5GiB").unwrap(), 3 << 29);
        assert_eq!(parse_size("100k").unwrap(), 100 << 10);
        assert_eq!(parse_size("4096").unwrap(), 4096);
        for bad in ["", "M", "-1G", "0", "12X", "lots"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_cpus("0.5").unwrap(), 0.5);
        assert!(parse_cpus("0").is_err() && parse_cpus("x").is_err());
        assert_eq!(parse_count("64").unwrap(), 64);
        assert!(parse_count("0").is_err() && parse_count("-3").is_err());
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_duration("1.5h").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("7").unwrap(), Duration::from_secs(7));
        for bad in ["soon", "-5s", "0", "1e99h"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn describes_limits_and_amounts() {
        let limits = Limits {
            mem: Some(512 << 20),
            cpu: Some(0.5),
            pids: Some(64),
            time: Some(Duration::from_secs(600)),
        };
        assert_eq!(limits.describe(), "mem 512MiB, cpu 0.5, pids 64, time 10m");
        assert!(limits.need_cgroup());
        assert!(!Limits::default().need_cgroup());
        assert_eq!(Limits::default().describe(), "");
        assert_eq!(size(3 << 29), "1.5GiB");
        assert_eq!(size(70 << 20), "70MiB");
        assert_eq!(size((1 << 20) - 1), "1024KiB");
        assert_eq!(size(512), "512B");
        assert_eq!(duration(Duration::from_millis(250)), "250ms");
        assert_eq!(duration(Duration::from_secs(5400)), "1h 30m");
        assert_eq!(duration(Duration::from_secs(75)), "1m 15s");
        assert_eq!(new_id().len(), 8);
    }
}
