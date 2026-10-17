//! Errors carry a message written for the person at the terminal, and an exit code.

use std::{fmt, io};

/// Exit code for a mistake in how the command was called.
pub const USAGE: i32 = 2;
/// Exit code for a request Arbor refuses on purpose: a busy world, conflicts, a world already kept.
pub const REFUSED: i32 = 3;
/// Exit code for a command a world cannot find, as shells give for a command they cannot find.
pub const NOT_FOUND: i32 = 127;
/// Exit code for a keep or undo stopped by Ctrl-C, a closed terminal or `kill`, as shells report SIGINT.
pub const STOPPED: i32 = 130;

#[derive(Debug)]
pub struct Error {
    pub msg: String,
    pub code: i32,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error {
            msg: msg.into(),
            code: 1,
        }
    }

    pub fn usage(msg: impl Into<String>) -> Self {
        Error {
            msg: msg.into(),
            code: USAGE,
        }
    }

    pub fn refused(msg: impl Into<String>) -> Self {
        Error {
            msg: msg.into(),
            code: REFUSED,
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Error {
            msg: msg.into(),
            code: NOT_FOUND,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::new(e.to_string())
    }
}

impl From<lexopt::Error> for Error {
    fn from(e: lexopt::Error) -> Self {
        Error::usage(e.to_string())
    }
}

/// Adds what Arbor was doing to an error, so "No such file or directory" says which file.
pub trait Context<T> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| Error::new(format!("{}: {e}", what())))
    }
}
