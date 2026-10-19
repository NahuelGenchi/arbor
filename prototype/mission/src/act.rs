//! What the window's buttons do. Each one runs the `arbor` command that a person or an agent would type, as a
//! process of its own, so that the outbox's own rules apply and the ledger records the step with you as the actor:
//! the window has no private path (docs/INTERFACE.md, rule 5). The window then reads the result back through arbor's
//! library. A command keeps running if the window closes, so an effect is never cut off halfway by it. Nothing here
//! touches GTK: the window calls it on a thread of GLib's pool.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use arbor::outbox::{self, Effect, Outbox};

/// The `arbor` next to this program (or one folder up, for tests), else the one on the PATH.
pub fn arbor() -> PathBuf {
    let here = std::env::current_exe().ok();
    let near = here
        .iter()
        .flat_map(|exe| exe.ancestors().skip(1).take(2))
        .map(|dir| dir.join("arbor"))
        .find(|p| p.is_file());
    near.unwrap_or_else(|| PathBuf::from("arbor"))
}

/// How an `arbor` command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code, or `None` when it could not start or was ended by a signal.
    pub code: Option<i32>,
    /// arbor's own error, without its `arbor: `, when it gave one; empty when it did what it was asked. A released
    /// command's own failure is not one: its exit code is in its receipt.
    pub said: String,
    pub ms: u64,
}

/// Runs `arbor ARGS` and waits for it. Its output is not read: an effect's output is kept in the outbox, and
/// arbor's hints are only for a terminal.
pub fn run(args: &[&str]) -> Ran {
    let started = Instant::now();
    let out = Command::new(arbor())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let ms = started.elapsed().as_millis() as u64;
    match out {
        Ok(o) => Ran {
            code: o.status.code(),
            said: said(&String::from_utf8_lossy(&o.stderr)),
            ms,
        },
        Err(e) => Ran {
            code: None,
            said: format!("cannot run arbor: {e}"),
            ms,
        },
    }
}

/// arbor's error, which it prints last, as `arbor: WHY`; its warnings are not errors.
fn said(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .filter_map(|l| l.trim().strip_prefix("arbor: "))
        .find(|l| !l.starts_with("warning: "))
        .unwrap_or_default()
        .to_string()
}

/// A person's answer to a waiting effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Release,
    /// With the person's reason, which may be empty.
    Refuse(String),
}

/// What came of an answer.
#[derive(Debug, Clone)]
pub struct Answer {
    pub id: u64,
    pub ask: Ask,
    /// The effect as the outbox holds it after the command: done with its receipt, refused, or still waiting when
    /// arbor did not answer it. `None` when it cannot be read.
    pub effect: Option<Effect>,
    /// Why arbor did not do it, in its own words, when it did not.
    pub failed: Option<String>,
    /// The start of what a released command printed.
    pub output: Option<String>,
    /// From the start of the command to its end.
    pub ms: u64,
}

/// How much of what a released command printed the window shows: a few lines.
pub const OUTPUT_LINES: usize = 6;
const OUTPUT_BYTES: usize = 2000;

/// Answers effect `id` as `arbor release ID` or `arbor refuse ID [--why WHY]` does, by running it, then reads the
/// effect back.
pub fn answer(id: u64, ask: &Ask) -> Answer {
    let n = id.to_string();
    let ran = match ask {
        Ask::Release => run(&["release", &n]),
        Ask::Refuse(why) if why.trim().is_empty() => run(&["refuse", &n]),
        Ask::Refuse(why) => run(&["refuse", &n, "--why", why.trim()]),
    };
    let ob = Outbox::open();
    let effect = ob.as_ref().ok().and_then(|o| o.load(id).ok());
    // A released command that ran exits with its own code, and its receipt says how it ended: arbor failed only
    // when it says why, or when the effect still waits.
    let waits = effect
        .as_ref()
        .is_none_or(|e| e.state == outbox::State::Waiting);
    let failed = match (ran.said.as_str(), ran.code) {
        ("", _) if !waits => None,
        ("", Some(c)) => Some(format!("arbor stopped with exit code {c}")),
        ("", None) => Some("arbor was stopped".to_string()),
        (s, _) => Some(s.to_string()),
    };
    let output = match (&ob, &effect) {
        (Ok(o), Some(e)) if e.receipt.as_ref().is_some_and(|r| r.output > 0) => {
            start_of(&std::fs::read(o.output_file(id)).unwrap_or_default())
        }
        _ => None,
    };
    Answer {
        id,
        ask: ask.clone(),
        effect,
        failed,
        output,
        ms: ran.ms,
    }
}

/// The first lines of what a command printed, as text, and a mark when there is more.
fn start_of(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(OUTPUT_BYTES)]);
    let lines: Vec<&str> = text.lines().collect();
    let shown = lines[..lines.len().min(OUTPUT_LINES)].join("\n");
    let more = lines.len() > OUTPUT_LINES || bytes.len() > OUTPUT_BYTES;
    let shown = shown.trim_end();
    match (shown.is_empty(), more) {
        (true, _) => None,
        (false, true) => Some(format!("{shown}\n…")),
        (false, false) => Some(shown.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbor_speaks_in_one_line() {
        assert_eq!(
            said("arbor: effect 3 was refused at 14:05\n"),
            "effect 3 was refused at 14:05"
        );
        assert_eq!(
            said("arbor: warning: this step is not in the ledger: full\n"),
            ""
        );
        assert_eq!(said("a hint for a terminal\n\n"), "");
        assert_eq!(said(""), "");
    }

    #[test]
    fn only_the_start_of_the_output_is_shown() {
        assert_eq!(start_of(b"published\n").as_deref(), Some("published"));
        assert_eq!(start_of(b"\n\n"), None);
        let many: String = (1..=9).map(|i| format!("line {i}\n")).collect();
        let shown = start_of(many.as_bytes()).unwrap();
        assert_eq!(shown.lines().count(), OUTPUT_LINES + 1);
        assert!(shown.ends_with("line 6\n…"));
        assert!(start_of(&[b'x'; 5000]).unwrap().ends_with('…'));
    }
}
