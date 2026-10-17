//! The outbox: what commands in worlds ask to do outside, waiting for a person. An effect is a command that runs
//! outside the world, as the person, with the network and their rights: a `git push`, an email, a payment. A cell
//! asks for one through a socket of its own (`Socket`), which the arbor running the cell serves; the cell never sees
//! the outbox and cannot answer for the person. A person releases an effect, which runs it once in the real folder
//! and keeps its receipt, or refuses it. A key's budget caps what the effects released under it may cost.
//!
//! On disk, in the state folder's `outbox`: `<id>.json` for each effect, `<id>.in` for the input it was given,
//! `<id>.out` for what it printed, and the `lock` that changes take turns under. An effect never runs twice: it is
//! marked running, on disk, before its command starts, and one whose arbor stopped while it ran stays that way, for
//! a person to look at.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{Context, Error, Result};
use crate::key::{self, Key, KeyRef};
use crate::ledger::{self, Cost, Ledger};
use crate::{cell, cgroup, paths, sys, table, world};

/// The variable that tells a command in a world where its outbox socket is.
pub const SOCKET_VAR: &str = "ARBOR_OUTBOX";
/// Where a cell sees its socket.
pub const CELL_SOCKET: &str = "/run/arbor/outbox.sock";
/// Where a cell finds the `arbor` command, first on its `PATH`.
pub const CELL_BIN: &str = "/run/arbor/bin";

// What a cell may send is capped, so a command in a world cannot make the arbor outside it use much memory, many
// threads or much disk.
/// The longest request line, the command and its words included.
const MAX_LINE: usize = 64 * 1024;
/// The most input an effect may carry.
pub const MAX_INPUT: usize = 1 << 20;
const MAX_ARGS: usize = 1024;
/// The longest `--why`, in characters.
const MAX_WHY: usize = 2000;
/// Effects one world may have waiting at once.
const MAX_WAITING: usize = 64;
/// Requests a cell may have open at once.
const MAX_OPEN: usize = 8;
/// How long a cell has to send a whole request.
const REQUEST_TIME: Duration = Duration::from_secs(5);
/// What arbor keeps of an effect's output, and sends back to a command that waits for it.
pub const KEPT_OUTPUT: usize = 1 << 20;
/// The largest amount of money arbor reads: $999,999,999.99.
const MAX_CENTS: Cents = 99_999_999_999;

/// Amounts of money, in cents.
pub type Cents = u64;

/// Reads an amount of money: `5`, `2.50`, `$0.40`.
pub fn parse_money(text: &str) -> Result<Cents> {
    let bad = || {
        Error::usage(format!(
            "'{text}' is not an amount of money: use for example 5, 2.50 or $0.40"
        ))
    };
    let t = text.trim();
    let t = t.strip_prefix('$').unwrap_or(t);
    let (whole, cents) = match t.split_once('.') {
        Some((whole, cents)) if (1..=2).contains(&cents.len()) => (whole, cents),
        Some(_) => return Err(bad()),
        None => (t, "0"),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(cents) || whole.len() > 9 {
        return Err(bad());
    }
    let cents = cents.parse::<u64>().map_err(|_| bad())? * if cents.len() == 1 { 10 } else { 1 };
    Ok(whole.parse::<u64>().map_err(|_| bad())? * 100 + cents)
}

/// `$2.50`.
pub fn money(cents: Cents) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Asked for, and waiting for a person.
    Waiting,
    /// Released: its command is running, or its arbor stopped while it ran.
    Running,
    /// Released, and its command ended.
    Done,
    /// A person said no, or threw away the world or took back the key that asked.
    Refused,
}

/// What a released effect did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Its exit code, or 128 + the signal that ended it.
    pub exit: i32,
    /// How many bytes it printed, on standard output and error together, and their SHA-256. The first
    /// `KEPT_OUTPUT` are kept in `<id>.out`.
    pub output: u64,
    pub sha256: String,
    /// Wall time and CPU time, in milliseconds.
    pub ms: u64,
    pub cpu_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// 1 for the first, then one more for each.
    pub id: u64,
    /// The world and the cell that asked, as the arbor running the cell knew them.
    pub world: String,
    pub cell: String,
    /// The key the cell ran under, then each key that one was made within, outwards. A budget on any of them
    /// counts this effect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyRef>,
    /// The world's real folder, and where in it the effect runs: where the asker stood.
    pub base: PathBuf,
    #[serde(default, skip_serializing_if = "is_empty_path")]
    pub dir: PathBuf,
    pub cmd: Vec<String>,
    /// How many bytes of input it was given, kept in `<id>.in`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    /// What it will cost, by the asker's word: arbor cannot check it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cents>,
    /// Why, in the asker's words, which nobody checked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// The asker waits for the answer, and gets back what the command prints.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub back: bool,
    /// When it was asked for, and when a person released or refused it, in nanoseconds since 1970.
    pub asked_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_ns: Option<u64>,
    pub state: State,
    /// Why it was refused: the person's words, or what refused it with the world or the key.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refusal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

fn is_empty_path(p: &Path) -> bool {
    p.as_os_str().is_empty()
}

impl Effect {
    /// Where it runs: the world's real folder, at the place the asker stood.
    pub fn folder(&self) -> PathBuf {
        self.base.join(&self.dir)
    }

    /// The key the asker ran under.
    pub fn key(&self) -> Option<&str> {
        self.keys.first().map(|k| k.name.as_str())
    }

