//! The words on screen, in the brief's plain terms (docs/INTERFACE.md, *Words on screen*): a try is a world, a room
//! is a cell, a decision is an effect waiting in the outbox, and history is the ledger. Arbor's own terms are one step
//! down, in each line's tooltip. Times are clock times, never "3 min ago", so nothing has to be redrawn to stay true.
//! The facts come first; what an asker said about an effect comes last and is marked as its own words.

use std::collections::HashSet;
use std::path::Path;

use arbor::ledger::Step;
use arbor::outbox::{self, Effect};

pub use crate::act::Do;
use crate::act::{Answer, Ask, Done, Look};
use crate::state::{Room, Snapshot, Try, shown_path};

/// The mark at the start of a line. Each state has a word and a shape of its own, never a colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Waiting for a person: an amber pill with the word in it.
    Wait,
    /// Running: a filled green dot.
    Work,
    /// Open and quiet: a ring.
    Idle,
    /// Kept: a small square.
    Kept,
    /// Released, and it finished: a green diamond.
    Done,
    /// Refused: a short bar.
    No,
    /// It failed, or arbor could not do what was asked: a hollow amber square.
    Failed,
    /// None: a folded line, or a step of history.
    None,
}

/// A button on a line, and the confirmation it asks for before anything happens.
#[derive(Debug, Clone, PartialEq)]
pub struct Act {
    pub what: Do,
    /// On the button: `Release…`.
    pub label: String,
    /// The button's name on the accessibility bus, which says what it acts on: `Release git push origin fix`.
    pub name: String,
    /// The confirmation: its question, the sentence that says what will happen, and the button that does it.
    pub ask: String,
    pub says: String,
    pub go: String,
    /// It offers a reason to give, in a box with this hint.
    pub reason: Option<String>,
}

/// One line of a list, everything the window shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// What the line stands for, so it keeps its place and its widgets from one reading to the next.
    pub key: String,
    pub mark: Mark,
    /// A word in the mark: `decision`.
    pub badge: Option<String>,
    pub title: String,
    /// The title is a command, shown as written.
    pub code: bool,
    /// A command under the title, on one line.
    pub command: Option<String>,
    pub detail: String,
    /// On the right: when, or where.
    pub aside: String,
    /// The asker's own words, which nobody checked, marked as such, or the person's own reason.
    pub note: Option<String>,
    /// What a command printed, as it printed it.
    pub output: Option<String>,
    /// The buttons on the line.
    pub acts: Vec<Act>,
    /// Arbor's own terms, one step down.
    pub tip: String,
    /// The whole line as one sentence, for a screen reader.
    pub said: String,
}

impl Line {
    fn new(key: String, mark: Mark, title: String) -> Line {
        Line {
            key,
            mark,
            badge: None,
            title,
            code: false,
            command: None,
            detail: String::new(),
            aside: String::new(),
            note: None,
            output: None,
            acts: Vec::new(),
            tip: String::new(),
            said: String::new(),
        }
    }
}

/// The question the window exists for.
pub fn heading(waiting: usize) -> String {
    match waiting {
        0 => "Nothing needs you".into(),
        1 => "1 thing needs you".into(),
        n => format!("{n} things need you"),
    }
}

/// The sentence under it: what is working, and where.
pub fn lead(s: &Snapshot) -> String {
    let tries = s.tries.len();
    let rooms = s.rooms.len();
    let working: HashSet<&str> = s.rooms.iter().map(|r| r.world.as_str()).collect();
    let busy = s
        .tries
        .iter()
        .filter(|t| working.contains(t.name.as_str()))
        .count();
    if tries == 0 {
        return "No tries yet. A try is a copy of a folder that an agent works on, kept or thrown away whole."
            .into();
    }
    if rooms == 0 {
        return match tries {
            1 => "Nothing is running in your try.".into(),
            n => format!("Nothing is running in your {n} tries."),
        };
    }
    let working = match rooms {
        1 => "1 room is working".to_string(),
        n => format!("{n} rooms are working"),
    };
    let place = match (busy, tries) {
        (_, 1) => "in your try".to_string(),
        (b, t) if b == t => format!("in all {t} of your tries"),
        (b, t) => format!("in {b} of your {t} tries"),
    };
    format!("{working}, {place}.")
}

