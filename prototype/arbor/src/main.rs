//! `arbor`: the command people and agents use.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use arbor::cell::{self, Options};
use arbor::cgroup::{self, Limits};
use arbor::error::{self, Error, Result};
use arbor::key::{self, Key};
use arbor::ledger::{self, Cost, Ledger};
use arbor::world::{self, State, World};
use arbor::{paths, table, tree};
use lexopt::prelude::*;

const HELP: &str = "\
arbor: try changes on a copy of a folder, then keep them or throw them away.

  arbor new [NAME] [--from DIR]      make a world: an instant copy of DIR (default: here)
  arbor worlds                       list the worlds
  arbor shell NAME [OPTIONS]         open a shell inside a world
  arbor run NAME [OPTIONS] -- CMD... run one command inside a world
  arbor diff NAME [--patch]          list what the world changed (--patch: show the lines)
  arbor keep NAME [--force]          apply the world's changes to the real folder
  arbor undo NAME [--force]          take kept changes back out of the real folder
  arbor discard NAME                 throw a world away
  arbor ps                           list the commands running in worlds and what they use now
  arbor stop CELL                    stop a command running in a world
  arbor key new NAME [GRANTS]        make a key that opens more than the world, until a set time
  arbor keys                         list the keys
  arbor key take-back NAME           end a key and every key made within it, and stop their commands
  arbor ledger [--last N] [--verify] the record of every step (--verify: check that none was changed)
  arbor watch [--since N]            print each new step as a JSON line (--since N: the steps after N first)

OPTIONS: --key KEY opens what the key opens. LIMITS stop a command that takes too much:
--mem SIZE (512M, 2G), --cpu CPUS (0.5, 2), --pids N (processes at once), --time DURATION (90s, 10m, 2h).

GRANTS: --read PATH (a folder or file), --toolchains (~/.cargo, ~/.npm, ~/.pyenv and the like),
--env VAR (passes a variable in), --world W (only that world), LIMITS, --for DURATION or --until HH:MM
(8h if neither), --within KEY (a narrower key, made from another).

The real folder does not change until `arbor keep`. Inside a world the rest of the computer is read-only,
your home folder is empty but for what a key opens, password files stay hidden, and there is no network.
Read the diff before keeping: kept files such as .git/hooks run later on your computer.
";

/// How long `arbor stop` waits for the arbor running a command to end it before ending it directly.
const STOP_GRACE: Duration = Duration::from_secs(3);

fn main() -> ExitCode {
    // Like other Unix tools, stop quietly when the reader goes away (`arbor diff demo --patch | head`).
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    match run() {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("arbor: {e}");
            ExitCode::from(e.code.clamp(1, 255) as u8)
        }
    }
}