    /// Its command as a shell would take it back, with anything a terminal would act on spelled out.
    pub fn shown(&self) -> String {
        cell::shown(&self.cmd)
    }
}

/// What the effects released under `key`, or under a key made within it, cost. One that is running counts: its
/// command may have spent already.
pub fn spent(all: &[Effect], key: &KeyRef) -> Cents {
    all.iter()
        .filter(|e| matches!(e.state, State::Running | State::Done) && e.keys.contains(key))
        .filter_map(|e| e.cost)
        .fold(0, Cents::saturating_add)
}

/// Those of `keys` that are still there: not taken back, nor taken back and made again.
pub fn present(keys: &[KeyRef]) -> Vec<Key> {
    keys.iter()
        .filter_map(|k| {
            Key::load(&k.name)
                .ok()
                .filter(|key| key.made_ns == k.made_ns)
        })
        .collect()
}

/// Why releasing `e` would spend more than a key it counts against allows. `keys` are those of its keys that are
/// still there, and `all` every effect. Under a key that may spend only so much, an effect must say what it costs.
pub fn over_budget(e: &Effect, all: &[Effect], keys: &[Key]) -> Option<String> {
    keys.iter().find_map(|k| {
        let most = k.spend?;
        let Some(cost) = e.cost else {
            return Some(format!(
                "effect {} does not say what it costs, and key '{}' may spend {} in all",
                e.id,
                k.name,
                money(most)
            ));
        };
        let spent = spent(all, &k.id());
        (spent.saturating_add(cost) > most).then(|| {
            format!(
                "effect {} costs {}, and key '{}' has {} left of {}",
                e.id,
                money(cost),
                k.name,
                money(most.saturating_sub(spent)),
                money(most)
            )
        })
    })
}

/// Writes a file only its owner can read, whole, to disk.
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

pub struct Outbox {
    dir: PathBuf,
}

impl Outbox {
    /// The outbox in Arbor's state folder.
    pub fn open() -> Result<Outbox> {
        Ok(Outbox::at(paths::state_dir()?.join("outbox")))
    }

