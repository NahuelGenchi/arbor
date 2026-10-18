//! What Mission Control shows, read from arbor's own records through its library: the worlds (tries), the cells
//! running in them (rooms) with what each uses, the effects waiting in the outbox (decisions), and the newest steps of
//! the ledger. Reading changes none of them. It runs off the main thread, so a slow disk never holds a frame up.
//!
//! The first reading reads everything. After it, the reader follows the ledger: every step that changes a world, a
//! cell or the outbox is recorded there and names what it changed, so a reading reads again only the worlds, the
//! cells and the outbox that the new steps name. What the rooms use, and the files they change in their tries, move
//! with no step recorded, so those are read at the room sample instead (`Kind::ROOMS`), with every running cell.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use arbor::cell::{self, Running};
use arbor::error::Result;
use arbor::ledger::{Ledger, Step};
use arbor::outbox::{self, Effect, Outbox};
use arbor::world::{self, State, World};
use arbor::{cgroup, paths, tree};

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
    /// Of those, how many were added, changed and deleted.
    pub counts: [usize; 3],
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
    /// The share of one CPU it used since the sample before, in percent.
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

/// What a reading reads again. Asks made before a reading starts fold into one (`and`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kind {
    /// What the ledger's new steps name: the ledger moved.
    pub ledger: bool,
    /// Every running cell, what each uses, and the changes in the tries they work in, which move with no step.
    pub rooms: bool,
}

impl Kind {
    pub const LEDGER: Kind = Kind {
        ledger: true,
        rooms: false,
    };
    pub const ROOMS: Kind = Kind {
        ledger: false,
        rooms: true,
    };

    pub fn and(self, other: Kind) -> Kind {
        Kind {
            ledger: self.ledger || other.ledger,
            rooms: self.rooms || other.rooms,
        }
    }
}

/// The reader, which keeps what it read from one reading to the next: where it got to in the ledger, the worlds,
/// the rooms and each room's CPU time.
pub struct Reader {
    tail: Tail,
    /// The worlds as last read, by name, to read their changes again.
    worlds: HashMap<String, World>,
    cpu: HashMap<String, (u64, u64)>,
    /// What the last reading found: the next one starts from it.
    last: Snapshot,
    problems: Problems,
    /// Whether everything has been read once.
    whole: bool,
    /// Whether the last reading found new steps.
    moved: bool,
}

/// What could not be read, kept until it is read again.
#[derive(Default)]
struct Problems {
    tries: Option<String>,
    waiting: Option<String>,
    ledger: Option<String>,
    rooms: Option<String>,
}

impl Reader {
    /// A reader that keeps the newest `steps` steps of the ledger.
    pub fn new(steps: usize) -> Reader {
        let path = Ledger::open()
            .map(|l| l.path().to_path_buf())
            .unwrap_or_default();
        Reader {
            tail: Tail::new(path, steps),
            worlds: HashMap::new(),
            cpu: HashMap::new(),
            last: Snapshot::default(),
            problems: Problems::default(),
            whole: false,
            moved: false,
        }
    }

    /// Whether the ledger has more than this reader read: a step recorded since, or one still being written.
    pub fn behind(&self) -> bool {
        std::fs::metadata(&self.tail.path).is_ok_and(|m| m.len() != self.tail.at)
    }

    /// Whether the last reading found new steps in the ledger: while it keeps finding them, the ledger is moving.
    pub fn moved(&self) -> bool {
        self.moved
    }