/// A decision: an effect waiting for a person.
pub fn decision(e: &Effect, now_ns: u64) -> Line {
    let cmd = e.shown();
    // Without a trailing slash when the asker stood at the top of the folder.
    let folder = shown_path(&e.folder().components().collect::<std::path::PathBuf>());
    let asked = clock(e.asked_ns, now_ns);
    let mut line = Line::new(format!("effect {}", e.id), Mark::Wait, cmd.clone());
    line.badge = Some("decision".into());
    line.code = true;
    let mut facts = vec![
        "Runs as you, with the network".to_string(),
        format!("in {folder}"),
    ];
    if let Some(bytes) = e.input {
        facts.push(format!("reads {} it was given", size(bytes)));
    }
    if e.back {
        facts.push("the room waits for your answer".into());
    }
    line.detail = facts.join(" · ");
    let from = match e.key() {
        Some(k) => format!("from try {} under key {k}", e.world),
        None => format!("from try {}", e.world),
    };
    line.aside = format!("asked at {asked}");
    let mut words = Vec::new();
    if !e.why.is_empty() {
        words.push(format!("“{}”", e.why));
    }
    if let Some(c) = e.cost {
        words.push(format!("says it costs {}", outbox::money(c)));
    }
    if !words.is_empty() {
        line.note = Some(format!(
            "In its own words, not checked: {}",
            words.join(", ")
        ));
    }
    line.tip = format!(
        "Effect {id} in arbor's outbox, asked by cell {cell} {from}. `arbor outbox {id}` shows all of it. \
         Release runs `arbor release {id}`, and Refuse `arbor refuse {id}`.",
        id = e.id,
        cell = e.cell,
    );
    let mut runs = format!(
        "It runs once, now: {cmd}, as you, with the network, in {folder}. Arbor cannot take back what it does \
         outside this computer."
    );
    if e.back {
        runs += &format!(" What it prints goes back to try {}.", e.world);
    }
    if let Some(c) = e.cost {
        runs += &format!(
            " It says it costs {}, which nobody checked.",
            outbox::money(c)
        );
    }
    line.acts = vec![
        Act {
            what: Do::Release,
            label: "Release…".into(),
            name: format!("Release {cmd}"),
            ask: "Release it?".into(),
            says: runs,
            go: "Release now".into(),
            reason: None,
        },
        Act {
            what: Do::Refuse,
            label: "Refuse…".into(),
            name: format!("Refuse {cmd}"),
            ask: "Refuse it?".into(),
            says: match e.back {
                true => format!(
                    "{cmd} will not run. Try {} waits for your answer, and is told no.",
                    e.world
                ),
                false => format!("{cmd} will not run. The refusal is kept in history."),
            },
            go: "Refuse".into(),
            reason: Some(match e.back {
                true => format!(
                    "Your reason, if you want to give one: try {} is told",
                    e.world
                ),
                false => "Your reason, if you want to give one: it is kept with the refusal".into(),
            }),
        },
    ];
    line.said = format!(
        "Decision waiting since {asked}: run {cmd}, as you, with the network, in {folder}, asked {from}.{}",
        line.note
            .as_ref()
            .map(|n| format!(" {n}."))
            .unwrap_or_default()
    );
    line
}

/// A try, and how many rooms work in it.
pub fn try_line(t: &Try, rooms: usize, now_ns: u64) -> Line {
    let (mark, state) = match (t.kept_ns, rooms) {
        (Some(at), _) => (Mark::Kept, format!("kept at {}", clock(at, now_ns))),
        (None, 0) => (Mark::Idle, "open".to_string()),
        (None, 1) => (Mark::Work, "1 room working".to_string()),
        (None, n) => (Mark::Work, format!("{n} rooms working")),
    };
    let changes = match (t.kept_ns, t.changes) {
        (Some(_), _) => None,
        (None, Some(0)) => Some("no changes yet".to_string()),
        (None, Some(1)) => Some("1 file changed".to_string()),
        (None, Some(n)) => Some(format!("{n} files changed")),
        (None, None) => Some("its changes could not be read".to_string()),
    };
    let base = shown_path(&t.base);
    let since = clock(t.since_ns, now_ns);
    let mut line = Line::new(format!("world {}", t.name), mark, t.name.clone());
    line.detail = match &changes {
        Some(c) => format!("{state} · {c}"),
        None => state.clone(),
    };
    line.aside = format!("copy of {base}, made at {since}");
    line.tip = format!(
        "World {} over {base}. `arbor diff {}` lists its changes; Keep runs `arbor keep {}`, and Undo the keep \
         `arbor undo {}`.",
        t.name, t.name, t.name, t.name
    );
    line.acts = try_acts(t, &base);
    line.said = format!(
        "Try {}, a copy of {base} made at {since}: {}.",
        t.name, line.detail
    )
    .replace(" · ", ", ");
    line
}