    pub fn at(dir: PathBuf) -> Outbox {
        Outbox { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: u64, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    pub fn input_file(&self, id: u64) -> PathBuf {
        self.file(id, "in")
    }

    pub fn output_file(&self, id: u64) -> PathBuf {
        self.file(id, "out")
    }

    /// Makes the folder, which only its owner may enter.
    pub fn create(&self) -> Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .ctx(|| format!("cannot create {}", self.dir.display()))
    }

    /// Takes the lock that changes take turns under, until the returned file is dropped.
    fn lock(&self) -> Result<File> {
        self.create()?;
        let path = self.dir.join("lock");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .ctx(|| format!("cannot open {}", path.display()))?;
        sys::flock(&file, true, true)?;
        Ok(file)
    }

    pub fn load(&self, id: u64) -> Result<Effect> {
        let text = match fs::read_to_string(self.file(id, "json")) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::usage(format!(
                    "there is no effect {id} (see `arbor outbox`)"
                )));
            }
            Err(e) => return Err(e).ctx(|| format!("cannot read effect {id}")),
        };
        serde_json::from_str(&text).ctx(|| format!("effect {id} is damaged"))
    }

    /// The numbers of every effect, damaged ones included.
    fn ids(&self) -> Result<Vec<u64>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx(|| format!("cannot read {}", self.dir.display())),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if let Some(id) = name
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|n| n.parse().ok())
            {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Every effect, oldest first. Files that cannot be read are skipped.
    pub fn list(&self) -> Result<Vec<Effect>> {
        Ok(self
            .ids()?
            .into_iter()
            .filter_map(|id| self.load(id).ok())
            .collect())
    }

    /// Writes an effect's file whole, through a temporary one. The caller holds the lock.
    fn save(&self, e: &Effect) -> Result<()> {
        let path = self.file(e.id, "json");
        let tmp = self.dir.join(format!(".{}.json.tmp", e.id));
        let text = serde_json::to_string_pretty(e).map_err(io::Error::other)?;
        write_file(&tmp, format!("{text}\n").as_bytes())
            .and_then(|()| fs::rename(&tmp, &path))
            .and_then(|()| File::open(&self.dir)?.sync_all())
            .ctx(|| format!("cannot write {}", path.display()))
    }

    /// Stores a new effect and its input, numbered one after the last, waiting. One world may have only
    /// `MAX_WAITING` effects waiting.
    pub fn add(&self, mut e: Effect, input: &[u8]) -> Result<Effect> {
        let _lock = self.lock()?;
        let waiting = self
            .list()?
            .iter()
            .filter(|x| x.state == State::Waiting && x.world == e.world)
            .count();
        if waiting >= MAX_WAITING {
            return Err(Error::refused(format!(
                "world {} has {MAX_WAITING} effects waiting already; ask again once a person has answered some",
                e.world
            )));
        }
        e.id = self.ids()?.last().map_or(1, |last| last + 1);
        e.input = None;
        if !input.is_empty() {
            let path = self.input_file(e.id);
            write_file(&path, input).ctx(|| format!("cannot write {}", path.display()))?;
            e.input = Some(input.len() as u64);
        }
        (e.asked_ns, e.answered_ns, e.state) = (world::now_ns(), None, State::Waiting);
        (e.refusal, e.receipt) = (String::new(), None);
        self.save(&e)?;
        Ok(e)
    }

    /// Marks a waiting effect running, once it may run: its folder is there, its key has not been taken back, and
    /// its cost fits within every budget it counts against. Returns it with its output file, created and locked:
    /// while the lock is held, its arbor is still there.
    pub fn begin(&self, id: u64) -> Result<(Effect, File)> {
        let _lock = self.lock()?;
        let mut e = self.load(id)?;
        unanswered(&e)?;
        let folder = e.folder();
        if !folder.is_dir() {
            return Err(Error::refused(format!(
                "{} is not in the real folder; if world {} made it, keep the world first",
                table::escaped(&paths::tilde(&folder)),
                e.world
            )));
        }
        if let Some(k) = e.keys.first()
            && !key::current(k)
        {
            return Err(Error::refused(format!(
                "key '{}' was taken back, so its effects can only be refused (`arbor refuse {id}`)",
                k.name
            )));
        }
        if let Some(why) = over_budget(&e, &self.list()?, &present(&e.keys)) {
            return Err(Error::refused(why));
        }
        let path = self.output_file(id);
        let out = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .ctx(|| format!("cannot write {}", path.display()))?;
        sys::flock(&out, true, false)?;
        (e.state, e.answered_ns) = (State::Running, Some(world::now_ns()));
        self.save(&e)?;
        Ok((e, out))
    }

    /// Puts an effect whose command could not start back to waiting: nothing ran.
    pub fn unbegin(&self, id: u64) {
        let Ok(_lock) = self.lock() else { return };
        if let Ok(mut e) = self.load(id)
            && e.state == State::Running
        {
            (e.state, e.answered_ns) = (State::Waiting, None);
            if self.save(&e).is_ok() {
                let _ = fs::remove_file(self.output_file(id));
            }
        }
    }

    /// Records how a released effect's command ended.
    pub fn finish(&self, id: u64, receipt: Receipt) -> Result<Effect> {
        let _lock = self.lock()?;
        let mut e = self.load(id)?;
        (e.state, e.receipt) = (State::Done, Some(receipt));
        self.save(&e)?;
        Ok(e)
    }

    /// Says no to a waiting effect.
    pub fn refuse(&self, id: u64, why: &str) -> Result<Effect> {
        let _lock = self.lock()?;
        let mut e = self.load(id)?;
        unanswered(&e)?;
        (e.state, e.answered_ns, e.refusal) = (State::Refused, Some(world::now_ns()), why.into());
        self.save(&e)?;
        Ok(e)
    }

    /// Says no to every waiting effect that `which` picks, because `why`, and returns them.
    pub fn refuse_waiting(
        &self,
        which: impl Fn(&Effect) -> bool,
        why: &str,
    ) -> Result<Vec<Effect>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        let mut refused = Vec::new();
        for mut e in self.list()? {
            if e.state == State::Waiting && which(&e) {
                (e.state, e.answered_ns, e.refusal) =
                    (State::Refused, Some(world::now_ns()), why.into());
                self.save(&e)?;
                refused.push(e);
            }
        }
        Ok(refused)
    }

    /// Whether the arbor that released a running effect is still there. One that is not stopped while the command
    /// ran, and what the effect did is not known.
    pub fn still_running(&self, id: u64) -> bool {
        File::open(self.output_file(id)).is_ok_and(|f| {
            matches!(sys::flock(&f, false, false), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
        })
    }
}

/// Refuses to answer an effect twice.
fn unanswered(e: &Effect) -> Result<()> {
    let at = || key::clock(e.answered_ns.unwrap_or(e.asked_ns), world::now_ns());
    let why = match e.state {
        State::Waiting => return Ok(()),
        State::Running => format!(
            "effect {} was released at {} and has not finished",
            e.id,
            at()
        ),
        State::Done => format!(
            "effect {} was released at {}; an effect runs only once",
            e.id,
            at()
        ),
        State::Refused => format!("effect {} was refused at {}", e.id, at()),
    };
    Err(Error::refused(why))
}

/// Runs a released effect's command as the person, in its folder, with its input, and returns its receipt. What it
/// prints, on standard output and error together, goes to `shown` as it comes, and its first `KEPT_OUTPUT` bytes to
/// `out`. Ctrl-C belongs to the command. Errs only when the command could not start: once it has, whatever goes
/// wrong is in the receipt, so that the effect is never put back to wait and run again.
pub fn run(outbox: &Outbox, e: &Effect, out: File, shown: &mut dyn Write) -> Result<Receipt> {
    let program = e
        .cmd
        .first()
        .ok_or_else(|| Error::new(format!("effect {} has no command", e.id)))?;
    let stdin = match e.input {
        Some(_) => {
            let path = outbox.input_file(e.id);
            Stdio::from(File::open(&path).ctx(|| format!("cannot read {}", path.display()))?)
        }
        None => Stdio::null(),
    };
    let (read, write) = sys::pipe()?;
    let (cpu_before, _) = sys::children_usage();
    let started = Instant::now();
    // The command, and with it this side's copies of the pipe's writing end, are gone once it has started, so the
    // pipe ends when the effect's own processes close it.
    let child = Command::new(program)
        .args(&e.cmd[1..])
        .current_dir(e.folder())
        .stdin(stdin)
        .stdout(Stdio::from(write.try_clone()?))
        .stderr(Stdio::from(write))
        .spawn();
    let mut child = child.ctx(|| format!("cannot run {}", table::escaped(program)))?;
    let old = [libc::SIGINT, libc::SIGQUIT, libc::SIGPIPE]
        .map(|sig| (sig, unsafe { libc::signal(sig, libc::SIG_IGN) }));
    let (output, sha256) = copy_output(read, child.id() as libc::pid_t, out, shown);
    let status = child.wait();
    for (sig, handler) in old {
        unsafe { libc::signal(sig, handler) };
    }
    let (cpu_after, _) = sys::children_usage();
    Ok(Receipt {
        // -1: arbor could not learn how it ended.
        exit: status.map_or(-1, |s| {
            s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
        }),
        output,
        sha256,
        ms: started.elapsed().as_millis() as u64,
        cpu_ms: cpu_after.saturating_sub(cpu_before),
    })
}