fn run() -> Result<i32> {
    let mut p = lexopt::Parser::from_env();
    let command = match p.next()? {
        None | Some(Short('h') | Long("help")) => return help(),
        Some(Short('V') | Long("version")) => {
            println!("arbor {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        Some(Value(v)) => v.string()?,
        Some(arg) => return Err(arg.unexpected().into()),
    };
    match command.as_str() {
        "new" => new(&mut p),
        "worlds" | "ls" => worlds(&mut p),
        "shell" => shell(&mut p),
        "run" => run_cmd(&mut p),
        "diff" => diff(&mut p),
        "keep" => keep(&mut p),
        "undo" => undo(&mut p),
        "discard" => discard(&mut p),
        "ps" => ps(&mut p),
        "stop" => stop(&mut p),
        "key" => key_cmd(&mut p),
        "keys" => keys(&mut p),
        "ledger" => ledger_cmd(&mut p),
        "watch" => watch(&mut p),
        "serve" => serve(&mut p),
        "help" => help(),
        other => Err(Error::usage(format!(
            "there is no command '{other}'; run `arbor help`"
        ))),
    }
}

fn help() -> Result<i32> {
    print!("{HELP}");
    Ok(0)
}

/// A hint for a person at a terminal; agents reading a pipe don't pay for it.
fn hint(text: &str) {
    if io::stderr().is_terminal() {
        eprintln!("{text}");
    }
}

/// Reads the world name and the flags named in `flags` (all boolean). Unknown arguments are mistakes.
fn name_and_flags(p: &mut lexopt::Parser, flags: &[&str]) -> Result<(String, Vec<String>)> {
    let (mut name, mut set) = (None, Vec::new());
    while let Some(arg) = p.next()? {
        match arg {
            Long(f) if flags.contains(&f) => set.push(f.to_string()),
            Value(v) if name.is_none() => name = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let name = name.ok_or_else(|| Error::usage("which world? (see `arbor worlds`)"))?;
    Ok((name, set))
}

/// What a step tells the ledger about itself while it runs.
#[derive(Default)]
struct Note {
    world: String,
    key: Option<String>,
    cell: Option<String>,
    what: String,
    limits: String,
    out: Option<String>,
    cpu_ms: Option<u64>,
    mem_kib: Option<u64>,
    pids: Option<u64>,
}

/// Records a step. It has happened by then, so a ledger that cannot be written is a warning, not a failure.
fn append(step: ledger::New) {
    if let Err(e) = Ledger::open().and_then(|l| l.append(step)) {
        eprintln!("arbor: warning: this step is not in the ledger: {e}");
    }
}

/// Takes a step and records it in the ledger however it ends, with its outcome, its detail (or the first line of
/// its error) and its cost. A mistake in how arbor was called changes nothing and is not recorded.
fn recorded(act: &str, world: &str, step: impl FnOnce(&mut Note) -> Result<i32>) -> Result<i32> {
    let started = Instant::now();
    let mut note = Note {
        world: world.to_string(),
        ..Note::default()
    };
    let res = step(&mut note);
    let out = match &res {
        Ok(_) => note.out.take().unwrap_or_else(|| "ok".into()),
        Err(e) if e.code == error::USAGE => return res,
        Err(e) if e.code == error::REFUSED => "refused".into(),
        Err(e) if e.code == error::STOPPED => "stopped".into(),
        Err(_) => "failed".into(),
    };
    if let Err(e) = &res
        && note.what.is_empty()
    {
        note.what = e.msg.lines().next().unwrap_or_default().to_string();
    }
    append(ledger::New {
        act,
        key: note.key.as_deref(),
        world: &note.world,
        cell: note.cell.as_deref(),
        what: &note.what,
        limits: Some(&note.limits),
        out: &out,
        cost: Cost {
            ms: started.elapsed().as_millis() as u64,
            cpu_ms: note.cpu_ms,
            mem_kib: note.mem_kib,
            pids: note.pids,
        },
    });
    res
}

fn new(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut name, mut from) = (None, None);
    while let Some(arg) = p.next()? {
        match arg {
            Long("from") => from = Some(PathBuf::from(p.value()?)),
            Value(v) if name.is_none() => name = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let from = match from {
        Some(f) => f,
        None => env::current_dir()?,
    };
    cell::check_bwrap()?;
    recorded("new", name.as_deref().unwrap_or("-"), |note| {
        let world = World::create(name.as_deref(), &from)?;
        note.world = world.name.clone();
        note.what = paths::tilde(&world.base);
        println!("made world {} over {}", world.name, note.what);
        hint(&format!(
            "Work in it with `arbor shell {}`; nothing in the real folder changes until `arbor keep {}`.",
            world.name, world.name
        ));
        Ok(0)
    })
}

fn worlds(p: &mut lexopt::Parser) -> Result<i32> {
    if let Some(arg) = p.next()? {
        return Err(arg.unexpected().into());
    }
    let all = World::list()?;
    if all.is_empty() {
        hint("No worlds yet. Make one with `arbor new NAME --from DIR`.");
        return Ok(0);
    }
    let mut rows = Vec::new();
    for w in &all {
        let changes = match w.state {
            State::Open => tree::changes(w, &tree::entries(w)?)?.len().to_string(),
            State::Kept => "-".to_string(),
        };
        let state = match w.state {
            State::Open => "open",
            State::Kept => "kept",
        };
        rows.push(vec![
            w.name.clone(),
            state.to_string(),
            changes,
            paths::tilde(&w.base),
        ]);
    }
    table::print(&["name", "state", "changes", "base"], &rows)?;
    Ok(0)
}

/// How `arbor run` and `arbor shell` were asked to run a command.
struct Asked {
    world: String,
    key: Option<String>,
    limits: Limits,
    interactive: bool,
}

/// Reads the world's name, `--key` and the limit flags. `more` takes each value after the name.
fn read_asked(
    p: &mut lexopt::Parser,
    mut more: impl FnMut(&mut lexopt::Parser, OsString) -> Result<()>,
) -> Result<(Option<String>, Option<String>, Limits)> {
    let (mut name, mut key, mut limits) = (None, None, Limits::default());
    while let Some(arg) = p.next()? {
        match arg {
            Long("key") => key = Some(p.value()?.string()?),
            Long(flag) if cgroup::FLAGS.contains(&flag) => {
                let flag = flag.to_string();
                limits.set(&flag, &p.value()?.string()?)?;
            }
            Value(v) if name.is_none() => name = Some(v.string()?),
            Value(v) => more(p, v)?,
            arg => return Err(arg.unexpected().into()),
        }
    }
    Ok((name, key, limits))
}

impl Asked {
    /// The limits the command will run under: its key's and its own, the tighter of each. Read before anything
    /// is recorded, to know whether it needs a cgroup; a key that cannot be read is reported when it is used.
    fn need_cgroup(&self) -> bool {
        let key = self.key.as_deref().and_then(|k| Key::load(k).ok());
        key.map_or(self.limits, |k| k.limits.tighter(self.limits))
            .need_cgroup()
    }
}

/// Runs `cmd` in a world and notes how it ended and what it cost. The cell's start is a step of its own, so
/// watchers see it while it runs.
fn run_noted(note: &mut Note, asked: Asked, cmd: &[OsString], shell: bool) -> Result<i32> {
    let key = asked.key.as_deref().map(Key::load).transpose()?;
    note.key = asked.key.clone();
    let (world, lock) = World::load_locked(&asked.world)?;
    note.what = cell::shown(cmd);
    if let Some(k) = &key {
        k.check(&world.name)?;
    }
    let opts = Options {
        interactive: asked.interactive,
        limits: key
            .as_ref()
            .map_or(asked.limits, |k| k.limits.tighter(asked.limits)),
        key,
    };
    note.limits = opts.limits.describe();
    if shell {
        eprintln!(
            "Inside world {} over {}. Changes stay in the world; `exit` to leave.",
            world.name,
            paths::tilde(&world.base)
        );
    }
    let begun = Instant::now();
    let ran = cell::run(&world, &lock, cmd, &opts, &mut |id| {
        append(ledger::New {
            act: "start",
            key: note.key.as_deref(),
            world: &world.name,
            cell: Some(id),
            what: &note.what,
            limits: Some(&note.limits),
            out: "running",
            cost: Cost {
                ms: begun.elapsed().as_millis() as u64,
                ..Cost::default()
            },
        })
    })?;
    if let Some(warning) = &ran.warning {
        eprintln!("arbor: {warning}");
    }
    note.cell = Some(ran.id);
    note.out = Some(ran.out);
    (note.cpu_ms, note.mem_kib, note.pids) = (Some(ran.cpu_ms), Some(ran.mem_kib), ran.pids);
    if shell {
        hint(&format!(
            "Back outside. See the changes with `arbor diff {}`.",
            world.name
        ));
    }
    Ok(ran.code)
}

fn shell(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, key, limits) = read_asked(p, |_, v| Err(Value(v).unexpected().into()))?;
    let name = name.ok_or_else(|| Error::usage("which world? (see `arbor worlds`)"))?;
    let sh = env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    let asked = Asked {
        world: name.clone(),
        key,
        limits,
        interactive: true,
    };
    if asked.need_cgroup() {
        cgroup::enter_user_manager();
    }
    recorded("shell", &name, |note| run_noted(note, asked, &[sh], true))
}

fn run_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let mut cmd = Vec::new();
    let (name, key, limits) = read_asked(p, |p, v| {
        cmd.push(v);
        cmd.extend(p.raw_args()?);
        Ok(())
    })?;
    let name = name.ok_or_else(|| Error::usage("which world? usage: arbor run NAME -- CMD..."))?;
    if cmd.is_empty() {
        return Err(Error::usage(
            "which command? usage: arbor run NAME -- CMD...",
        ));
    }
    let asked = Asked {
        world: name.clone(),
        key,
        limits,
        interactive: io::stdin().is_terminal(),
    };
    if asked.need_cgroup() {
        cgroup::enter_user_manager();
    }
    recorded("run", &name, |note| run_noted(note, asked, &cmd, false))
}

fn diff(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, flags) = name_and_flags(p, &["patch"])?;
    let world = World::load(&name)?;
    if world.state == State::Kept {
        return Err(Error::refused(format!(
            "world {name} was kept; `arbor undo {name}` reopens it"
        )));
    }
    if flags.iter().any(|f| f == "patch") {
        tree::patch(&world, &mut io::stdout().lock())?;
        return Ok(0);
    }
    let changes = tree::changes(&world, &tree::entries(&world)?)?;
    let mut out = io::stdout().lock();
    if changes.is_empty() {
        writeln!(out, "no changes")?;
    }
    for c in &changes {
        writeln!(out, "{} {}", c.mark, c.path)?;
    }
    if !changes.is_empty() && io::stdout().is_terminal() {
        writeln!(out, "{}", summary(&changes))?;
    }
    Ok(0)
}

fn summary(changes: &[tree::Change]) -> String {
    let count = |m| changes.iter().filter(|c| c.mark == m).count();
    format!(
        "{} change{} ({} added, {} modified, {} deleted)",
        changes.len(),
        if changes.len() == 1 { "" } else { "s" },
        count('A'),
        count('M'),
        count('D')
    )
}

fn keep(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, flags) = name_and_flags(p, &["force"])?;
    recorded("keep", &name, |note| {
        let (mut world, _lock) = World::load_locked(&name)?;
        let _near = world.lock_neighbours()?;
        let kept = tree::keep(&mut world, flags.iter().any(|f| f == "force"))?;
        let applied: Vec<_> = kept
            .changes
            .iter()
            .filter(|c| c.mark != '?')
            .cloned()
            .collect();
        note.what = summary(&applied);
        println!(
            "kept world {} in {}: {}",
            world.name,
            paths::tilde(&world.base),
            note.what
        );
        if kept.skipped > 0 {
            println!(
                "skipped {} special file(s) (pipes, sockets or devices)",
                kept.skipped
            );
        }
        hint(&format!("`arbor undo {}` takes them back out.", world.name));
        Ok(0)
    })
}

fn undo(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, flags) = name_and_flags(p, &["force"])?;
    recorded("undo", &name, |note| {
        let (mut world, _lock) = World::load_locked(&name)?;
        let _near = world.lock_neighbours()?;
        tree::undo(&mut world, flags.iter().any(|f| f == "force"))?;
        note.what = summary(&tree::changes(&world, &tree::entries(&world)?)?);
        println!(
            "undid world {}: {} taken back out of {}; the world is open again",
            world.name,
            note.what,
            paths::tilde(&world.base)
        );
        Ok(0)
    })
}