/// The buttons of a try: See changes and Keep while it is open and has changes, Undo the keep once kept.
fn try_acts(t: &Try, base: &str) -> Vec<Act> {
    let name = &t.name;
    match (t.kept_ns, t.changes) {
        (Some(_), _) => vec![Act {
            what: Do::Undo,
            label: "Undo the keep…".into(),
            name: format!("Undo the keep of try {name}"),
            ask: "Undo the keep?".into(),
            says: format!(
                "What keeping try {name} put into {base} comes back out, and the try is open again. It is \
                 refused if any of those files changed there since."
            ),
            go: "Undo the keep".into(),
            reason: None,
        }],
        (None, Some(n)) if n > 0 => vec![
            Act {
                what: Do::Changes,
                label: "See changes".into(),
                name: format!("See what try {name} changed"),
                ask: String::new(),
                says: String::new(),
                go: String::new(),
                reason: None,
            },
            Act {
                what: Do::Keep,
                label: "Keep…".into(),
                name: format!("Keep try {name}"),
                ask: "Keep it?".into(),
                says: format!(
                    "{} {} into {base}, all at once: {}. You can undo the keep later, as long as those files \
                     are not changed there meanwhile.",
                    capital(&files(n)),
                    if n == 1 { "goes" } else { "go" },
                    counted(t.counts)
                ),
                go: "Keep".into(),
                reason: None,
            },
        ],
        _ => Vec::new(),
    }
}

/// `1 file`, `3 files`.
fn files(n: usize) -> String {
    match n {
        1 => "1 file".into(),
        n => format!("{n} files"),
    }
}

/// `2 added, 1 changed`, leaving out what is none.
fn counted([a, m, d]: [usize; 3]) -> String {
    let parts: Vec<String> = [(a, "added"), (m, "changed"), (d, "removed")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect();
    match parts.as_slice() {
        [] => "no files".into(),
        _ => parts.join(", "),
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Why a file stops a keep or an undo, as arbor says it, in fewer words: `cart.rs (changed)`.
fn plain(problem: &str) -> String {
    let (path, why) = problem.split_once(": ").unwrap_or((problem, ""));
    let short = if why.contains("folder is gone") {
        "its folder is gone"
    } else if why.contains("folder") {
        "its folder changed"
    } else if why.contains("made again") {
        "made again"
    } else if why.contains("deleted") {
        "deleted"
    } else {
        "changed"
    };
    format!("{path} ({short})")
}

/// A list of problems in plain words, cut after a few.
fn plain_list(problems: &[String]) -> String {
    let mut shown: Vec<String> = problems.iter().take(5).map(|p| plain(p)).collect();
    if problems.len() > 5 {
        shown.push(format!("and {} more", problems.len() - 5));
    }
    shown.join(", ")
}

/// A keep or an undo on its way.
pub fn keeping(name: &str, what: Do, now_ns: u64) -> Line {
    let title = match what {
        Do::Undo => format!("Undoing the keep of try {name}"),
        _ => format!("Keeping try {name}"),
    };
    let mut line = Line::new(format!("try {name}"), Mark::Work, title);
    line.aside = format!("at {}", clock(now_ns, now_ns));
    line.said = format!("{}.", line.title);
    line
}

/// A keep or an undo, done, or why arbor did not do it.
pub fn kept(d: &Done, now_ns: u64) -> Line {
    let (name, base) = (&d.name, shown_path(&d.base));
    let n: usize = d.counts.iter().sum();
    let (mark, title, detail) = match (d.what, &d.failed, d.problems.is_empty()) {
        (Do::Undo, None, _) => (
            Mark::Done,
            format!("Undid the keep of try {name}: it is open again"),
            format!(
                "{} taken back out of {base}: {}",
                files(n),
                counted(d.counts)
            ),
        ),
        (_, None, _) => (
            Mark::Done,
            format!("Kept try {name}: its changes are in {base}"),
            format!("{}: {}", capital(&files(n)), counted(d.counts)),
        ),
        (Do::Undo, Some(_), false) => (
            Mark::Failed,
            format!("Not undone: files changed in {base} since the keep"),
            format!(
                "{}. Undoing would lose those changes, so nothing was undone.",
                capital(&plain_list(&d.problems))
            ),
        ),
        (_, Some(_), false) => (
            Mark::Failed,
            format!("Not kept: your own files changed since try {name} began"),
            format!(
                "In {base}: {}. Keeping would overwrite them, so nothing was kept.",
                plain_list(&d.problems)
            ),
        ),
        (Do::Undo, Some(why), true) => (Mark::Failed, format!("Not undone: {why}"), String::new()),
        (_, Some(why), true) => (Mark::Failed, format!("Not kept: {why}"), String::new()),
    };
    let mut line = Line::new(format!("try {name}"), mark, title);
    line.detail = detail;
    line.aside = format!("at {}", clock(now_ns, now_ns));
    line.tip = match d.what {
        Do::Undo => format!("`arbor undo {name}` on world {name}."),
        _ => format!("`arbor keep {name}` on world {name}."),
    };
    line.said = match line.detail.as_str() {
        "" => format!("{}.", line.title),
        detail => format!("{}. {detail}", line.title),
    };
    line
}

/// One file in the view of what a try changed.
#[derive(Debug, Clone, PartialEq)]
pub struct FileLine {
    /// `added`, `changed`, `removed`.
    pub word: String,
    /// Its name within its folder.
    pub name: String,
    /// The lines that changed, when it is text.
    pub patch: Option<String>,
    /// The button's name on the accessibility bus.
    pub show: String,
}

/// The view of what a try changed: a note when a keep would be refused, then the files by folder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Changes {
    pub note: Option<String>,
    pub groups: Vec<(String, Vec<FileLine>)>,
    /// What an announcement says when it opens.
    pub news: String,
}

pub fn changes(name: &str, base: &Path, look: &Look) -> Changes {
    let base = shown_path(base);
    if let Some(why) = &look.failed {
        return Changes {
            note: Some(format!("Its changes could not be read: {why}")),
            news: format!("The changes of try {name} could not be read"),
            ..Changes::default()
        };
    }
    let note = (!look.conflicts.is_empty()).then(|| {
        format!(
            "Keep would be refused: since this try began, these changed in {base}: {}. Keeping would \
             overwrite them.",
            plain_list(&look.conflicts)
        )
    });
    let mut groups: Vec<(String, Vec<FileLine>)> = Vec::new();
    for c in &look.files {
        let path = c.path.trim_end_matches('/');
        let (dir, file) = match path.rsplit_once('/') {
            Some((d, f)) => (format!("in {d}/"), f.to_string()),
            None => (format!("at the top of {base}"), path.to_string()),
        };
        let word = match c.mark {
            'A' => "added",
            'M' => "changed",
            'D' => "removed",
            _ => "special, not kept",
        };
        let patch = look.patches.get(&c.path).map(|p| match p.more {
            0 => p.text.clone(),
            n => format!(
                "{}\n… {n} more lines: `arbor diff {name} --patch` shows them all",
                p.text
            ),
        });
        let line = FileLine {
            word: word.into(),
            name: file + if c.path.ends_with('/') { "/" } else { "" },
            patch,
            show: format!("Show the change to {path}"),
        };
        match groups.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, files)) => files.push(line),
            None => groups.push((dir, vec![line])),
        }
    }
    // The top of the folder first, then each folder by name.
    groups.sort_by_key(|(d, _)| (!d.starts_with("at the top"), d.clone()));
    let n = look.files.len();
    let news = format!(
        "{} changed in try {name}{}",
        capital(&files(n)),
        if note.is_some() {
            "; keep would be refused"
        } else {
            ""
        }
    );
    Changes { note, groups, news }
}