    /// Reads arbor's state again: everything the first time, then what `kind` asks for.
    pub fn read(&mut self, kind: Kind) -> Snapshot {
        let now = world::now_ns();
        let mut touched = Touched::default();
        if kind.ledger || !self.whole {
            touched = match self.tail.read() {
                Ok(t) => {
                    self.problems.ledger = None;
                    t
                }
                Err(e) => {
                    self.problems.ledger = Some(format!("cannot read the ledger: {e}"));
                    Touched::everything()
                }
            };
            self.last.steps = self.tail.steps.iter().cloned().collect();
        }
        self.moved = touched.steps > 0;
        if touched.all || !self.whole {
            self.read_tries();
            self.read_waiting();
            self.read_rooms(now, true);
            self.whole = true;
        } else {
            for name in &touched.worlds {
                self.read_try(name);
            }
            if touched.outbox {
                self.read_waiting();
            }
            if touched.rooms || kind.rooms {
                self.read_rooms(now, kind.rooms);
            } else {
                for id in &touched.cells {
                    self.read_room(id, now);
                }
            }
            if kind.rooms {
                self.read_changes();
            }
        }
        self.last.now_ns = now;
        self.last.problems = [
            &self.problems.tries,
            &self.problems.waiting,
            &self.problems.ledger,
            &self.problems.rooms,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        self.last.clone()
    }

    /// Every world.
    fn read_tries(&mut self) {
        match World::list() {
            Ok(worlds) => {
                self.problems.tries = None;
                self.last.tries = worlds.iter().map(try_of).collect();
                self.worlds = worlds.into_iter().map(|w| (w.name.clone(), w)).collect();
            }
            Err(e) => self.problems.tries = Some(e.to_string()),
        }
    }

    /// One world, by name: read again, added, or gone. Like `World::list`, a world that cannot be read is left out.
    fn read_try(&mut self, name: &str) {
        let at = self
            .last
            .tries
            .binary_search_by(|t| t.name.as_str().cmp(name));
        match (World::load(name), at) {
            (Ok(w), Ok(i)) => {
                self.last.tries[i] = try_of(&w);
                self.worlds.insert(w.name.clone(), w);
            }
            (Ok(w), Err(i)) => {
                self.last.tries.insert(i, try_of(&w));
                self.worlds.insert(w.name.clone(), w);
            }
            (Err(_), at) => {
                if let Ok(i) = at {
                    self.last.tries.remove(i);
                }
                self.worlds.remove(name);
            }
        }
    }

    /// What changed in the open tries that rooms work in.
    fn read_changes(&mut self) {
        let working: HashSet<&str> = self.last.rooms.iter().map(|r| r.world.as_str()).collect();
        for t in &mut self.last.tries {
            if t.kept_ns.is_none()
                && working.contains(t.name.as_str())
                && let Some(w) = self.worlds.get(&t.name)
            {
                (t.changes, t.counts) = changes(w);
            }
        }
    }

    fn read_waiting(&mut self) {
        match waiting() {
            Ok(w) => {
                self.problems.waiting = None;
                self.last.waiting = w;
            }
            Err(e) => self.problems.waiting = Some(e.to_string()),
        }
    }

    /// Every running cell. With `usage`, what each uses is read from its cgroup; without, a room keeps what it was
    /// last read to use, and only a new one is read.
    fn read_rooms(&mut self, now: u64, usage: bool) {
        let cells = match cell::running_seen() {
            Ok(c) => c,
            Err(e) => {
                self.problems.rooms = Some(e.to_string());
                return;
            }
        };
        self.problems.rooms = None;
        let mut old: HashMap<String, Room> = self
            .last
            .rooms
            .drain(..)
            .map(|r| (r.id.clone(), r))
            .collect();
        let mut rooms = Vec::with_capacity(cells.len());
        for c in cells {
            let room = match old.remove(&c.id) {
                Some(r) if !usage => r,
                _ => self.room(c, now),
            };
            rooms.push(room);
        }
        if usage {
            let ids: HashSet<&str> = rooms.iter().map(|r| r.id.as_str()).collect();
            self.cpu.retain(|id, _| ids.contains(id.as_str()));
        }
        self.last.rooms = rooms;
    }

    /// One cell, by id, that a step started or ended: shown while it runs, with what it uses read when it starts.
    fn read_room(&mut self, id: &str, now: u64) {
        let at = self.last.rooms.iter().position(|r| r.id == id);
        match (cell::seen(id), at) {
            (Ok(Some(_)), Some(_)) => {}
            (Ok(Some(c)), None) => {
                let room = self.room(c, now);
                let i = self
                    .last
                    .rooms
                    .partition_point(|r| r.started_ns <= room.started_ns);
                self.last.rooms.insert(i, room);
            }
            (Ok(None), at) => {
                if let Some(i) = at {
                    self.last.rooms.remove(i);
                }
                self.cpu.remove(id);
            }
            // List them all instead, which says what went wrong, if it still does.
            (Err(_), _) => self.read_rooms(now, false),
        }
    }

    /// A room, with what it uses now, and its share of a CPU since the sample before.
    fn room(&mut self, c: Running, now_ns: u64) -> Room {
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
            self.cpu.insert(c.id.clone(), (us, now_ns));
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
    }
}

fn try_of(w: &World) -> Try {
    let (changes, counts) = match w.state {
        State::Open => changes(w),
        State::Kept => (None, [0; 3]),
    };
    Try {
        name: w.name.clone(),
        base: w.base.clone(),
        since_ns: w.since_ns,
        kept_ns: match w.state {
            State::Open => None,
            State::Kept => Some(w.kept_ns.unwrap_or(w.since_ns)),
        },
        changes,
        counts,
    }
}

/// How many files a world changes, and how many of them it adds, changes and deletes.
fn changes(w: &World) -> (Option<usize>, [usize; 3]) {
    match tree::entries(w).and_then(|e| tree::changes(w, &e)) {
        Ok(c) => (Some(c.len()), counts(&c)),
        Err(_) => (None, [0; 3]),
    }
}

/// Added, changed and deleted, in a list of changes.
pub fn counts(changes: &[tree::Change]) -> [usize; 3] {
    let n = |m| changes.iter().filter(|c| c.mark == m).count();
    [n('A'), n('M'), n('D')]
}

fn waiting() -> Result<Vec<Effect>> {
    Ok(Outbox::open()?
        .list()?
        .into_iter()
        .filter(|e| e.state == outbox::State::Waiting)
        .collect())
}

/// What new steps of the ledger may have changed, so what a reading reads again.
#[derive(Debug, Default, PartialEq, Eq)]
struct Touched {
    /// Everything: the steps before these were not read, the ledger was written anew, or a step was not understood.
    all: bool,
    /// The worlds the steps name.
    worlds: BTreeSet<String>,
    /// The cells that started or ended.
    cells: BTreeSet<String>,
    /// Every running cell: a key was taken back, and the cells under it were stopped.
    rooms: bool,
    /// The effects: one was asked for, released or refused.
    outbox: bool,
    /// How many new steps there were.
    steps: usize,
}

impl Touched {
    fn everything() -> Touched {
        Touched {
            all: true,
            ..Touched::default()
        }
    }