fn discard(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, _) = name_and_flags(p, &[])?;
    recorded("discard", &name, |note| {
        let (world, lock) = World::load_locked(&name)?;
        let (state, base) = (world.state, paths::tilde(&world.base));
        world.discard(&lock)?;
        note.what = base.clone();
        match state {
            State::Open => println!("threw away world {name}; {base} is as it was"),
            State::Kept => {
                println!("removed world {name} and its undo copy; the kept changes stay in {base}")
            }
        }
        Ok(0)
    })
}

/// Wall time, then CPU time, peak memory and peak processes when a command ran.
fn cost(c: &Cost) -> String {
    let mut s = format!("{}ms", c.ms);
    if let Some(cpu) = c.cpu_ms {
        s += &format!(" cpu {cpu}ms");
    }
    if let Some(kib) = c.mem_kib {
        s += &format!(" {}", cgroup::size(kib * 1024));
    }
    if let Some(pids) = c.pids {
        s += &format!(" pids {pids}");
    }
    s
}

/// `40ms`, `2.5s`.
fn cpu_time(us: u64) -> String {
    if us < 1_000_000 {
        format!("{}ms", us / 1000)
    } else {
        format!("{:.1}s", us as f64 / 1e6)
    }
}

fn ps(p: &mut lexopt::Parser) -> Result<i32> {
    if let Some(arg) = p.next()? {
        return Err(arg.unexpected().into());
    }
    let cells = cell::running()?;
    if cells.is_empty() {
        hint("Nothing is running in a world.");
        return Ok(0);
    }
    let now = world::now_ns();
    let rows: Vec<Vec<String>> = cells
        .iter()
        .map(|c| {
            let (cpu, mem) = match &c.cgroup {
                Some(dir) => (
                    cgroup::read_usage(dir).cpu_us.map_or("-".into(), cpu_time),
                    cgroup::memory_now(dir).map_or("-".into(), cgroup::size),
                ),
                None => ("-".into(), "-".into()),
            };
            vec![
                c.id.clone(),
                c.world.clone(),
                c.key.clone().unwrap_or_else(|| "-".into()),
                cgroup::duration(Duration::from_nanos(now.saturating_sub(c.started_ns))),
                cpu,
                mem,
                if c.limits.is_empty() {
                    "-".into()
                } else {
                    c.limits.clone()
                },
                c.what.clone(),
            ]
        })
        .collect();
    table::print(
        &[
            "cell", "world", "key", "running", "cpu", "mem", "limits", "what",
        ],
        &rows,
    )?;
    Ok(0)
}

