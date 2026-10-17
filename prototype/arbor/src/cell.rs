//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process: the computer read-only, the world's overlay mounted over its base folder's own path so tools
//! see the usual paths, a throwaway layer over the rest of the home folder (writes there vanish at exit), Arbor's
//! own state and the session's sockets hidden, and no network unless asked for. Sealing the home folder, keys and
//! limits come in M3.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Context, Error, Result};
use crate::paths;
use crate::world::{State, World};

pub struct Options {
    /// Share the computer's network with the cell.
    pub net: bool,
    /// The command will talk to the person at the terminal.
    pub interactive: bool,
}

/// Finds `name` on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub fn bwrap() -> Result<PathBuf> {
    which("bwrap").ok_or_else(|| {
        Error::new("Arbor needs bubblewrap (the `bwrap` command) to run commands in a world; on Ubuntu: sudo apt install bubblewrap")
    })
}

/// TIOCSTI lets a process type into its terminal. Linux 6.2+ can switch it off for everyone; where it is still
/// on, the cell gets a new session so it cannot type into the person's terminal.
fn tiocsti_possible() -> bool {
    fs::read_to_string("/proc/sys/dev/tty/legacy_tiocsti")
        .map(|v| v.trim() != "0")
        .unwrap_or(true)
}

/// The `bwrap` arguments for a cell in `world` that runs `cmd` in `cwd`.
pub fn bwrap_args(
    world: &World,
    cmd: &[OsString],
    cwd: &Path,
    opts: &Options,
) -> Result<Vec<OsString>> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |items: &[&dyn AsRef<std::ffi::OsStr>]| {
        a.extend(items.iter().map(|s| s.as_ref().to_os_string()));
    };
    push(&[
        &"--ro-bind",
        &"/",
        &"/",
        &"--dev",
        &"/dev",
        &"--proc",
        &"/proc",
        &"--tmpfs",
        &"/tmp",
    ]);
    // Writes anywhere else in the home folder land in a layer that is thrown away when the cell ends.
    if let Some(home) = paths::home().and_then(|h| fs::canonicalize(h).ok())
        && home.is_dir()
        && !paths::contains(&world.base, &home)
    {
        push(&[&"--overlay-src", &home, &"--tmp-overlay", &home]);
    }
    push(&[
        &"--overlay-src",
        &world.base,
        &"--overlay",
        &world.up(),
        &world.work(),
        &world.base,
    ]);
    // What the cell must not see: every world's changes and the session's sockets (D-Bus, Wayland, agents).
    let state = fs::canonicalize(paths::state_dir()?)?;
    push(&[&"--tmpfs", &state]);
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    if runtime.is_dir() {
        push(&[&"--tmpfs", &runtime]);
    }
    push(&[&"--unshare-all", &"--die-with-parent"]);
    if opts.net {
        push(&[&"--share-net"]);
    }
    if !opts.interactive || tiocsti_possible() {
        push(&[&"--new-session"]);
    }
    push(&[
        &"--chdir",
        &cwd,
        &"--setenv",
        &"ARBOR_WORLD",
        &world.name.as_str(),
        &"--",
    ]);
    a.extend(cmd.iter().cloned());
    Ok(a)
}

/// Where the command starts: the same folder when the person stands inside the base, else the base itself.
pub fn start_dir(world: &World) -> PathBuf {
    match env::current_dir()
        .ok()
        .and_then(|d| fs::canonicalize(d).ok())
    {
        Some(dir) if paths::contains(&world.base, &dir) => dir,
        _ => world.base.clone(),
    }
}

/// Runs `cmd` in the world and returns its exit code (128 + signal when it was killed).
pub fn run(world: &World, cmd: &[OsString], opts: &Options) -> Result<i32> {
    if world.state != State::Open {
        return Err(Error::refused(format!(
            "world '{}' was kept; run `arbor undo {}` to reopen it",
            world.name, world.name
        )));
    }
    if !world.base.is_dir() {
        return Err(Error::new(format!(
            "the base folder {} is gone",
            paths::tilde(&world.base)
        )));
    }
    let bwrap = bwrap()?;
    let _lock = world.lock()?;
    let args = bwrap_args(world, cmd, &start_dir(world), opts)?;
    let mut child = Command::new(bwrap)
        .args(&args)
        .spawn()
        .ctx(|| "cannot start bwrap".to_string())?;
    // Ctrl-C belongs to the command inside: Arbor waits for it instead of dying first and leaving it behind.
    let (old_int, old_quit) = unsafe {
        (
            libc::signal(libc::SIGINT, libc::SIG_IGN),
            libc::signal(libc::SIGQUIT, libc::SIG_IGN),
        )
    };
    let status = child.wait();
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    let status = status?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}
