//! The ledger: one line per step, appended to `ledger.jsonl` in Arbor's state folder. Each line ends with a
//! SHA-256 over the previous line's hash and its own text, so editing or deleting a line in the middle shows up in
//! `verify`. A process running as the same user can still rewrite the whole file: the chain catches partial
//! edits, and the head hash that `verify` prints lets a person notice a rewrite.
//!
//! Routine steps are not synced to disk one by one. A power cut can lose the last lines, never corrupt earlier
//! ones; a line torn by a crash mid-write is moved to `ledger.torn` on the next append.
//!
//! `follow` streams new lines as they are appended (`arbor watch`), for Mission Control and other watchers.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Context, Error, Result};
use crate::{paths, sys, world};

/// The hash before the first line.
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Where the hash starts in a line: `{…,"hash":"<64 hex>"}`.
const HASH_KEY: &str = ",\"hash\":\"";
/// Details longer than this are cut, so every line stays short and the tail is cheap to find.
const MAX_WHAT: usize = 240;

/// What a step cost.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    /// Wall time, in milliseconds.
    pub ms: u64,
    /// CPU time of the command's processes, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ms: Option<u64>,
    /// Peak memory in KiB: the whole cell's when it ran in a cgroup of its own, else its largest process's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_kib: Option<u64>,
    /// Peak number of processes and threads in the cell, where it had a cgroup and the kernel keeps the count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids: Option<u64>,
    /// Money, in cents: what a released effect said it would cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cents: Option<u64>,
}

/// One step, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// 1 for the first step, then one more for each.
    pub seq: u64,
    /// When it finished, in nanoseconds since 1970.
    pub ts: u64,
    /// Who acted: `you` for the arbor command, `cell` for a command in a world asking for an effect. Agents come
    /// later, in cells of their own.
    pub by: String,
    /// The key a command ran under, or the key a `key` step made and a `take-back` step took back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// `new`, `start`, `run`, `shell`, `stop`, `keep`, `undo`, `discard`, `key`, `take-back`, `effect`, `release` or
    /// `refuse`. A command's cell records `start` when it begins, then `run` or `shell` when it ends. An effect is
    /// asked for (`effect`), then refused, or released: `release` records it running before it starts, then how it
    /// ended.
    pub act: String,
    pub world: String,
    /// The cell a command ran in, as `arbor ps` names it, or the one that asked for an effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<String>,
    /// The effect an `effect`, `release` or `refuse` step is about, as `arbor outbox` numbers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<u64>,
    /// What exactly: the command, the folder, or a summary of the changes.
    pub what: String,
    /// The limits a command ran under: `mem 512MiB, cpu 0.5, pids 64, time 10m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<String>,
    /// `ok`, `running`, `waiting`, `exit N`, `killed: time limit`, `killed: memory limit`, `killed: key expired`,
    /// `stopped`, `stopped: key taken back`, `refused` or `failed`.
    pub out: String,
    pub cost: Cost,
}

/// A step to record; the ledger numbers and stamps it.
pub struct New<'a> {
    pub act: &'a str,
    pub by: &'a str,
    pub key: Option<&'a str>,
    pub world: &'a str,
    pub cell: Option<&'a str>,
    pub effect: Option<u64>,
    pub what: &'a str,
    pub limits: Option<&'a str>,
    pub out: &'a str,
    pub cost: Cost,
}

pub struct Ledger {
    path: PathBuf,
}

