//! What Mission Control shows, read from arbor's own records through its library: the worlds (tries), the cells
//! running in them (rooms) with what each uses, the effects waiting in the outbox (decisions), and the newest steps of
//! the ledger. Reading changes none of them. It runs off the main thread, so a slow disk never holds a frame up.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use arbor::error::Result;
use arbor::ledger::{Ledger, Step};
use arbor::outbox::{self, Effect, Outbox};
use arbor::world::{self, State, World};
use arbor::{cell, cgroup, paths, tree};

/// A world, as the window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Try {
    pub name: String,
    /// The real folder it is a copy of.
    pub base: PathBuf,
    pub since_ns: u64,
    /// When it was kept; `None` while it is open.
    pub kept_ns: Option<u64>,
    /// Files added, changed and deleted, while it is open; `None` when they could not be read.
    pub changes: Option<usize>,
}

/// A running cell, with what it used when it was read.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub id: String,
    pub world: String,
    pub what: String,
    pub key: Option<String>,
    pub limits: String,
    pub started_ns: u64,
    /// Memory in use, and the CPU time used so far in microseconds, where the cell has a cgroup to read them from.
    pub mem: Option<u64>,
    pub cpu_us: Option<u64>,
    /// The share of one CPU it used since the reading before, in percent.
    pub cpu_pct: Option<f64>,
}

/// Everything one look at arbor's state found.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub tries: Vec<Try>,
    pub rooms: Vec<Room>,
    /// The effects waiting for a person, oldest first.
    pub waiting: Vec<Effect>,
    /// The newest steps, oldest first.
    pub steps: Vec<Step>,
    /// When it was read, in nanoseconds since 1970.
    pub now_ns: u64,
    /// What could not be read, in words.
    pub problems: Vec<String>,
}

/// How much to read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Everything: the ledger moved.
    All,
    /// Only the rooms and what they use: nothing was recorded, but their memory and CPU change as they run.
    Rooms,
}

/// The reader, which keeps what it needs from one reading to the next: where it got to in the ledger and each
/// room's CPU time.
pub struct Reader {
    tail: Tail,
    cpu: HashMap<String, (u64, u64)>,
    last: Snapshot,
}

impl Reader {
    /// A reader that keeps the newest `steps` steps of the ledger.
    pub fn new(steps: usize) -> Reader {
        let path = Ledger::open()
            .map(|l| l.path().to_path_buf())
            .unwrap_or_default();
        Reader {
            tail: Tail::new(path, steps),
            cpu: HashMap::new(),
            last: Snapshot::default(),
        }
    }

    /// Whether the ledger has more than this reader read: a step recorded since, or one still being written.
    pub fn behind(&self) -> bool {
        std::fs::metadata(&self.tail.path).is_ok_and(|m| m.len() != self.tail.at)
    }

    /// Reads arbor's state again, all of it or only the rooms.
    pub fn read(&mut self, kind: Kind) -> Snapshot {
        let mut snap = if kind == Kind::Rooms {
            self.last.clone()
        } else {
            Snapshot::default()
        };
        snap.problems.clear();
        snap.now_ns = world::now_ns();
        if kind == Kind::All {
            match tries() {
                Ok(t) => snap.tries = t,
                Err(e) => snap.problems.push(e.to_string()),
            }
            match waiting() {
                Ok(w) => snap.waiting = w,
                Err(e) => snap.problems.push(e.to_string()),
            }
            match self.tail.read() {
                Ok(()) => snap.steps = self.tail.steps.iter().cloned().collect(),
                Err(e) => snap.problems.push(format!("cannot read the ledger: {e}")),
            }
        }
        match self.rooms(snap.now_ns) {
            Ok(r) => snap.rooms = r,
            Err(e) => snap.problems.push(e.to_string()),
        }
        self.last = snap.clone();
        snap
    }

    fn rooms(&mut self, now_ns: u64) -> Result<Vec<Room>> {
        let mut seen = HashMap::new();
        let rooms = cell::running_seen()?
            .into_iter()
            .map(|c| {
                let (mem, cpu_us) = match &c.cgroup {
                    Some(dir) => (cgroup::memory_now(dir), cgroup::read_usage(dir).cpu_us),
                    None => (None, None),
                };
                let cpu_pct = cpu_us.and_then(|us| {
                    let (was, at) = *self.cpu.get(&c.id)?;
                    let wall_us = now_ns.saturating_sub(at) / 1000;
                    (wall_us > 0).then(|| us.saturating_sub(was) as f64 * 100.0 / wall_us as f64)
                });
                if let Some(us) = cpu_us {
                    seen.insert(c.id.clone(), (us, now_ns));
                }
                Room {
                    id: c.id,
                    world: c.world,
                    what: c.what,
                    key: c.key,
                    limits: c.limits,
                    started_ns: c.started_ns,
                    mem,
                    cpu_us,
                    cpu_pct,
                }
            })
            .collect();
        self.cpu = seen;
        Ok(rooms)
    }
}