/// A room: a command running in a try.
pub fn room(r: &Room, now_ns: u64) -> Line {
    let mut line = Line::new(format!("cell {}", r.id), Mark::Work, r.what.clone());
    line.code = true;
    let since = clock(r.started_ns, now_ns);
    let mut uses = Vec::new();
    if let Some(m) = r.mem {
        uses.push(format!("{} of memory", size(m)));
    }
    match r.cpu_pct {
        Some(p) if p < 0.5 => {
            line.mark = Mark::Idle;
            uses.push("idle".into());
        }
        Some(p) => uses.push(format!("{} of one processor", percent(p))),
        None => {}
    }
    let mut detail = vec![format!("running since {since}")];
    if let Some(k) = &r.key {
        detail.push(format!("key {k}"));
    }
    detail.extend(uses.iter().cloned());
    line.detail = detail.join(" · ");
    line.tip = match r.limits.as_str() {
        "" => format!("Cell {} in world {}, with no limits.", r.id, r.world),
        l => format!("Cell {} in world {}, limits: {l}.", r.id, r.world),
    };
    line.said = format!(
        "Room running {} in try {}, {}.",
        r.what,
        r.world,
        detail.join(", ")
    );
    line
}

/// The line that folds the rooms past the first few.
pub fn more_rooms(world: &str, n: usize) -> Line {
    let title = match n {
        1 => "and 1 more room".to_string(),
        n => format!("and {n} more rooms"),
    };
    let mut line = Line::new(format!("more {world}"), Mark::None, title);
    line.tip = format!("`arbor ps` lists every cell in world {world}.");
    line.said = format!("{}, running in try {world}.", line.title);
    line
}