/// Copies what the command prints to `shown`, and its first `KEPT_OUTPUT` bytes to `out`, until every writer has
/// closed the pipe, or the command has ended and the pipe holds nothing more (a process it left behind may keep the
/// pipe open). Returns how many bytes it printed and their SHA-256.
fn copy_output(
    read: OwnedFd,
    pid: libc::pid_t,
    mut out: File,
    shown: &mut dyn Write,
) -> (u64, String) {
    // Without a way to see the command end, copying goes on until the pipe closes.
    let exited = sys::pidfd_open(pid).ok();
    let mut pipe = File::from(read);
    let (mut hash, mut total, mut showing, mut ended) = (Sha256::new(), 0u64, true, false);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let ready = match (&exited, ended) {
            (_, true) => sys::ready(&[pipe.as_fd()], Some(Duration::ZERO)),
            (Some(exited), false) => sys::ready(&[pipe.as_fd(), exited.as_fd()], None),
            (None, false) => sys::ready(&[pipe.as_fd()], None),
        };
        let Ok(ready) = ready else { break };
        if ready[0] {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let chunk = &buf[..n];
            hash.update(chunk);
            let keep = KEPT_OUTPUT.saturating_sub(total as usize).min(n);
            // A full disk loses the kept copy, never the count or the hash.
            let _ = out.write_all(&chunk[..keep]);
            total += n as u64;
            if showing && shown.write_all(chunk).and_then(|()| shown.flush()).is_err() {
                showing = false; // the person's end went away; the command runs on
            }
            continue;
        }
        if ended {
            break;
        }
        ended = ready.get(1) == Some(&true);
    }
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (total, digest)
}

/// Who asks through a cell's socket: set by the arbor that runs the cell, never by what runs in it.
#[derive(Debug, Clone)]
pub struct Asker {
    pub world: String,
    pub base: PathBuf,
    pub cell: String,
    /// The key the cell runs under, then each key it was made within; empty without one.
    pub keys: Vec<KeyRef>,
}

/// A request from a cell: one JSON line, then `input` bytes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    cmd: Vec<String>,
    /// Where the asker stood, as the cell sees it.
    #[serde(default)]
    dir: String,
    #[serde(default)]
    cost: Option<Cents>,
    #[serde(default)]
    why: String,
    #[serde(default)]
    input: u64,
    #[serde(default)]
    wait: bool,
}

/// Checks what a cell asked for. Who asked is not in the request: the arbor that runs the cell knows.
fn check(req: &Request) -> Result<()> {
    let bad = |why: String| Err(Error::usage(why));
    match req.cmd.first() {
        None => bad("which command? usage: arbor effect -- CMD...".into()),
        Some(name) if name.is_empty() => bad("the command's name is empty".into()),
        _ if req.cmd.len() > MAX_ARGS => {
            bad(format!("the command has more than {MAX_ARGS} arguments"))
        }
        _ if req.cmd.iter().any(|a| a.contains('\0')) => {
            bad("an argument holds a NUL character".into())
        }
        _ if req.why.chars().count() > MAX_WHY => {
            bad(format!("--why is longer than {MAX_WHY} characters"))
        }
        _ if req.cost.is_some_and(|c| c > MAX_CENTS) => bad("the cost is too high".into()),
        _ => Ok(()),
    }
}

/// Where `dir`, a folder as the cell sees it, lies within the world's folder `base`: the cell sees the world at the
/// folder's own path. The folder itself when `dir` lies elsewhere, such as in the cell's `/tmp`.
fn inside(base: &Path, dir: &Path) -> PathBuf {
    match dir.strip_prefix(base) {
        Ok(rel) if rel.components().all(|c| matches!(c, Component::Normal(_))) => rel.into(),
        _ => PathBuf::new(),
    }
}

/// A read from a cell that gives up at `until`, so a request that trickles in cannot hold its thread for long.
struct Deadline<'a> {
    conn: &'a UnixStream,
    until: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.conn.set_read_timeout(Some(left))?;
        let mut conn = self.conn;
        conn.read(buf)
    }
}

fn read_request(conn: &UnixStream) -> Result<(Request, Vec<u8>)> {
    let mut r = BufReader::new(Deadline {
        conn,
        until: Instant::now() + REQUEST_TIME,
    });
    let mut line = Vec::new();
    let whole = (&mut r)
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut line)
        .is_ok_and(|_| line.last() == Some(&b'\n'));
    if !whole {
        return Err(Error::usage(if line.len() > MAX_LINE {
            "the request is too long"
        } else {
            "the request did not arrive whole"
        }));
    }
    let req: Request = serde_json::from_slice(&line)
        .map_err(|e| Error::usage(format!("the request cannot be read: {e}")))?;
    if req.input > MAX_INPUT as u64 {
        return Err(Error::usage(format!(
            "the input is larger than {}",
            cgroup::size(MAX_INPUT as u64)
        )));
    }
    let mut input = vec![0; req.input as usize];
    r.read_exact(&mut input)
        .map_err(|_| Error::usage("the input did not arrive whole"))?;
    Ok((req, input))
}