fn stop(p: &mut lexopt::Parser) -> Result<i32> {
    let mut id = None;
    while let Some(arg) = p.next()? {
        match arg {
            Value(v) if id.is_none() => id = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let id = id.ok_or_else(|| Error::usage("which cell? (see `arbor ps`)"))?;
    let Some(c) = cell::running()?.into_iter().find(|c| c.id == id) else {
        return Err(Error::usage(format!(
            "no cell '{id}' is running (see `arbor ps`)"
        )));
    };
    recorded("stop", &c.world, |note| {
        (note.key, note.cell, note.what) = (c.key.clone(), Some(c.id.clone()), c.what.clone());
        if cell::stop(&c, STOP_GRACE)? {
            println!("stopped cell {} in world {}: {}", c.id, c.world, c.what);
        } else {
            note.out = Some("already ended".into());
            println!("cell {} had already ended", c.id);
        }
        Ok(0)
    })
}

fn key_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let usage =
        "usage: arbor key new NAME [GRANTS], or arbor key take-back NAME (see `arbor help`)";
    match p.next()? {
        Some(Value(v)) => match v.string()?.as_str() {
            "new" => key_new(p),
            "take-back" => take_back(p),
            other => Err(Error::usage(format!(
                "there is no `arbor key {other}`; {usage}"
            ))),
        },
        None => Err(Error::usage(usage)),
        Some(arg) => Err(arg.unexpected().into()),
    }
}

fn key_new(p: &mut lexopt::Parser) -> Result<i32> {
    let now = world::now_ns();
    let (mut name, mut spec) = (None, key::Spec::default());
    let end = |spec: &mut key::Spec, at: u64| {
        if spec.until_ns.replace(at).is_some() {
            return Err(Error::usage("give --for or --until, not both"));
        }
        Ok(())
    };
    while let Some(arg) = p.next()? {
        match arg {
            Long("world") => spec.worlds.push(p.value()?.string()?),
            Long("read") => spec.read.push(PathBuf::from(p.value()?)),
            Long("toolchains") => spec.toolchains = true,
            Long("env") => spec.env.push(p.value()?.string()?),
            Long("within") => spec.within = Some(p.value()?.string()?),
            Long("for") => {
                let life = cgroup::parse_duration(&p.value()?.string()?)?;
                end(&mut spec, now.saturating_add(life.as_nanos() as u64))?;
            }
            Long("until") => {
                let at = key::parse_until(&p.value()?.string()?, now)?;
                end(&mut spec, at)?;
            }
            Long(flag) if cgroup::FLAGS.contains(&flag) => {
                let flag = flag.to_string();
                spec.limits.set(&flag, &p.value()?.string()?)?;
            }
            Value(v) if name.is_none() => name = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    spec.name = name.ok_or_else(|| {
        Error::usage("what is the key called? usage: arbor key new NAME --read PATH")
    })?;
    recorded("key", "-", |note| {
        note.key = Some(spec.name.clone());
        let k = Key::create(spec)?;
        note.what = k.describe();
        println!("made key {}: {}", k.name, note.what);
        hint(&format!(
            "Use it with `arbor run WORLD --key {} -- CMD`. `arbor key take-back {}` ends it early.",
            k.name, k.name
        ));
        Ok(0)
    })
}

fn take_back(p: &mut lexopt::Parser) -> Result<i32> {
    let mut name = None;
    while let Some(arg) = p.next()? {
        match arg {
            Value(v) if name.is_none() => name = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let name = name.ok_or_else(|| Error::usage("which key? (see `arbor keys`)"))?;
    recorded("take-back", "-", |note| {
        note.key = Some(name.clone());
        let gone = key::take_back(&name)?;
        // A cell that started under one of them before it went is stopped; one starting now sees it gone.
        let mut stopped = 0;
        for c in cell::running()? {
            if c.key.as_ref().is_some_and(|k| gone.contains(k)) && cell::stop(&c, STOP_GRACE)? {
                stopped += 1;
            }
        }
        let mut what = format!("key {}", gone[0]);
        match &gone[1..] {
            [] => {}
            [one] => what += &format!(", and key {one} made within it"),
            more => what += &format!(", and keys {} made within it", more.join(", ")),
        }
        what += &format!(
            "; stopped {stopped} command{}",
            if stopped == 1 { "" } else { "s" }
        );
        println!("took back {what}");
        note.what = what;
        Ok(0)
    })
}

fn keys(p: &mut lexopt::Parser) -> Result<i32> {
    if let Some(arg) = p.next()? {
        return Err(arg.unexpected().into());
    }
    let all = Key::list()?;
    if all.is_empty() {
        hint("No keys yet. Make one with `arbor key new NAME --read PATH`.");
        return Ok(0);
    }
    let now = world::now_ns();
    let or_dash = |s: String| if s.is_empty() { "-".into() } else { s };
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|k| {
            let until = key::clock(k.until_ns, now);
            vec![
                k.name.clone(),
                if k.ended(now) {
                    format!("ended {until}")
                } else {
                    until
                },
                k.within.clone().unwrap_or_else(|| "-".into()),
                k.opened(),
                or_dash(k.limits.describe()),
                or_dash(k.env.join(", ")),
                or_dash(k.reads()),
            ]
        })
        .collect();
    table::print(
        &["name", "until", "within", "opens", "limits", "env", "reads"],
        &rows,
    )?;
    Ok(0)
}

fn watch(p: &mut lexopt::Parser) -> Result<i32> {
    let mut since = None;
    while let Some(arg) = p.next()? {
        match arg {
            Long("since") => since = Some(p.value()?.parse()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    Ledger::open()?.follow(since, &mut io::stdout().lock())?;
    Ok(0)
}

/// Run by systemd, not by people: see `cgroup::serve`.
fn serve(p: &mut lexopt::Parser) -> Result<i32> {
    if let Some(arg) = p.next()? {
        return Err(arg.unexpected().into());
    }
    cgroup::serve()
}

fn ledger_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut last, mut verify) = (20, false);
    while let Some(arg) = p.next()? {
        match arg {
            Long("last") => last = p.value()?.parse()?,
            Long("verify") => verify = true,
            arg => return Err(arg.unexpected().into()),
        }
    }
    let ledger = Ledger::open()?;
    if verify {
        let v = ledger.verify()?;
        println!(
            "ledger ok: {} step{}, head {}",
            v.steps,
            if v.steps == 1 { "" } else { "s" },
            &v.head[..16]
        );
        return Ok(0);
    }
    let steps = ledger.last(last)?;
    if steps.is_empty() {
        hint("Nothing recorded yet: every step that changes something or runs a command will be.");
        return Ok(0);
    }
    let rows: Vec<Vec<String>> = steps
        .iter()
        .map(|s| {
            vec![
                s.seq.to_string(),
                ledger::when(s.ts),
                s.act.clone(),
                s.world.clone(),
                s.key.clone().unwrap_or_else(|| "-".into()),
                s.out.clone(),
                cost(&s.cost),
                match &s.limits {
                    Some(limits) => format!("{} [{limits}]", s.what),
                    None => s.what.clone(),
                },
            ]
        })
        .collect();
    table::print(
        &[
            "step", "time", "act", "world", "key", "result", "cost", "what",
        ],
        &rows,
    )?;
    Ok(0)
}