/// The line that folds the tries past the first few.
pub fn more_tries(n: usize, working: usize) -> Line {
    let tries = match n {
        1 => "and 1 more try".to_string(),
        n => format!("and {n} more tries"),
    };
    let title = match working {
        0 => tries,
        1 => format!("{tries}, 1 with a room working"),
        w => format!("{tries}, {w} with a room working"),
    };
    let mut line = Line::new("more tries".into(), Mark::None, title);
    line.tip = "`arbor worlds` lists every world, and `arbor ps` every cell.".into();
    line.said = format!("{}.", line.title);
    line
}

/// An answer on its way: the command that answers it runs.
pub fn answering(e: &Effect, ask: &Ask, now_ns: u64) -> Line {
    let cmd = e.shown();
    let at = clock(now_ns, now_ns);
    let (mark, title) = match ask {
        Ask::Release => (Mark::Work, format!("Releasing: running since {at}")),
        Ask::Refuse(_) => (Mark::Work, "Refusing".to_string()),
    };
    let mut line = Line::new(format!("answer {}", e.id), mark, title);
    line.command = Some(cmd.clone());
    line.detail = match ask {
        Ask::Release => "It goes on if you close this window.".into(),
        Ask::Refuse(_) => String::new(),
    };
    line.tip = format!("{cmd}\nEffect {} in arbor's outbox.", e.id);
    line.said = format!("{}: {cmd}. {}", line.title, line.detail)
        .trim_end()
        .to_string();
    line
}

/// An answer, done: the receipt of a released command, a refusal, or why arbor did neither. `e` is the effect as it
/// was when the person answered it.
pub fn answered(e: &Effect, a: &Answer, now_ns: u64) -> Line {
    let cmd = e.shown();
    let after = a.effect.as_ref();
    let at = clock(after.and_then(|x| x.answered_ns).unwrap_or(now_ns), now_ns);
    let verb = match a.ask {
        Ask::Release => "released",
        Ask::Refuse(_) => "refused",
    };
    let mut detail = Vec::new();
    let (mark, title) = match (
        after.map(|x| x.state),
        after.and_then(|x| x.receipt.as_ref()),
    ) {
        _ if a.failed.is_some() => (
            Mark::Failed,
            format!("Not {verb}: {}", a.failed.as_deref().unwrap_or_default()),
        ),
        (Some(outbox::State::Done), Some(r)) => {
            detail.push(took(r.ms));
            detail.push(printed(r.output));
            match r.exit {
                0 => (Mark::Done, "Released: it finished".to_string()),
                -1 => (
                    Mark::Failed,
                    "Released: how it ended is not known".to_string(),
                ),
                c => (
                    Mark::Failed,
                    format!("Released: it failed with exit code {c}"),
                ),
            }
        }
        (Some(outbox::State::Refused), _) => (
            Mark::No,
            match a.ask {
                Ask::Refuse(_) => "Refused: it did not run".to_string(),
                Ask::Release => "Not released: it was refused first".to_string(),
            },
        ),
        (Some(outbox::State::Running), _) => (
            Mark::Failed,
            "Released, but arbor stopped while it ran, so what it did is not known".to_string(),
        ),
        _ => (
            Mark::Failed,
            format!("Not {verb}: its record cannot be read"),
        ),
    };
    let mut line = Line::new(format!("answer {}", e.id), mark, title);
    line.command = Some(cmd.clone());
    line.aside = format!("at {at}");
    line.detail = detail.join(" · ");
    if let Some(x) = after
        && x.state == outbox::State::Refused
        && !x.refusal.is_empty()
    {
        line.note = Some(match a.ask {
            Ask::Refuse(_) => format!("Your reason: “{}”", x.refusal),
            Ask::Release => format!("Why: {}", x.refusal),
        });
    }
    line.output = a.output.clone();
    line.tip = match after.and_then(|x| x.receipt.as_ref()) {
        Some(r) => format!(
            "{cmd}\nEffect {id}'s receipt: exit {}, {} printed, SHA-256 {}…, {} ms, {} ms of processor. \
             `arbor outbox {id}` shows all of it.",
            r.exit,
            size_exact(r.output),
            &r.sha256[..r.sha256.len().min(16)],
            r.ms,
            r.cpu_ms,
            id = e.id
        ),
        None => format!(
            "{cmd}\nEffect {id} in arbor's outbox. `arbor outbox {id}` shows all of it.",
            id = e.id
        ),
    };
    let mut said = format!("{} at {at}: {cmd}", line.title);
    if !line.detail.is_empty() {
        said += &format!(", {}", line.detail.replace(" · ", ", "));
    }
    said.push('.');
    if let Some(n) = &line.note {
        said += &format!(" {n}.");
    }
    line.said = said;
    line
}