/// Sends one JSON line.
fn send(conn: &UnixStream, value: &serde_json::Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    let mut conn = conn;
    conn.write_all(&line)
}

/// What a cell's server threads share.
struct Shared {
    asker: Asker,
    outbox: Outbox,
    ledger: Ledger,
}

impl Shared {
    /// Stores what the cell asked for, with its step in the ledger.
    fn ask(&self, req: Request, input: &[u8]) -> Result<Effect> {
        let started = Instant::now();
        check(&req)?;
        let a = &self.asker;
        if let Some(k) = a.keys.first()
            && !key::current(k)
        {
            return Err(Error::refused(format!("key '{}' was taken back", k.name)));
        }
        if req.cost.is_none()
            && let Some((k, most)) = present(&a.keys)
                .into_iter()
                .find_map(|k| Some((k.name, k.spend?)))
        {
            return Err(Error::usage(format!(
                "key '{k}' may spend {} in all, so say what this costs: arbor effect --cost AMOUNT -- CMD",
                money(most)
            )));
        }
        let e = self.outbox.add(
            Effect {
                id: 0,
                world: a.world.clone(),
                cell: a.cell.clone(),
                keys: a.keys.clone(),
                base: a.base.clone(),
                dir: inside(&a.base, Path::new(&req.dir)),
                cmd: req.cmd,
                input: None,
                cost: req.cost,
                why: req.why,
                back: req.wait,
                asked_ns: 0,
                answered_ns: None,
                state: State::Waiting,
                refusal: String::new(),
                receipt: None,
            },
            input,
        )?;
        let step = self.ledger.append(ledger::New {
            act: "effect",
            by: "cell",
            key: e.key(),
            world: &e.world,
            cell: Some(&e.cell),
            effect: Some(e.id),
            what: &e.shown(),
            limits: None,
            out: "waiting",
            cost: Cost {
                ms: started.elapsed().as_millis() as u64,
                ..Cost::default()
            },
        });
        if let Err(err) = step {
            eprintln!(
                "arbor: warning: effect {} is not in the ledger: {err}",
                e.id
            );
        }
        Ok(e)
    }

    /// Answers one connection: a request, its effect's number, and with `wait` the answer once a person gives it.
    fn answer(&self, conn: UnixStream, stop: BorrowedFd) {
        let _ = conn.set_write_timeout(Some(REQUEST_TIME));
        let asked = read_request(&conn).and_then(|(req, input)| self.ask(req, &input));
        let e = match asked {
            Ok(e) => e,
            Err(err) => {
                let _ = send(&conn, &json!({ "error": err.msg, "code": err.code }));
                return;
            }
        };
        if send(&conn, &json!({ "id": e.id })).is_err() || !e.back {
            return;
        }
        // Watching starts before the first look, so an answer given in between is not missed.
        let Ok(watch) = sys::DirWatch::new(self.outbox.dir()) else {
            return;
        };
        loop {
            let Ok(now) = self.outbox.load(e.id) else {
                return;
            };
            let answer = match (now.state, &now.receipt) {
                (State::Done, Some(r)) => {
                    let mut output = Vec::new();
                    if let Ok(f) = File::open(self.outbox.output_file(e.id)) {
                        let _ = f.take(KEPT_OUTPUT as u64).read_to_end(&mut output);
                    }
                    Some((
                        json!({ "state": "done", "exit": r.exit, "output": output.len() }),
                        output,
                    ))
                }
                (State::Refused, _) => Some((
                    json!({ "state": "refused", "why": now.refusal }),
                    Vec::new(),
                )),
                _ => None,
            };
            if let Some((head, output)) = answer {
                let _ = send(&conn, &head).and_then(|()| (&mut &conn).write_all(&output));
                return;
            }
            // A command that gave up waiting, or a cell that ended, ends the wait.
            if !watch.wait_or(&[conn.as_fd(), stop]).unwrap_or(false) {
                return;
            }
        }
    }
}

/// Removes a file when dropped.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A cell's socket to the outbox, made before the cell starts so that bwrap can show it inside, and removed when
/// dropped.
pub struct Socket {
    listener: UnixListener,
    path: Removed,
}