    fn add(&mut self, s: &Step) {
        self.steps += 1;
        if !s.world.is_empty() && s.world != "-" {
            self.worlds.insert(s.world.clone());
        }
        match s.act.as_str() {
            // A cell started, ended or was stopped. A command arbor refused to start names no cell.
            "start" | "run" | "shell" | "stop" => self.cells.extend(s.cell.clone()),
            // A discarded world's waiting effects are refused with it.
            "effect" | "release" | "refuse" | "discard" => self.outbox = true,
            "take-back" => (self.rooms, self.outbox) = (true, true),
            // The world they name is all they change; keys and a proxy's traffic are not shown.
            "new" | "keep" | "undo" | "key" | "renew" | "net" => {}
            _ => self.all = true,
        }
    }
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

    /// Reads the steps added since the last time, and says what they changed.
    fn read(&mut self) -> io::Result<Touched> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let gone = self.at > 0;
                (self.at, self.steps) = (0, VecDeque::new());
                return Ok(Touched {
                    all: gone,
                    ..Touched::default()
                });
            }
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        let mut touched = Touched::default();
        if len < self.at {
            (self.at, self.steps) = (0, VecDeque::new()); // rewritten: read it again
            touched.all = true;
        }
        let mut skip_first = false;
        if self.at == 0 && len > FIRST_READ {
            self.at = len - FIRST_READ;
            skip_first = true; // it starts mid-line
            touched.all = true; // and what the steps before it changed is not known
        }
        if len == self.at {
            return Ok(touched);
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
        for line in lines.filter(|l| !l.is_empty()) {
            // A line is a step with its hash as one more field, which a step does not keep.
            match serde_json::from_slice::<Step>(line) {
                Ok(step) if step.seq > newest => {
                    touched.add(&step);
                    self.steps.push_back(step);
                    if self.steps.len() > self.keep {
                        self.steps.pop_front();
                    }
                }
                Ok(_) => {}
                Err(_) => touched.all = true,
            }
        }
        self.at += whole as u64;
        Ok(touched)
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
    use arbor::ledger::{Cost, New};
    use std::fs;
    use std::io::Write;

    fn line(seq: u64) -> String {
        format!(
            r#"{{"seq":{seq},"ts":{seq}000,"by":"you","act":"new","world":"w","what":"~/w","out":"ok","cost":{{"ms":0}},"hash":"00"}}"#
        )
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mission-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_tail_reads_only_what_was_added() {
        let dir = temp("tail");
        let path = dir.join("ledger.jsonl");
        let mut f = File::create(&path).unwrap();
        for seq in 1..=5 {
            writeln!(f, "{}", line(seq)).unwrap();
        }
        let mut tail = Tail::new(path.clone(), 3);
        assert_eq!(tail.read().unwrap().steps, 5);
        let seqs = |t: &Tail| t.steps.iter().map(|s| s.seq).collect::<Vec<_>>();
        assert_eq!(seqs(&tail), [3, 4, 5]);
        // nothing new: nothing read
        assert_eq!(tail.read().unwrap(), Touched::default());
        // half a line is left for later, as a step still being written
        write!(f, "{}\n{}", line(6), &line(7)[..20]).unwrap();
        let t = tail.read().unwrap();
        assert_eq!((t.steps, t.all), (1, false));
        assert_eq!(seqs(&tail), [4, 5, 6]);
        writeln!(f, "{}", &line(7)[20..]).unwrap();
        tail.read().unwrap();
        assert_eq!(seqs(&tail), [5, 6, 7]);
        // a line it does not understand: everything is read again
        writeln!(f, "not a step").unwrap();
        assert!(tail.read().unwrap().all);
        // a ledger written anew is read again from its start
        let mut f = File::create(&path).unwrap();
        writeln!(f, "{}", line(1)).unwrap();
        let t = tail.read().unwrap();
        assert!(t.all);
        assert_eq!(seqs(&tail), [1]);
        // and a ledger that is gone
        fs::remove_file(&path).unwrap();
        assert!(tail.read().unwrap().all);
        assert!(!tail.read().unwrap().all);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn step(act: &str, world: &str, cell: Option<&str>) -> Step {
        Step {
            seq: 1,
            ts: 0,
            by: "you".into(),
            key: None,
            act: act.into(),
            world: world.into(),
            cell: cell.map(str::to_string),
            effect: None,
            what: String::new(),
            limits: None,
            out: "ok".into(),
            cost: Cost::default(),
        }
    }

    fn touched(steps: &[Step]) -> Touched {
        let mut t = Touched::default();
        for s in steps {
            t.add(s);
        }
        t
    }

    #[test]
    fn each_step_names_what_it_changed() {
        let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<BTreeSet<_>>();
        // a command: its world, and its cell when it starts and when it ends
        let t = touched(&[
            step("start", "shop", Some("0a1b2c3d")),
            step("run", "shop", Some("0a1b2c3d")),
            step("new", "site", None),
        ]);
        assert_eq!(
            (&t.worlds, &t.cells, t.rooms, t.outbox, t.all, t.steps),
            (
                &set(&["shop", "site"]),
                &set(&["0a1b2c3d"]),
                false,
                false,
                false,
                3
            )
        );
        // a command arbor refused to start, and a stop
        let t = touched(&[step("run", "shop", None), step("stop", "shop", Some("9f"))]);
        assert_eq!(t.cells, set(&["9f"]));
        // effects, and a key's steps, which name no world
        for act in ["effect", "release", "refuse"] {
            let t = touched(&[step(act, "-", Some("0a"))]);
            assert!(t.outbox && t.worlds.is_empty() && t.cells.is_empty());
        }
        let t = touched(&[step("key", "-", None), step("renew", "-", None)]);
        assert_eq!(
            t,
            Touched {
                steps: 2,
                ..Touched::default()
            }
        );
        // a discard refuses what its world asked for; a take-back stops cells and refuses effects
        assert!(touched(&[step("discard", "shop", None)]).outbox);
        let t = touched(&[step("take-back", "-", None)]);
        assert!(t.rooms && t.outbox && !t.all);
        // a step this reader does not know: everything
        assert!(touched(&[step("rename", "shop", None)]).all);
    }

    #[test]
    fn kinds_fold_into_one() {
        assert_eq!(
            Kind::LEDGER.and(Kind::ROOMS),
            Kind {
                ledger: true,
                rooms: true
            }
        );
        assert_eq!(Kind::default().and(Kind::ROOMS), Kind::ROOMS);
    }

    /// A cell's file, held as the arbor that runs it holds it.
    fn running(state: &Path, id: &str, world: &str, started_ns: u64) -> File {
        let dir = state.join("cells");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.json"));
        let c = Running {
            id: id.into(),
            world: world.into(),
            what: "sleep 60".into(),
            limits: String::new(),
            key: None,
            pid: std::process::id() as i32,
            started_ns,
            cgroup: None,
        };
        fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
        let f = File::open(&path).unwrap();
        arbor::sys::flock(&f, true, false).unwrap();
        f
    }

    fn record(act: &str, world: &str, cell: Option<&str>) {
        Ledger::open()
            .unwrap()
            .append(New {
                act,
                by: "you",
                key: None,
                world,
                cell,
                effect: None,
                what: "",
                limits: None,
                out: "ok",
                cost: Cost::default(),
            })
            .unwrap();
    }

    fn names(s: &Snapshot) -> Vec<(&str, Option<usize>)> {
        s.tries
            .iter()
            .map(|t| (t.name.as_str(), t.changes))
            .collect()
    }

    fn ids(s: &Snapshot) -> Vec<&str> {
        s.rooms.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn a_reading_reads_again_only_what_new_steps_name() {
        let dir = temp("reader");
        let state = dir.join("state");
        // SAFETY: the only test in this crate that reads arbor's state folder, and nothing else here reads
        // ARBOR_HOME. Every reading below goes through it.
        unsafe { std::env::set_var("ARBOR_HOME", &state) };
        let made = |name: &str| {
            let base = dir.join("projects").join(name);
            fs::create_dir_all(&base).unwrap();
            let w = World::create(Some(name), &base).unwrap();
            record("new", name, None);
            w
        };
        let (alpha, beta) = (made("alpha"), made("beta"));
        let change = |w: &World, file: &str| fs::write(w.up().join(file), "x").unwrap();

        let mut reader = Reader::new(8);
        let s = reader.read(Kind::LEDGER);
        assert_eq!(names(&s), [("alpha", Some(0)), ("beta", Some(0))]);
        assert_eq!((s.steps.len(), reader.moved()), (2, true));
        assert!(s.problems.is_empty(), "{:?}", s.problems);

        // Files change in both, but a step names only alpha: beta is not read again.
        change(&alpha, "a1");
        change(&beta, "b1");
        record("run", "alpha", None);
        let s = reader.read(Kind::LEDGER);
        assert_eq!(names(&s), [("alpha", Some(1)), ("beta", Some(0))]);
        // No new step: nothing is read, and the ledger holds still.
        assert!(!reader.behind());
        let s = reader.read(Kind::LEDGER);
        assert_eq!((names(&s)[1], reader.moved()), (("beta", Some(0)), false));

        // A cell starts in beta: it is read by its id, and beta with it.
        let one = running(&state, "c0ffee01", "beta", 1);
        record("start", "beta", Some("c0ffee01"));
        let s = reader.read(Kind::LEDGER);
        assert_eq!(ids(&s), ["c0ffee01"]);
        assert_eq!(names(&s), [("alpha", Some(1)), ("beta", Some(1))]);

        // What changes with no step waits for the room sample: a cell that no step names, and the files of a try
        // with a room working in it. Alpha has none, so its changes are not read again.
        let two = running(&state, "c0ffee02", "beta", 2);
        change(&alpha, "a2");
        change(&beta, "b2");
        let s = reader.read(Kind::LEDGER);
        assert_eq!(
            (ids(&s), names(&s)[1]),
            (vec!["c0ffee01"], ("beta", Some(1)))
        );
        let s = reader.read(Kind::ROOMS);
        assert_eq!(ids(&s), ["c0ffee01", "c0ffee02"]);
        assert_eq!(names(&s), [("alpha", Some(1)), ("beta", Some(2))]);

        // A cell ends: its arbor lets go of its file and removes it, then records how it ended.
        drop(one);
        fs::remove_file(state.join("cells/c0ffee01.json")).unwrap();
        record("run", "beta", Some("c0ffee01"));
        let s = reader.read(Kind::LEDGER);
        assert_eq!(ids(&s), ["c0ffee02"]);

        // A world thrown away goes; one made without a step shows only once everything is read again.
        drop(two);
        fs::remove_dir_all(alpha.dir.clone()).unwrap();
        let _gamma = World::create(Some("gamma"), &dir.join("projects/alpha")).unwrap();
        record("discard", "alpha", None);
        let s = reader.read(Kind::LEDGER);
        assert_eq!(names(&s), [("beta", Some(2))]);
        // The ledger written anew: everything is read again, and a cell whose arbor is gone is not running.
        fs::write(state.join("ledger.jsonl"), "").unwrap();
        record("new", "gamma", None);
        let s = reader.read(Kind::LEDGER);
        assert_eq!(names(&s), [("beta", Some(2)), ("gamma", Some(0))]);
        assert!(ids(&s).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