pub struct Verified {
    pub steps: u64,
    pub head: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn chain(prev: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(b"\n");
    h.update(body.as_bytes());
    hex(&h.finalize())
}

/// Splits a line into the text its hash covers and the hash.
fn split(line: &str) -> Option<(String, &str)> {
    let at = line.rfind(HASH_KEY)?;
    let hash = line[at + HASH_KEY.len()..].strip_suffix("\"}")?;
    Some((format!("{}}}", &line[..at]), hash))
}

/// Cuts `text` to `MAX_WHAT` characters, marking the cut.
fn short(text: &str) -> String {
    match text.char_indices().nth(MAX_WHAT) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

impl Ledger {
    /// The ledger in Arbor's state folder.
    pub fn open() -> Result<Ledger> {
        Ok(Ledger::at(paths::state_dir()?.join("ledger.jsonl")))
    }

    pub fn at(path: PathBuf) -> Ledger {
        Ledger { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends a step and returns it as recorded.
    pub fn append(&self, new: New) -> Result<Step> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).ctx(|| format!("cannot create {}", dir.display()))?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)
            .ctx(|| format!("cannot open {}", self.path.display()))?;
        // Held until the line is written, so concurrent commands take turns and never share a number.
        sys::flock(&file, true, true)?;
        let (seq, prev) = match self.last_line(&mut file)? {
            None => (0, GENESIS.to_string()),
            Some(line) => {
                let (body, hash) = split(&line).ok_or_else(|| self.damaged("its last line"))?;
                let last: Step =
                    serde_json::from_str(&body).map_err(|_| self.damaged("its last line"))?;
                (last.seq, hash.to_string())
            }
        };
        let step = Step {
            seq: seq + 1,
            ts: world::now_ns(),
            by: new.by.into(),
            key: new.key.map(Into::into),
            act: new.act.into(),
            world: new.world.into(),
            cell: new.cell.map(Into::into),
            effect: new.effect,
            what: short(new.what),
            limits: new.limits.filter(|l| !l.is_empty()).map(Into::into),
            out: new.out.into(),
            cost: new.cost,
        };
        let body = serde_json::to_string(&step).map_err(io::Error::other)?;
        let hash = chain(&prev, &body);
        let line = format!("{}{HASH_KEY}{hash}\"}}\n", &body[..body.len() - 1]);
        file.write_all(line.as_bytes())?;
        Ok(step)
    }

    fn damaged(&self, what: &str) -> Error {
        Error::new(format!(
            "the ledger {} is damaged ({what}); `arbor ledger --verify` says where",
            paths::tilde(&self.path)
        ))
    }

    /// The last complete line, after moving a torn tail (a crash mid-write) to `ledger.torn`.
    fn last_line(&self, file: &mut File) -> Result<Option<String>> {
        let len = file.metadata()?.len();
        if len == 0 {
            return Ok(None);
        }
        let start = len.saturating_sub(16 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        let end = match tail.iter().rposition(|&b| b == b'\n') {
            Some(i) => i,
            None if start == 0 => {
                self.set_aside(file, &tail, 0)?;
                return Ok(None);
            }
            None => return Err(self.damaged("its last line is too long")),
        };
        if end + 1 < tail.len() {
            self.set_aside(file, &tail[end + 1..], start + end as u64 + 1)?;
        }
        let from = tail[..end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        if from == 0 && start > 0 {
            return Err(self.damaged("its last line is too long"));
        }
        Ok(Some(String::from_utf8_lossy(&tail[from..end]).into_owned()))
    }

    fn set_aside(&self, file: &File, torn: &[u8], keep: u64) -> Result<()> {
        let mut aside = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.path.with_extension("torn"))?;
        aside.write_all(torn)?;
        aside.write_all(b"\n")?;
        file.set_len(keep)?;
        Ok(())
    }

    /// The last `n` steps, oldest first.
    pub fn last(&self, n: usize) -> Result<Vec<Step>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx(|| format!("cannot read {}", self.path.display())),
        };
        let mut steps = VecDeque::with_capacity(n + 1);
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Some(step) = split(&line).and_then(|(body, _)| serde_json::from_str(&body).ok())
            {
                steps.push_back(step);
                if steps.len() > n {
                    steps.pop_front();
                }
            }
        }
        Ok(steps.into())
    }

    /// Writes the steps after `since` to `out` as their JSON lines, then each new step as it is appended, until
    /// `out`'s reader goes away; without `since`, only new steps. It waits on inotify, so it costs nothing while
    /// nothing happens. A reader that missed steps asks again from the last number it has.
    pub fn follow<W: Write + AsFd>(&self, since: Option<u64>, out: &mut W) -> Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).ctx(|| format!("cannot create {}", dir.display()))?;
        // Watching starts before the first read, so a step appended in between is not missed.
        let watch = sys::DirWatch::new(dir)?;
        let (mut at, mut last, mut quiet) = (0u64, since.unwrap_or(0), since.is_none());
        let mut buf = Vec::new();
        loop {
            match File::open(&self.path) {
                Ok(mut file) => {
                    if file.metadata()?.len() < at {
                        at = 0; // rewritten: read it again, going by step numbers
                    }
                    file.seek(SeekFrom::Start(at))?;
                    buf.clear();
                    file.read_to_end(&mut buf)?;
                    // Whole lines only: one still being written is read once it is done.
                    let whole = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
                    for line in buf[..whole].split(|&b| b == b'\n') {
                        let seq = std::str::from_utf8(line)
                            .ok()
                            .and_then(split)
                            .and_then(|(body, _)| serde_json::from_str::<Step>(&body).ok())
                            .map(|s| s.seq);
                        if let Some(seq) = seq
                            && seq > last
                        {
                            if !quiet {
                                out.write_all(line)?;
                                out.write_all(b"\n")?;
                            }
                            last = seq;
                        }
                    }
                    at += whole as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => at = 0,
                Err(e) => return Err(e).ctx(|| format!("cannot read {}", self.path.display())),
            }
            out.flush()?;
            quiet = false;
            if !watch.wait(out.as_fd())? {
                return Ok(());
            }
        }
    }

    /// Checks every line: numbered one after another, each hash right. Returns the count and the head hash.
    pub fn verify(&self) -> Result<Verified> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Verified {
                    steps: 0,
                    head: GENESIS.into(),
                });
            }
            Err(e) => return Err(e).ctx(|| format!("cannot read {}", self.path.display())),
        };
        let mut reader = BufReader::new(file);
        let (mut prev, mut seq, mut line) = (GENESIS.to_string(), 0u64, String::new());
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let at = seq + 1;
            let broken =
                |why: &str| Error::new(format!("the ledger is broken at step {at}: {why}"));
            let Some(text) = line.strip_suffix('\n') else {
                return Err(broken(
                    "the line is cut off (the next append sets it aside)",
                ));
            };
            let (body, hash) = split(text).ok_or_else(|| broken("the line has no hash"))?;
            let step: Step =
                serde_json::from_str(&body).map_err(|_| broken("the line cannot be read"))?;
            if step.seq != at {
                return Err(broken(&format!(
                    "it is numbered {}; a step is missing or added",
                    step.seq
                )));
            }
            if chain(&prev, &body) != hash {
                return Err(broken(
                    "its hash does not match: the line or one before it was changed",
                ));
            }
            (prev, seq) = (hash.to_string(), at);
        }
        Ok(Verified {
            steps: seq,
            head: prev,
        })
    }
}

