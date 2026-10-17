//! Thin wrappers for the few Linux calls the standard library lacks: locks, atomic renames, extended attributes,
//! whiteouts, the session keyring and deferred interrupts. Path calls never follow a final symlink.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn check(ret: libc::c_int) -> io::Result<()> {
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Takes an advisory lock on `file`; it is released when the file is closed or the process exits.
/// With `wait` false, a lock held elsewhere returns `WouldBlock`.
pub fn flock(file: &File, exclusive: bool, wait: bool) -> io::Result<()> {
    let mut op = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    if !wait {
        op |= libc::LOCK_NB;
    }
    loop {
        let ret = unsafe { libc::flock(file.as_raw_fd(), op) };
        match check(ret) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

fn renameat2(from: &Path, to: &Path, flags: libc::c_uint) -> io::Result<()> {
    let (f, t) = (cpath(from)?, cpath(to)?);
    check(unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            f.as_ptr(),
            libc::AT_FDCWD,
            t.as_ptr(),
            flags,
        )
    })
}

/// Swaps two paths in one step: neither is ever missing. Both must exist on the same filesystem.
pub fn rename_exchange(a: &Path, b: &Path) -> io::Result<()> {
    renameat2(a, b, libc::RENAME_EXCHANGE)
}

/// Renames `from` to `to`, failing with `AlreadyExists` instead of replacing anything at `to`.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    renameat2(from, to, libc::RENAME_NOREPLACE)
}

/// Reads one extended attribute; `None` when the path does not have it.
pub fn get_xattr(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    let mut buf = vec![0u8; 256];
    loop {
        let len =
            unsafe { libc::lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if len >= 0 {
            buf.truncate(len as usize);
            return Ok(Some(buf));
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENODATA) | Some(libc::ENOTSUP) => return Ok(None),
            Some(libc::ERANGE) => buf.resize(buf.len() * 4, 0),
            _ => return Err(err),
        }
    }
}

/// Lists the names of a path's extended attributes.
pub fn list_xattrs(path: &Path) -> io::Result<Vec<String>> {
    let p = cpath(path)?;
    let len = unsafe { libc::llistxattr(p.as_ptr(), std::ptr::null_mut(), 0) };
    if len < 0 {
        let err = io::Error::last_os_error();
        return if err.raw_os_error() == Some(libc::ENOTSUP) {
            Ok(Vec::new())
        } else {
            Err(err)
        };
    }
    let mut buf = vec![0u8; len as usize];
    let len = unsafe { libc::llistxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if len < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(len as usize);
    Ok(buf
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

pub fn set_xattr(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    check(unsafe {
        libc::lsetxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    })
}

pub fn remove_xattr(path: &Path, name: &str) -> io::Result<()> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    check(unsafe { libc::lremovexattr(p.as_ptr(), n.as_ptr()) })
}

/// Creates an overlayfs whiteout: a character device numbered 0:0. Linux has allowed this without privileges
/// since 5.8, so a world can be restored to exactly what it held.
pub fn make_whiteout(path: &Path) -> io::Result<()> {
    let p = cpath(path)?;
    check(unsafe { libc::mknod(p.as_ptr(), libc::S_IFCHR, libc::makedev(0, 0)) })
}

/// True for an overlayfs whiteout: a character device numbered 0:0.
pub fn is_whiteout(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.file_type().is_char_device() && meta.rdev() == 0
}

/// Gives the calling process a new, empty session keyring, so a command started from it cannot use the keys of
/// the person's login session. Safe between fork and exec.
pub fn new_session_keyring() -> io::Result<()> {
    const KEYCTL_JOIN_SESSION_KEYRING: libc::c_long = 1;
    let ret = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_JOIN_SESSION_KEYRING,
            std::ptr::null::<libc::c_char>(),
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
const DEFERRED: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

extern "C" fn note_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// While held, Ctrl-C, a closed terminal or `kill` only raise a flag that `interrupted` reads, so keep and undo
/// stop between two steps with their journal in order, never halfway through one. Signals ignored before (as
/// under `nohup`) stay ignored.
pub struct Interrupts {
    old: [libc::sighandler_t; 3],
}

pub fn defer_interrupts() -> Interrupts {
    INTERRUPTED.store(false, Ordering::SeqCst);
    let handler = note_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t;
    let old = DEFERRED.map(|sig| unsafe {
        let old = libc::signal(sig, handler);
        if old == libc::SIG_IGN {
            libc::signal(sig, libc::SIG_IGN);
        }
        old
    });
    Interrupts { old }
}

impl Interrupts {
    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::SeqCst)
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        for (sig, old) in DEFERRED.into_iter().zip(self.old) {
            unsafe { libc::signal(sig, old) };
        }
    }
}