/// What an announcement says of an answer once it is done: short, since the line says the rest.
pub fn answer_news(line: &Line) -> String {
    match (&line.command, line.title.split_once(": ")) {
        (Some(c), Some((head, tail))) => format!("{head} {c}: {tail}"),
        (Some(c), None) => format!("{}: {c}", line.title),
        (None, _) => line.title.clone(),
    }
}

/// How much a command printed.
fn printed(bytes: u64) -> String {
    match bytes {
        0 => "printed nothing".into(),
        _ => format!("printed {}", size_exact(bytes)),
    }
}

/// Bytes, counted one by one below a kB.
fn size_exact(bytes: u64) -> String {
    match bytes {
        1 => "1 byte".into(),
        0..1000 => format!("{bytes} bytes"),
        _ => size(bytes),
    }
}

/// A step of history.
pub fn step(s: &Step, now_ns: u64) -> Line {
    let at = clock(s.ts, now_ns);
    let w = &s.world;
    let key = s.key.as_deref().unwrap_or("-");
    let what = shown(&s.what);
    // The sentence, and the command it is about, if any. A command arbor refused to start records why instead.
    let refused = s.out == "refused" && matches!(s.act.as_str(), "run" | "shell");
    let (title, command) = match s.act.as_str() {
        _ if refused => (
            format!("Could not start a command in try {w}: {what}"),
            None,
        ),
        "new" => (format!("Made try {w}, a copy of {what}"), None),
        "start" => (format!("Started a command in try {w}"), Some(what)),
        "run" => (format!("Ran a command in try {w}"), Some(what)),
        "shell" => (format!("Opened a shell in try {w}"), None),
        "stop" => (format!("Stopped a command in try {w}"), Some(what)),
        "keep" => (format!("Kept try {w}: {what}"), None),
        "undo" => (format!("Took back what try {w} kept: {what}"), None),
        "discard" => (format!("Threw away try {w}"), None),
        "key" => (format!("Made key {key}"), None),
        "renew" => (format!("Renewed {what}"), None),
        "take-back" => (format!("Took back key {key}"), None),
        "effect" => (format!("Try {w} asked to run a command"), Some(what)),
        "release" => ("Released a command".to_string(), Some(what)),
        "refuse" => ("Refused a command".to_string(), Some(what)),
        "net" => (format!("Network from try {w}: {what}"), None),
        act => (format!("{act} {what}"), None),
    };
    // A start is a past fact: whether its command still runs is in the step that ends it, or in the tries.
    let mut detail = if s.act == "start" {
        String::new()
    } else {
        outcome(&s.out)
    };
    if matches!(s.act.as_str(), "run" | "shell" | "release") && !refused && s.out != "running" {
        detail = format!("{detail} {}", took(s.cost.ms));
    }
    let mut line = Line::new(format!("step {}", s.seq), Mark::None, title);
    line.aside = at.clone();
    line.detail = detail;
    line.tip = format!(
        "Step {} of the ledger: {} by {}, key {key}.",
        s.seq, s.act, s.by
    );
    let rest = match line.detail.as_str() {
        "" => String::new(),
        d => format!(", {d}"),
    };
    line.said = match &command {
        Some(c) => format!("{at}: {}: {c}{rest}.", line.title),
        None => format!("{at}: {}{rest}.", line.title),
    };
    if let Some(c) = &command {
        line.tip = format!("{c}\n{}", line.tip);
    }
    line.command = command;
    line
}

/// A step's result in plain words.
pub fn outcome(out: &str) -> String {
    match out {
        "ok" => "done".into(),
        "exit 0" => "finished".into(),
        "running" => "running".into(),
        "waiting" => "waiting for you".into(),
        "refused" => "refused".into(),
        "stopped" => "stopped".into(),
        "killed: time limit" => "stopped at its time limit".into(),
        "killed: memory limit" => "stopped at its memory limit".into(),
        "killed: key expired" => "stopped when its key ended".into(),
        "stopped: key taken back" => "stopped when its key was taken back".into(),
        o => match o.strip_prefix("exit ") {
            Some(code) => format!("failed with exit code {code}"),
            None => o.into(),
        },
    }
}

/// How long a step took.
pub fn took(ms: u64) -> String {
    match ms {
        0..1000 => "in under a second".into(),
        1000..60_000 => format!("in {} s", ms / 1000),
        60_000..3_600_000 => format!("in {} min", ms / 60_000),
        _ => format!("in {} h {} min", ms / 3_600_000, ms % 3_600_000 / 60_000),
    }
}