impl Socket {
    pub fn bind(path: &Path) -> Result<Socket> {
        let _ = fs::remove_file(path); // left by a crash
        let made = || format!("cannot make the outbox socket {}", path.display());
        // A socket's address holds at most 107 bytes, and a state folder can be deeper than that: the socket is
        // made through the folder's descriptor, which names the folder in a few bytes whatever its depth.
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Error::new(made()));
        };
        let dir = File::open(dir).ctx(made)?;
        let short = Path::new("/proc/self/fd")
            .join(dir.as_raw_fd().to_string())
            .join(name);
        let listener = UnixListener::bind(short).ctx(made)?;
        Ok(Socket {
            listener,
            path: Removed(path.into()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path.0
    }

    /// Answers what the cell asks, on threads of their own, until the returned server is dropped.
    pub fn serve(self, asker: Asker, outbox: Outbox, ledger: Ledger) -> Result<Server> {
        outbox.create()?;
        self.listener.set_nonblocking(true)?;
        let (stop, stopping) = sys::pipe()?;
        let shared = Arc::new(Shared {
            asker,
            outbox,
            ledger,
        });
        let Socket { listener, path } = self;
        let thread = sys::spawn_quiet(move || accept(&listener, &stop, &shared))?;
        Ok(Server {
            stopping: Some(stopping),
            thread: Some(thread),
            _path: path,
        })
    }
}

/// Takes the cell's connections, a few at a time, until `stop` closes.
fn accept(listener: &UnixListener, stop: &OwnedFd, shared: &Arc<Shared>) {
    let mut open: Vec<JoinHandle<()>> = Vec::new();
    loop {
        match sys::ready(&[listener.as_fd(), stop.as_fd()], None) {
            Ok(ready) if !ready[1] => {}
            _ => break,
        }
        let conn = match listener.accept() {
            Ok((conn, _)) => conn,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(_) => {
                // Out of descriptors, say: wait instead of spinning.
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        open.retain(|t| !t.is_finished());
        let _ = conn.set_nonblocking(false);
        if open.len() >= MAX_OPEN {
            let _ = conn.set_write_timeout(Some(REQUEST_TIME));
            let _ = send(
                &conn,
                &json!({ "error": "this world has too many requests open at once", "code": 3 }),
            );
            continue;
        }
        let Ok(stop) = stop.try_clone() else { continue };
        let shared = shared.clone();
        if let Ok(t) = sys::spawn_quiet(move || shared.answer(conn, stop.as_fd())) {
            open.push(t);
        }
    }
    for t in open {
        let _ = t.join();
    }
}

/// The threads that answer a cell. Dropping it ends them, once the cell has ended, and removes the socket.
pub struct Server {
    stopping: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
    _path: Removed,
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stopping.take()); // closing it ends every thread's wait
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What `arbor effect` asks for.
pub struct Ask {
    pub cmd: Vec<String>,
    /// Where the asker stands.
    pub dir: String,
    pub cost: Option<Cents>,
    pub why: String,
    pub input: Vec<u8>,
    pub wait: bool,
}

/// A person's answer, for a command that waits for it.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Released: how the command ended and what it printed, up to `KEPT_OUTPUT` bytes.
    Done {
        exit: i32,
        output: Vec<u8>,
    },
    Refused {
        why: String,
    },
}

/// Asks the outbox behind `socket`, from inside a cell. `asked` is told the effect's number as soon as it waits;
/// with `wait`, the answer follows once a person gives it.
pub fn ask(socket: &Path, a: &Ask, asked: &mut dyn FnMut(u64)) -> Result<Option<Answer>> {
    let conn = UnixStream::connect(socket).ctx(|| "cannot reach the outbox".to_string())?;
    let head = json!({
        "cmd": a.cmd, "dir": a.dir, "cost": a.cost, "why": a.why, "input": a.input.len(), "wait": a.wait,
    });
    send(&conn, &head)
        .and_then(|()| (&mut &conn).write_all(&a.input))
        .ctx(|| "cannot reach the outbox".to_string())?;
    let mut r = BufReader::new(&conn);
    let mut next = || -> Result<serde_json::Value> {
        let mut line = String::new();
        r.read_line(&mut line)?;
        serde_json::from_str(&line).map_err(|_| Error::new("the outbox stopped answering"))
    };
    let first = next()?;
    if let Some(msg) = first["error"].as_str() {
        return Err(Error {
            msg: msg.to_string(),
            code: first["code"].as_i64().unwrap_or(1) as i32,
        });
    }
    let id = first["id"]
        .as_u64()
        .ok_or_else(|| Error::new("the outbox gave no number"))?;
    asked(id);
    if !a.wait {
        return Ok(None);
    }
    let answer = next().map_err(|_| {
        Error::new(format!(
            "the outbox stopped before effect {id} was answered"
        ))
    })?;
    match answer["state"].as_str() {
        Some("done") => {
            let n = answer["output"]
                .as_u64()
                .unwrap_or(0)
                .min(KEPT_OUTPUT as u64);
            let mut output = vec![0; n as usize];
            r.read_exact(&mut output)?;
            Ok(Some(Answer::Done {
                exit: answer["exit"].as_i64().unwrap_or(1) as i32,
                output,
            }))
        }
        Some("refused") => Ok(Some(Answer::Refused {
            why: answer["why"].as_str().unwrap_or_default().to_string(),
        })),
        _ => Err(Error::new(format!(
            "the outbox's answer to effect {id} cannot be read"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cgroup::Limits;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arbor-outbox-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("base/src")).unwrap();
        fs::canonicalize(dir).unwrap()
    }

    fn asker(dir: &Path) -> Asker {
        Asker {
            world: "demo".into(),
            base: dir.join("base"),
            cell: "0badcafe".into(),
            keys: Vec::new(),
        }
    }

    fn ask_for(cmd: &[&str]) -> Ask {
        Ask {
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            dir: String::new(),
            cost: None,
            why: String::new(),
            input: Vec::new(),
            wait: false,
        }
    }

    #[test]
    fn reads_amounts_of_money() {
        for (text, cents) in [
            ("5", 500),
            ("2.50", 250),
            ("2.5", 250),
            ("$0.40", 40),
            ("0.05", 5),
            (" 12 ", 1200),
        ] {
            assert_eq!(parse_money(text).unwrap(), cents, "{text}");
        }
        for bad in [
            "",
            "$",
            "5.",
            ".5",
            "1.234",
            "-1",
            "1e3",
            "five",
            "1,000",
            "1234567890",
        ] {
            assert!(parse_money(bad).is_err(), "{bad}");
        }
        assert_eq!(money(5), "$0.05");
        assert_eq!(money(123_456), "$1234.56");
    }

    #[test]
    fn a_budget_counts_what_its_keys_and_the_keys_within_them_released() {
        let key = |name: &str, made_ns, spend| Key {
            name: name.into(),
            within: None,
            worlds: Vec::new(),
            read: Vec::new(),
            env: Vec::new(),
            limits: Limits::default(),
            spend,
            made_ns,
            until_ns: u64::MAX,
        };
        let (dev, tests) = (key("dev", 1, Some(500)), key("tests", 2, Some(300)));
        let effect = |id, keys: Vec<KeyRef>, cost, state| Effect {
            id,
            world: "demo".into(),
            cell: "0badcafe".into(),
            keys,
            base: PathBuf::from("/x"),
            dir: PathBuf::new(),
            cmd: vec!["true".into()],
            input: None,
            cost: Some(cost),
            why: String::new(),
            back: false,
            asked_ns: 0,
            answered_ns: None,
            state,
            refusal: String::new(),
            receipt: None,
        };
        let both = vec![tests.id(), dev.id()];
        let all = vec![
            effect(1, vec![dev.id()], 100, State::Done),
            effect(2, both.clone(), 150, State::Running),
            effect(3, both.clone(), 1000, State::Refused),
            effect(4, both.clone(), 1000, State::Waiting),
            // An older key that had the same name is another key.
            effect(
                5,
                vec![KeyRef {
                    name: "dev".into(),
                    made_ns: 0,
                }],
                999,
                State::Done,
            ),
        ];
        assert_eq!(spent(&all, &dev.id()), 250);
        assert_eq!(spent(&all, &tests.id()), 150);
        let keys = [tests.clone(), dev.clone()];
        // tests has 150 of 300 left; dev has 250 of 500.
        assert_eq!(
            over_budget(&effect(6, both.clone(), 150, State::Waiting), &all, &keys),
            None
        );
        let over = over_budget(&effect(6, both.clone(), 151, State::Waiting), &all, &keys).unwrap();
        assert_eq!(
            over,
            "effect 6 costs $1.51, and key 'tests' has $1.50 left of $3.00"
        );
        let mut wide = tests.clone();
        wide.spend = Some(10_000);
        let over = over_budget(
            &effect(6, both.clone(), 251, State::Waiting),
            &all,
            &[wide, dev],
        )
        .unwrap();
        assert!(over.contains("key 'dev' has $2.50 left of $5.00"), "{over}");
        // Under a budget, an effect must say what it costs; without one, nothing is over.
        let mut unsaid = effect(6, both.clone(), 0, State::Waiting);
        unsaid.cost = None;
        assert_eq!(
            over_budget(&unsaid, &all, &keys).unwrap(),
            "effect 6 does not say what it costs, and key 'tests' may spend $3.00 in all"
        );
        let free = [key("tests", 2, None)];
        assert_eq!(over_budget(&unsaid, &all, &free), None);
        assert_eq!(
            over_budget(&effect(6, both, 99_999, State::Waiting), &all, &free),
            None
        );
    }

    #[test]
    fn a_request_is_checked_and_its_folder_kept_inside_the_world() {
        let req = |cmd: &[&str], why: &str, cost| Request {
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            dir: String::new(),
            cost,
            why: why.into(),
            input: 0,
            wait: false,
        };
        assert!(check(&req(&["git", "push"], "ship it", Some(100))).is_ok());
        for bad in [
            req(&[], "", None),
            req(&[""], "", None),
            req(&["a\0b"], "", None),
            req(&["true"], &"x".repeat(MAX_WHY + 1), None),
            req(&["true"], "", Some(100_000_000_000)),
            req(&vec!["x"; MAX_ARGS + 1], "", None),
        ] {
            assert!(
                check(&bad).is_err_and(|e| e.code == crate::error::USAGE),
                "{bad:?}"
            );
        }
        let base = Path::new("/home/u/shop");
        assert_eq!(
            inside(base, Path::new("/home/u/shop/src/a")),
            PathBuf::from("src/a")
        );
        assert_eq!(inside(base, Path::new("/home/u/shop")), PathBuf::new());
        for elsewhere in ["/tmp", "/home/u/shop/../x", "/home/u/shopping", "src", ""] {
            assert_eq!(
                inside(base, Path::new(elsewhere)),
                PathBuf::new(),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn a_cell_asks_through_its_socket_and_hears_the_answer() {
        let dir = temp("ask");
        let outbox = Outbox::at(dir.join("outbox"));
        let ledger = Ledger::at(dir.join("ledger.jsonl"));
        let path = dir.join("cell.sock");
        let server = Socket::bind(&path)
            .unwrap()
            .serve(
                asker(&dir),
                Outbox::at(dir.join("outbox")),
                Ledger::at(dir.join("ledger.jsonl")),
            )
            .unwrap();

        // An effect waits, numbered, with its step in the ledger.
        let mut a = ask_for(&["sh", "-c", "echo hi; cat"]);
        (a.dir, a.cost, a.why, a.input) = (
            dir.join("base/src").display().to_string(),
            Some(250),
            "say hi\x1b[2J".into(),
            b"from the world".to_vec(),
        );
        let mut got = None;
        assert_eq!(ask(&path, &a, &mut |id| got = Some(id)).unwrap(), None);
        assert_eq!(got, Some(1));
        let e = outbox.load(1).unwrap();
        assert_eq!(
            (
                e.state,
                e.world.as_str(),
                e.cell.as_str(),
                e.dir.as_path(),
                e.cost,
                e.input
            ),
            (
                State::Waiting,
                "demo",
                "0badcafe",
                Path::new("src"),
                Some(250),
                Some(14)
            )
        );
        assert_eq!(fs::read(outbox.input_file(1)).unwrap(), b"from the world");
        let step = ledger.last(1).unwrap().remove(0);
        assert_eq!(
            (
                step.act.as_str(),
                step.by.as_str(),
                step.effect,
                step.out.as_str()
            ),
            ("effect", "cell", Some(1), "waiting")
        );
        assert_eq!(step.what, "sh -c 'echo hi; cat'");

        // Released, it runs once, in its folder, with its input; the waiting asker gets what it printed.
        a.wait = true;
        let waiting = {
            let path = path.clone();
            std::thread::spawn(move || ask(&path, &a, &mut |_| {}))
        };
        let wait_for = |id| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while outbox.load(id).is_err() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait_for(2);
        let (e, out) = outbox.begin(2).unwrap();
        assert!(
            outbox
                .begin(2)
                .is_err_and(|e| e.code == crate::error::REFUSED)
        );
        assert!(outbox.still_running(2));
        let mut shown = Vec::new();
        let receipt = run(&outbox, &e, out, &mut shown).unwrap();
        assert_eq!(shown, b"hi\nfrom the world");
        assert_eq!((receipt.exit, receipt.output), (0, 17));
        let digest: String = Sha256::digest(b"hi\nfrom the world")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(receipt.sha256, digest);
        outbox.finish(2, receipt).unwrap();
        assert_eq!(
            waiting.join().unwrap().unwrap(),
            Some(Answer::Done {
                exit: 0,
                output: b"hi\nfrom the world".to_vec()
            })
        );
        assert!(
            outbox
                .begin(2)
                .is_err_and(|e| e.msg.contains("runs only once"))
        );

        // Refused, the waiting asker hears why.
        let waiting = {
            let path = path.clone();
            let mut a = ask_for(&["false"]);
            a.wait = true;
            std::thread::spawn(move || ask(&path, &a, &mut |_| {}))
        };
        wait_for(3);
        outbox.refuse(3, "not today").unwrap();
        assert_eq!(
            waiting.join().unwrap().unwrap(),
            Some(Answer::Refused {
                why: "not today".into()
            })
        );
        assert!(outbox.refuse(3, "again").is_err());

        // A command that cannot start leaves its effect waiting.
        ask(&path, &ask_for(&["/no/such/program"]), &mut |_| {}).unwrap();
        let (e, out) = outbox.begin(4).unwrap();
        assert!(run(&outbox, &e, out, &mut Vec::new()).is_err());
        outbox.unbegin(4);
        assert_eq!(outbox.load(4).unwrap().state, State::Waiting);
        drop(server);
        assert!(!path.exists(), "the socket was left behind");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_a_cell_sends_is_capped() {
        let dir = temp("caps");
        let path = dir.join("cell.sock");
        let outbox = Outbox::at(dir.join("outbox"));
        let _server = Socket::bind(&path)
            .unwrap()
            .serve(
                asker(&dir),
                Outbox::at(dir.join("outbox")),
                Ledger::at(dir.join("ledger.jsonl")),
            )
            .unwrap();
        let raw = |bytes: &[u8]| -> serde_json::Value {
            let conn = UnixStream::connect(&path).unwrap();
            let _ = (&conn).write_all(bytes);
            let _ = conn.shutdown(std::net::Shutdown::Write);
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        for (bytes, says) in [
            (b"not json\n".to_vec(), "cannot be read"),
            (
                b"{\"cmd\":[\"true\"],\"by\":\"you\"}\n".to_vec(),
                "cannot be read",
            ),
            (b"{\"cmd\":[\"true\"]}".to_vec(), "did not arrive whole"),
            (vec![b'x'; MAX_LINE + 10], "too long"),
            (
                format!("{{\"cmd\":[\"true\"],\"input\":{}}}\n", MAX_INPUT + 1).into_bytes(),
                "larger than",
            ),
            (
                b"{\"cmd\":[\"true\"],\"input\":5}\nab".to_vec(),
                "did not arrive whole",
            ),
        ] {
            let answer = raw(&bytes);
            assert!(answer["error"].as_str().unwrap().contains(says), "{answer}");
            assert_eq!(answer["code"], 2);
        }
        // One world may have only so many effects waiting.
        for _ in 0..MAX_WAITING {
            ask(&path, &ask_for(&["true"]), &mut |_| {}).unwrap();
        }
        let full = ask(&path, &ask_for(&["true"]), &mut |_| {}).unwrap_err();
        assert_eq!(full.code, crate::error::REFUSED);
        assert_eq!(outbox.list().unwrap().len(), MAX_WAITING);
        fs::remove_dir_all(dir).unwrap();
    }
}
