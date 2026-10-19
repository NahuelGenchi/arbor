//! What the window's buttons do. Each one runs the `arbor` command that a person or an agent would type, as a
//! process of its own, so that the outbox's own rules apply and the ledger records the step with you as the actor:
//! the window has no private path (docs/INTERFACE.md, rule 5). The window then reads the result back through arbor's
//! library. A command keeps running if the window closes, so an effect is never cut off halfway by it. Nothing here
//! touches GTK: the window calls it on a thread of GLib's pool.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use arbor::outbox::{self, Effect, Outbox};
use arbor::tree::{self, Change};
use arbor::world::{State, World};

/// What a button on a line does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    Release,
    Refuse,
    /// Shows or hides what a try changed: it changes nothing, so it asks for no confirmation.
    Changes,
    Keep,
    Undo,
}

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
    run_reading(args, false).0
}

/// Runs `arbor ARGS`, and returns what it printed too when `read`.
fn run_reading(args: &[&str], read: bool) -> (Ran, Vec<u8>) {
    let started = Instant::now();
    let out = Command::new(arbor())
        .args(args)
        .stdin(Stdio::null())
        .stdout(if read { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::piped())
        .output();
    let ms = started.elapsed().as_millis() as u64;
    match out {
        Ok(o) => (
            Ran {
                code: o.status.code(),
                said: said(&String::from_utf8_lossy(&o.stderr)),
                ms,
            },
            o.stdout,
        ),
        Err(e) => (
            Ran {
                code: None,
                said: format!("cannot run arbor: {e}"),
                ms,
            },
            Vec::new(),
        ),
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

/// What a try changed, for the window to show before it is kept.
#[derive(Debug, Clone, Default)]
pub struct Look {
    /// The files, in path order.
    pub files: Vec<Change>,
    /// What would refuse a keep: `path: why`, as `arbor keep` says it.
    pub conflicts: Vec<String>,
    /// Each text file's change, as `arbor diff --patch` gives it, cut to `PATCH_LINES` lines, by path.
    pub patches: HashMap<String, Patch>,
    /// Why the changes could not be read.
    pub failed: Option<String>,
}

/// One file's lines of a diff, the first `PATCH_LINES` of them, and how many more there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub text: String,
    pub more: usize,
}

/// How many lines of one file's diff the window shows.
pub const PATCH_LINES: usize = 40;

/// Reads what try `name` changed: the files and the conflicts through arbor's library, as `arbor diff` and
/// `arbor keep` read them, and the lines by running `arbor diff NAME --patch`. It changes nothing.
pub fn look(name: &str) -> Look {
    let read = || -> arbor::error::Result<(Vec<Change>, Vec<String>)> {
        let w = World::load(name)?;
        let entries = tree::entries(&w)?;
        Ok((tree::changes(&w, &entries)?, tree::conflicts(&w, &entries)?))
    };
    let (files, conflicts) = match read() {
        Ok(x) => x,
        Err(e) => {
            return Look {
                failed: Some(e.to_string()),
                ..Look::default()
            };
        }
    };
    let (ran, out) = run_reading(&["diff", name, "--patch"], true);
    let patches = match ran.code {
        Some(0) => patches(&String::from_utf8_lossy(&out)),
        _ => HashMap::new(),
    };
    Look {
        files,
        conflicts,
        patches,
        failed: None,
    }
}

/// A whole diff, split by file: each file starts with its `--- ` and `+++ ` lines. The line numbers of each part
/// (`@@ -1 +1 @@`) are left out, and a gap between two parts is shown as `…`.
fn patches(diff: &str) -> HashMap<String, Patch> {
    let lines: Vec<&str> = diff.lines().collect();
    let mut starts = Vec::new();
    for i in 0..lines.len().saturating_sub(1) {
        let (a, b) = (lines[i], lines[i + 1]);
        if let (Some(a), Some(b)) = (a.strip_prefix("--- "), b.strip_prefix("+++ "))
            && (a == "/dev/null" || a.starts_with("a/"))
            && (b == "/dev/null" || b.starts_with("b/"))
        {
            let path = match b.strip_prefix("b/") {
                Some(p) => p,
                None => a.strip_prefix("a/").unwrap_or(a),
            };
            starts.push((i, path.to_string()));
        }
    }
    let mut out = HashMap::new();
    for (n, (at, path)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(lines.len(), |(next, _)| *next);
        let body: Vec<&str> = lines[at + 2..end]
            .iter()
            .enumerate()
            .filter_map(|(i, l)| match (l.starts_with("@@ "), i) {
                (true, 0) => None,
                (true, _) => Some("…"),
                (false, _) => Some(*l),
            })
            .collect();
        out.insert(
            path.clone(),
            Patch {
                text: body[..body.len().min(PATCH_LINES)].join("\n"),
                more: body.len().saturating_sub(PATCH_LINES),
            },
        );
    }
    out
}

/// What came of a keep or an undo.
#[derive(Debug, Clone)]
pub struct Done {
    pub name: String,
    pub what: Do,
    /// The real folder.
    pub base: PathBuf,
    /// Added, changed and deleted: what was kept, or taken back out.
    pub counts: [usize; 3],
    /// Why arbor did not do it, in its own words, when it did not.
    pub failed: Option<String>,
    /// The files that stopped it, as `path: why`.
    pub problems: Vec<String>,
    pub ms: u64,
}

/// Keeps try `name` (`arbor keep NAME`) or takes its keep back (`arbor undo NAME`), by running it, then reads the
/// world back. When arbor refuses, the files that stopped it are read through its library.
pub fn keep_or_undo(name: &str, what: Do) -> Done {
    let world = World::load(name);
    let counts_now = |w: &World| {
        tree::entries(w)
            .and_then(|e| tree::changes(w, &e))
            .map(|c| crate::state::counts(&c))
            .unwrap_or_default()
    };
    // What a keep applies is the try's changes before it; what an undo takes back, its changes after.
    let before = match (&world, what) {
        (Ok(w), Do::Keep) => counts_now(w),
        _ => [0; 3],
    };
    let ran = run(&[if what == Do::Undo { "undo" } else { "keep" }, name]);
    let after = World::load(name);
    let want = if what == Do::Undo {
        State::Open
    } else {
        State::Kept
    };
    let done = ran.said.is_empty() && after.as_ref().is_ok_and(|w| w.state == want);
    let failed = (!done).then(|| match ran.said.as_str() {
        "" => format!("arbor stopped with exit code {}", ran.code.unwrap_or(-1)),
        s => s.to_string(),
    });
    let problems = match (&after, failed.is_some()) {
        (Ok(w), true) if what == Do::Undo => tree::undo_conflicts(w, false).unwrap_or_default(),
        (Ok(w), true) => tree::entries(w)
            .and_then(|e| tree::conflicts(w, &e))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let counts = match (&after, what, done) {
        (Ok(w), Do::Undo, true) => counts_now(w),
        (_, Do::Keep, true) => before,
        _ => [0; 3],
    };
    Done {
        name: name.to_string(),
        what,
        base: after
            .map(|w| w.base)
            .or(world.map(|w| w.base))
            .unwrap_or_default(),
        counts,
        failed,
        problems,
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
    fn a_diff_splits_by_file() {
        let diff = "--- a/cart.rs\n+++ b/cart.rs\n@@ -1 +1 @@\n-fn total() {}\n+fn total() -> u32 { 0 }\n\
                    --- /dev/null\n+++ b/tests/cart.rs\n@@ -0,0 +1 @@\n+#[test] fn t() {}\n\
                    --- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let p = patches(diff);
        assert_eq!(p.len(), 3);
        assert_eq!(
            p["cart.rs"].text,
            "-fn total() {}\n+fn total() -> u32 { 0 }"
        );
        assert_eq!(p["tests/cart.rs"].text, "+#[test] fn t() {}");
        let two = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n";
        assert_eq!(patches(two)["x"].text, "-a\n+b\n…\n-c\n+d");
        assert_eq!(p["old.txt"].more, 0);
        let long: String = "--- a/x\n+++ b/x\n".to_string() + &"+line\n".repeat(PATCH_LINES + 5);
        assert_eq!(patches(&long)["x"].more, 5);
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