fn tries() -> Result<Vec<Try>> {
    Ok(World::list()?
        .into_iter()
        .map(|w| Try {
            changes: match w.state {
                State::Open => tree::entries(&w)
                    .and_then(|e| tree::changes(&w, &e))
                    .ok()
                    .map(|c| c.len()),
                State::Kept => None,
            },
            kept_ns: match w.state {
                State::Open => None,
                State::Kept => Some(w.kept_ns.unwrap_or(w.since_ns)),
            },
            name: w.name,
            base: w.base,
            since_ns: w.since_ns,
        })
        .collect())
}

fn waiting() -> Result<Vec<Effect>> {
    Ok(Outbox::open()?
        .list()?
        .into_iter()
        .filter(|e| e.state == outbox::State::Waiting)
        .collect())
}

/// The ledger's newest steps, read from where the last reading stopped, so a long ledger costs nothing to follow.
struct Tail {
    path: PathBuf,
    keep: usize,
    at: u64,
    steps: VecDeque<Step>,
}

/// How far back the first reading of a long ledger starts: its lines are short, so this holds far more than a
/// screen of steps.
const FIRST_READ: u64 = 256 * 1024;

impl Tail {
    fn new(path: PathBuf, keep: usize) -> Tail {
        Tail {
            path,
            keep,
            at: 0,
            steps: VecDeque::new(),
        }
    }

    fn read(&mut self) -> io::Result<()> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                (self.at, self.steps) = (0, VecDeque::new());
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        if len < self.at {
            (self.at, self.steps) = (0, VecDeque::new()); // rewritten: read it again
        }
        let mut skip_first = false;
        if self.at == 0 && len > FIRST_READ {
            self.at = len - FIRST_READ;
            skip_first = true; // it starts mid-line
        }
        file.seek(SeekFrom::Start(self.at))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        // Whole lines only: one still being written is read once it is done.
        let whole = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let mut lines = buf[..whole].split(|&b| b == b'\n');
        if skip_first {
            lines.next();
        }
        let newest = self.steps.back().map_or(0, |s| s.seq);
        for line in lines {
            // A line is a step with its hash as one more field, which a step does not keep.
            if let Ok(step) = serde_json::from_slice::<Step>(line)
                && step.seq > newest
            {
                self.steps.push_back(step);
                if self.steps.len() > self.keep {
                    self.steps.pop_front();
                }
            }
        }
        self.at += whole as u64;
        Ok(())
    }
}

/// The folder to watch for the ledger: arbor's state folder, or the nearest folder above it that exists, until arbor
/// makes it.
pub fn watched() -> Option<(PathBuf, bool)> {
    let ledger = Ledger::open().ok()?.path().to_path_buf();
    let state = ledger.parent()?.to_path_buf();
    if state.is_dir() {
        return Some((ledger, true));
    }
    let mut up: &Path = &state;
    while let Some(parent) = up.parent() {
        if parent.is_dir() {
            return Some((parent.to_path_buf(), false));
        }
        up = parent;
    }
    None
}

/// A path as people read it: under the home folder, with `~`.
pub fn shown_path(path: &Path) -> String {
    paths::tilde(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(seq: u64) -> String {
        format!(
            r#"{{"seq":{seq},"ts":{seq}000,"by":"you","act":"new","world":"w","what":"~/w","out":"ok","cost":{{"ms":0}},"hash":"00"}}"#
        )
    }

    #[test]
    fn the_tail_reads_only_what_was_added() {
        let dir = std::env::temp_dir().join(format!("mission-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger.jsonl");
        let mut f = File::create(&path).unwrap();
        for seq in 1..=5 {
            writeln!(f, "{}", line(seq)).unwrap();
        }
        let mut tail = Tail::new(path.clone(), 3);
        tail.read().unwrap();
        let seqs = |t: &Tail| t.steps.iter().map(|s| s.seq).collect::<Vec<_>>();
        assert_eq!(seqs(&tail), [3, 4, 5]);
        // half a line is left for later, as a step still being written
        write!(f, "{}\n{}", line(6), &line(7)[..20]).unwrap();
        tail.read().unwrap();
        assert_eq!(seqs(&tail), [4, 5, 6]);
        writeln!(f, "{}", &line(7)[20..]).unwrap();
        tail.read().unwrap();
        assert_eq!(seqs(&tail), [5, 6, 7]);
        // a ledger written anew is read again from its start
        let mut f = File::create(&path).unwrap();
        writeln!(f, "{}", line(1)).unwrap();
        tail.read().unwrap();
        assert_eq!(seqs(&tail), [1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