/// Bytes, in the units people read: MB and GB of 1000.
pub fn size(bytes: u64) -> String {
    match bytes {
        0..1_000_000 => format!("{} kB", bytes.div_ceil(1000)),
        1_000_000..1_000_000_000 => format!("{} MB", (bytes + 500_000) / 1_000_000),
        _ => format!("{:.1} GB", bytes as f64 / 1e9),
    }
}

fn percent(p: f64) -> String {
    if p < 10.0 {
        format!("{}%", (p.round() as u64).max(1))
    } else {
        format!("{:.0}%", (p / 5.0).round() * 5.0)
    }
}

/// A folder in a step's details, shown with `~`.
fn shown(what: &str) -> String {
    if what.starts_with('/') {
        shown_path(Path::new(what))
    } else {
        what.to_string()
    }
}

/// A clock time: `14:05` today, `17 Oct 14:05` on another day.
pub fn clock(ts_ns: u64, now_ns: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match (local(ts_ns), local(now_ns)) {
        (Some(t), Some(n)) if (t.tm_year, t.tm_yday) == (n.tm_year, n.tm_yday) => {
            format!("{:02}:{:02}", t.tm_hour, t.tm_min)
        }
        (Some(t), _) => format!(
            "{} {} {:02}:{:02}",
            t.tm_mday,
            MONTHS[t.tm_mon.clamp(0, 11) as usize],
            t.tm_hour,
            t.tm_min
        ),
        _ => "?".into(),
    }
}