/// A local date and time for people: `10-17 06:30:01`.
pub fn when(ts_ns: u64) -> String {
    let secs = (ts_ns / 1_000_000_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return "?".into();
    }
    format!(
        "{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arbor-ledger-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn add(l: &Ledger, what: &str) -> Step {
        l.append(New {
            act: "run",
            by: "you",
            key: None,
            world: "demo",
            cell: Some("0badcafe"),
            effect: None,
            what,
            limits: None,
            out: "exit 0",
            cost: Cost {
                ms: 3,
                cpu_ms: Some(1),
                ..Cost::default()
            },
        })
        .unwrap()
    }

    #[test]
    fn a_chain_verifies_and_an_edit_is_found_at_its_step() {
        let dir = temp("edit");
        let l = Ledger::at(dir.join("ledger.jsonl"));
        assert_eq!(l.verify().unwrap().steps, 0);
        for what in ["one", "two", "three"] {
            add(&l, what);
        }
        let v = l.verify().unwrap();
        assert_eq!(v.steps, 3);
        assert_eq!(v.head.len(), 64);
        assert_eq!(
            l.last(2)
                .unwrap()
                .iter()
                .map(|s| s.what.as_str())
                .collect::<Vec<_>>(),
            ["two", "three"]
        );

        let text = fs::read_to_string(l.path()).unwrap();
        fs::write(l.path(), text.replacen("\"two\"", "\"TWO\"", 1)).unwrap();
        let err = l.verify().err().unwrap();
        assert!(err.msg.contains("step 2"), "{}", err.msg);

        let lines: Vec<&str> = text.lines().collect();
        fs::write(l.path(), format!("{}\n{}\n", lines[0], lines[2])).unwrap();
        let err = l.verify().err().unwrap();
        assert!(
            err.msg.contains("step 2") && err.msg.contains("missing"),
            "{}",
            err.msg
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_torn_last_line_is_set_aside() {
        let dir = temp("torn");
        let l = Ledger::at(dir.join("ledger.jsonl"));
        add(&l, "one");
        let mut f = OpenOptions::new().append(true).open(l.path()).unwrap();
        f.write_all(b"{\"seq\":2,\"ts\":1,\"by\":\"yo").unwrap();
        assert!(l.verify().is_err());
        assert_eq!(add(&l, "two").seq, 2);
        assert_eq!(l.verify().unwrap().steps, 2);
        assert!(
            fs::read_to_string(dir.join("ledger.torn"))
                .unwrap()
                .contains("\"seq\":2")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_appends_take_turns() {
        let dir = temp("turns");
        let path = dir.join("ledger.jsonl");
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let l = Ledger::at(path);
                    for i in 0..50 {
                        add(&l, &format!("{t}-{i}"));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let l = Ledger::at(path);
        assert_eq!(l.verify().unwrap().steps, 200);
        assert_eq!(l.last(1000).unwrap().len(), 200);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn follow_sends_the_steps_after_a_number_then_each_new_one() {
        let dir = temp("follow");
        let path = dir.join("ledger.jsonl");
        let l = Ledger::at(path.clone());
        add(&l, "one");
        add(&l, "two");
        let (reader, mut writer) = io::pipe().unwrap();
        let follower = std::thread::spawn(move || Ledger::at(path).follow(Some(1), &mut writer));
        let mut lines = BufReader::new(reader).lines();
        let mut next = || {
            let line = lines.next().unwrap().unwrap();
            let (body, hash) = split(&line).unwrap();
            assert_eq!(hash.len(), 64);
            serde_json::from_str::<Step>(&body).unwrap()
        };
        assert_eq!(next().what, "two");
        add(&l, "three");
        let three = next();
        assert_eq!((three.seq, three.cell.as_deref()), (3, Some("0badcafe")));
        // The reader goes away, so the follower stops instead of waiting forever.
        drop(lines);
        assert!(follower.join().unwrap().is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn long_details_are_cut() {
        assert_eq!(short("abc"), "abc");
        let long = "é".repeat(MAX_WHAT + 10);
        assert_eq!(short(&long).chars().count(), MAX_WHAT + 1);
    }
}