fn local(ts_ns: u64) -> Option<libc::tm> {
    let secs = (ts_ns / 1_000_000_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (!unsafe { libc::localtime_r(&secs, &mut tm) }.is_null()).then_some(tm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn try_named(name: &str, kept: bool, changes: Option<usize>) -> Try {
        Try {
            name: name.into(),
            base: PathBuf::from(format!("/nowhere/{name}")),
            since_ns: 0,
            kept_ns: kept.then_some(0),
            changes,
            counts: [changes.unwrap_or(0), 0, 0],
        }
    }

    fn room_in(world: &str, cpu: Option<f64>) -> Room {
        Room {
            id: "c1".into(),
            world: world.into(),
            what: "npm test".into(),
            key: Some("dev".into()),
            limits: String::new(),
            started_ns: 0,
            mem: Some(180_400_000),
            cpu_us: None,
            cpu_pct: cpu,
        }
    }

    #[test]
    fn the_heading_counts_what_needs_you() {
        assert_eq!(heading(0), "Nothing needs you");
        assert_eq!(heading(1), "1 thing needs you");
        assert_eq!(heading(3), "3 things need you");
    }

    #[test]
    fn the_lead_says_what_works_where() {
        let mut s = Snapshot::default();
        assert!(lead(&s).starts_with("No tries yet."));
        s.tries = vec![try_named("shop", false, Some(0))];
        assert_eq!(lead(&s), "Nothing is running in your try.");
        s.tries.push(try_named("site", false, Some(2)));
        s.tries.push(try_named("notes", true, None));
        assert_eq!(lead(&s), "Nothing is running in your 3 tries.");
        s.rooms = vec![room_in("shop", None), room_in("shop", None)];
        assert_eq!(lead(&s), "2 rooms are working, in 1 of your 3 tries.");
    }

    #[test]
    fn a_try_says_its_state_in_words() {
        let open = try_line(&try_named("shop", false, Some(4)), 2, 0);
        assert_eq!(
            (open.mark, open.detail.as_str()),
            (Mark::Work, "2 rooms working · 4 files changed")
        );
        assert!(
            open.said
                .starts_with("Try shop, a copy of /nowhere/shop made at ")
        );
        assert!(open.said.ends_with(": 2 rooms working, 4 files changed."));
        let quiet = try_line(&try_named("site", false, Some(1)), 0, 0);
        assert_eq!(
            (quiet.mark, quiet.detail.as_str()),
            (Mark::Idle, "open · 1 file changed")
        );
        let kept = try_line(&try_named("notes", true, None), 0, 0);
        assert_eq!(kept.mark, Mark::Kept);
        assert!(kept.detail.starts_with("kept at "));
    }

    #[test]
    fn a_try_keeps_after_one_confirmation() {
        let acts = |t: &Try| try_line(t, 0, 0).acts;
        let open = acts(&try_named("site", false, Some(1)));
        let whats: Vec<Do> = open.iter().map(|a| a.what).collect();
        assert_eq!(whats, [Do::Changes, Do::Keep]);
        assert_eq!(open[0].ask, "", "seeing the changes needs no confirmation");
        assert_eq!(
            (open[1].ask.as_str(), open[1].go.as_str()),
            ("Keep it?", "Keep")
        );
        assert!(
            open[1]
                .says
                .starts_with("1 file goes into /nowhere/site, all at once: 1 added.")
        );
        let many = acts(&try_named("shop", false, Some(3)));
        assert!(many[1].says.starts_with("3 files go into /nowhere/shop"));
        assert!(acts(&try_named("help", false, Some(0))).is_empty());
        let kept = acts(&try_named("notes", true, None));
        assert_eq!(
            (kept.len(), kept[0].what, kept[0].go.as_str()),
            (1, Do::Undo, "Undo the keep")
        );
    }

    #[test]
    fn a_room_says_what_it_uses() {
        let r = room(&room_in("shop", Some(31.0)), 0);
        assert!(
            r.detail
                .ends_with("key dev · 180 MB of memory · 30% of one processor"),
            "{}",
            r.detail
        );
        let idle = room(&room_in("shop", Some(0.1)), 0);
        assert_eq!(idle.mark, Mark::Idle);
        assert!(idle.detail.ends_with("· idle"));
    }

    fn waiting(back: bool) -> Effect {
        serde_json::from_value(serde_json::json!({
            "id": 4, "world": "shop", "cell": "0a1b", "base": "/nowhere/shop", "cmd": ["git", "push", "origin", "fix"],
            "cost": 40, "why": "publish the fix", "back": back, "asked_ns": 0, "state": "waiting",
        }))
        .unwrap()
    }

    #[test]
    fn a_decision_asks_before_it_answers() {
        let d = decision(&waiting(false), 0);
        let [release, refuse] = &d.acts[..] else {
            panic!("{:?}", d.acts)
        };
        assert_eq!(
            (release.what, release.label.as_str(), release.name.as_str()),
            (Do::Release, "Release…", "Release git push origin fix")
        );
        assert_eq!(
            release.says,
            "It runs once, now: git push origin fix, as you, with the network, in /nowhere/shop. Arbor cannot \
             take back what it does outside this computer. It says it costs $0.40, which nobody checked."
        );
        assert_eq!(release.reason, None);
        assert_eq!(refuse.what, Do::Refuse);
        assert!(refuse.says.ends_with("The refusal is kept in history."));
        // An asker that waits for the answer gets what the command prints, or the refusal.
        let d = decision(&waiting(true), 0);
        assert!(d.acts[0].says.ends_with(
            "What it prints goes back to try shop. It says it costs $0.40, which nobody checked."
        ));
        assert!(
            d.acts[1]
                .says
                .ends_with("Try shop waits for your answer, and is told no.")
        );
        let busy = answering(&waiting(false), &Ask::Release, 0);
        assert_eq!(
            (busy.key.as_str(), busy.mark, busy.command.as_deref()),
            ("answer 4", Mark::Work, Some("git push origin fix"))
        );
        assert!(busy.title.starts_with("Releasing: running since "));
    }

    fn step_of(act: &str, out: &str) -> Step {
        let json = serde_json::json!({
            "seq": 7, "ts": 0, "by": "you", "act": act, "world": "shop", "what": "npm test", "out": out,
            "cost": { "ms": 1200 },
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_step_says_what_happened_then() {
        let start = step(&step_of("start", "running"), 0);
        assert_eq!(
            (start.title.as_str(), start.detail.as_str()),
            ("Started a command in try shop", "")
        );
        assert!(
            start
                .said
                .ends_with(": Started a command in try shop: npm test."),
            "{}",
            start.said
        );
        let run = step(&step_of("run", "exit 0"), 0);
        assert_eq!(
            (run.command.as_deref(), run.detail.as_str()),
            (Some("npm test"), "finished in 1 s")
        );
        let refused = step(&step_of("run", "refused"), 0);
        assert_eq!(
            refused.title,
            "Could not start a command in try shop: npm test"
        );
        assert_eq!(refused.command, None);
    }

    #[test]
    fn units_are_the_ones_people_read() {
        assert_eq!(size(999), "1 kB");
        assert_eq!(size(180_400_000), "180 MB");
        assert_eq!(size(5_200_000_000), "5.2 GB");
        assert_eq!(took(17), "in under a second");
        assert_eq!(took(2_300), "in 2 s");
        assert_eq!(took(185_000), "in 3 min");
        assert_eq!(percent(0.6), "1%");
        assert_eq!(percent(7.4), "7%");
        assert_eq!(percent(31.0), "30%");
        assert_eq!(outcome("exit 2"), "failed with exit code 2");
        assert_eq!(outcome("killed: key expired"), "stopped when its key ended");
    }
}
