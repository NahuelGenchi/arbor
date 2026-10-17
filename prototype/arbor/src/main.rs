//! `arbor`: the command people and agents use.

use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use arbor::cell::{self, Options};
use arbor::cgroup::{self, Limits};
use arbor::error::{self, Context, Error, Result};
use arbor::key::{self, Key};
use arbor::ledger::{self, Cost, Ledger};
use arbor::outbox::{self, Answer, Effect, Outbox};
use arbor::world::{self, State, World};
use arbor::{look, net, paths, table, tree};
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
  arbor key renew NAME [--for D]     move a key's end later, ended or not; its home folder stays
  arbor key take-back NAME           end a key and every key made within it, and stop their commands
  arbor outbox [ID] [--all]          list the effects waiting for you (ID: everything about one)
  arbor release ID                   run an effect a world asked for: as you, once
  arbor refuse ID [--why TEXT]       say no to an effect
  arbor ledger [--last N] [--verify] the record of every step (--verify: check that none was changed)
  arbor watch [--since N]            print each new step as a JSON line (--since N: the steps after N first)
  arbor look QUERY [+ QUERY]...      read a folder, git's status and log, processes: short tables, several
                                     in one call (`arbor look` alone says how)

OPTIONS: --key KEY opens what the key opens. LIMITS stop a command that takes too much:
--mem SIZE (512M, 2G), --cpu CPUS (0.5, 2), --pids N (processes at once), --time DURATION (90s, 10m, 2h).

GRANTS: --read PATH (a folder or file), --toolchains (~/.cargo, ~/.npm, ~/.pyenv and the like),
--env VAR (passes a variable in), --net HOST (a host it may reach over https: api.example.com,
*.example.com, HOST:PORT), --home (a home folder of its own, kept between runs), --world W (only that
world), LIMITS, --spend AMOUNT (what its effects may cost in all: 5, 2.50), --for DURATION or
--until HH:MM (8h if neither), --within KEY (a narrower key, made from another). A renewal takes --for or
--until too: without either, 8h from now, or the end of the key it was made within.

Inside a world, `arbor look` reads what the world sees, and `arbor effect [--cost AMOUNT] [--why TEXT]
[--input FILE] [--wait] -- CMD...` asks for CMD to run outside: in the real folder, as you, with the
network, once you release it. --wait waits for your answer and prints what CMD printed.

The real folder does not change until `arbor keep`. Inside a world the rest of the computer is read-only,
your home folder is empty (or the key's own, with --home) but for what a key opens, password files stay
hidden, and there is no network but the hosts a key opens.
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
    // Inside a world arbor's own folder is hidden: there, arbor can only read what the world sees and ask, through
    // the cell's socket.
    if env::var_os(outbox::SOCKET_VAR).is_some_and(|s| !s.is_empty())
        && !matches!(command.as_str(), "look" | "effect" | "help" | "proxy-port")
    {
        return Err(Error::refused(
            "inside a world, arbor only looks and asks for effects: `arbor look`, `arbor effect -- CMD` \
             (see `arbor help`)",
        ));
    }
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
        "effect" => effect(&mut p),
        "outbox" => outbox_cmd(&mut p),
        "release" => release(&mut p),
        "refuse" => refuse(&mut p),
        "ledger" => ledger_cmd(&mut p),
        "watch" => watch(&mut p),
        "look" => look_cmd(&mut p),
        "serve" => serve(&mut p),
        "proxy-port" => proxy_port(&mut p),
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
    effect: Option<u64>,
    what: String,
    limits: String,
    out: Option<String>,
    cpu_ms: Option<u64>,
    mem_kib: Option<u64>,
    pids: Option<u64>,
    cents: Option<u64>,
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
        Err(e) if e.code == error::NOT_FOUND => "not found".into(),
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
        by: "you",
        key: note.key.as_deref(),
        world: &note.world,
        cell: note.cell.as_deref(),
        effect: note.effect,
        what: &note.what,
        limits: Some(&note.limits),
        out: &out,
        cost: Cost {
            ms: started.elapsed().as_millis() as u64,
            cpu_ms: note.cpu_ms,
            mem_kib: note.mem_kib,
            pids: note.pids,
            cents: note.cents,
        },
    });
    res
}

/// `1 effect`, `3 effects`.
fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
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
            by: "you",
            key: note.key.as_deref(),
            world: &world.name,
            cell: Some(id),
            effect: None,
            what: &note.what,
            limits: Some(&note.limits),
            out: "running",
            cost: Cost {
                ms: begun.elapsed().as_millis() as u64,
                ..Cost::default()
            },
        })
    })?;
    if let Some(net) = &ran.net {
        noted_net(
            note.key.as_deref().unwrap_or_default(),
            &world.name,
            &ran.id,
            net,
        );
    }
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

/// Records what went through a cell's proxy as a step of the cell's, and names the hosts its key kept it from.
fn noted_net(key: &str, world: &str, cell: &str, net: &net::Tally) {
    if net.is_empty() {
        return;
    }
    append(ledger::New {
        act: "net",
        by: "cell",
        key: Some(key),
        world,
        cell: Some(cell),
        effect: None,
        what: &net.summary(),
        limits: None,
        out: net.outcome(),
        cost: Cost {
            ms: net.ms,
            ..Cost::default()
        },
    });
    for line in net.refusals(key) {
        eprintln!("arbor: {line}");
    }
}

/// `arbor proxy-port FD -- CMD...`, which arbor runs itself as the first process of a cell whose key opens hosts:
/// it opens the cell's proxy port, hands it out over the descriptor FD, and becomes CMD (see `net`).
fn proxy_port(p: &mut lexopt::Parser) -> Result<i32> {
    let inside = env::var_os(outbox::SOCKET_VAR).is_some_and(|s| !s.is_empty());
    let fd = match p.next()? {
        Some(Value(v)) if inside => v.string()?,
        _ => {
            return Err(Error::usage(
                "`arbor proxy-port` is run by arbor itself, as the first process of a world whose key opens hosts",
            ));
        }
    };
    let mut cmd: Vec<OsString> = p.raw_args()?.collect();
    if cmd.first().is_some_and(|a| a == "--") {
        cmd.remove(0);
    }
    if cmd.is_empty() {
        return Err(Error::usage(
            "which command? usage: arbor proxy-port FD -- CMD...",
        ));
    }
    let opened = fd
        .parse()
        .ok()
        .and_then(net::inherited)
        .ok_or_else(|| Error::new("there is no port to hand over"))
        .and_then(net::open_port);
    if let Err(e) = opened {
        eprintln!("arbor: warning: this command has no network: {e}");
    }
    Ok(cell::exec(&cmd))
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
    let mut refused = Vec::new();
    let res = recorded("discard", &name, |note| {
        let (world, lock) = World::load_locked(&name)?;
        let (state, base) = (world.state, paths::tilde(&world.base));
        // What it asked for and nobody answered goes with it. Its lock keeps commands from running in it, so
        // nothing asks again before it is gone.
        refused = refuse_waiting(
            |e| e.world == name,
            &format!("world {name} was thrown away"),
        );
        world.discard(&lock)?;
        note.what = base.clone();
        match state {
            State::Open => println!("threw away world {name}; {base} is as it was"),
            State::Kept => {
                println!("removed world {name} and its undo copy; the kept changes stay in {base}")
            }
        }
        if !refused.is_empty() {
            println!(
                "refused the {} it asked for",
                count(refused.len(), "effect")
            );
        }
        Ok(0)
    });
    record_refused(&refused);
    res
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
    if let Some(cents) = c.cents {
        s += &format!(" {}", outbox::money(cents));
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
    let usage = "usage: arbor key new NAME [GRANTS], arbor key renew NAME [--for D], \
                 or arbor key take-back NAME (see `arbor help`)";
    match p.next()? {
        Some(Value(v)) => match v.string()?.as_str() {
            "new" => key_new(p),
            "renew" => key_renew(p),
            "take-back" => take_back(p),
            other => Err(Error::usage(format!(
                "there is no `arbor key {other}`; {usage}"
            ))),
        },
        None => Err(Error::usage(usage)),
        Some(arg) => Err(arg.unexpected().into()),
    }
}

/// Reads the value of `--for DURATION` (`life`) or `--until HH:MM` into `until`, as the time it names; giving both
/// is a mistake.
fn read_end(p: &mut lexopt::Parser, life: bool, now: u64, until: &mut Option<u64>) -> Result<()> {
    let text = p.value()?.string()?;
    let at = if life {
        now.saturating_add(cgroup::parse_duration(&text)?.as_nanos() as u64)
    } else {
        key::parse_until(&text, now)?
    };
    if until.replace(at).is_some() {
        return Err(Error::usage("give --for or --until, not both"));
    }
    Ok(())
}

fn key_new(p: &mut lexopt::Parser) -> Result<i32> {
    let now = world::now_ns();
    let (mut name, mut spec) = (None, key::Spec::default());
    while let Some(arg) = p.next()? {
        match arg {
            Long("world") => spec.worlds.push(p.value()?.string()?),
            Long("read") => spec.read.push(PathBuf::from(p.value()?)),
            Long("toolchains") => spec.toolchains = true,
            Long("env") => spec.env.push(p.value()?.string()?),
            Long("net") => spec.net.push(p.value()?.string()?),
            Long("home") => spec.home = true,
            Long("within") => spec.within = Some(p.value()?.string()?),
            Long(flag @ ("for" | "until")) => {
                let life = flag == "for";
                read_end(p, life, now, &mut spec.until_ns)?;
            }
            Long(flag) if cgroup::FLAGS.contains(&flag) => {
                let flag = flag.to_string();
                spec.limits.set(&flag, &p.value()?.string()?)?;
            }
            Long("spend") => spec.spend = Some(outbox::parse_money(&p.value()?.string()?)?),
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

fn key_renew(p: &mut lexopt::Parser) -> Result<i32> {
    let now = world::now_ns();
    let (mut name, mut until) = (None, None);
    while let Some(arg) = p.next()? {
        match arg {
            Long(flag @ ("for" | "until")) => {
                let life = flag == "for";
                read_end(p, life, now, &mut until)?;
            }
            Value(v) if name.is_none() => name = Some(v.string()?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let name = name.ok_or_else(|| Error::usage("which key? (see `arbor keys`)"))?;
    recorded("renew", "-", |note| {
        note.key = Some(name.clone());
        let (k, was) = key::renew(&name, until)?;
        let now = world::now_ns();
        note.what = format!(
            "key {} until {} instead of {}",
            k.name,
            key::clock(k.until_ns, now),
            key::clock(was, now)
        );
        println!("renewed {}", note.what);
        // The keys made within it keep their own ends, and each may be renewed up to this one's.
        let within: Vec<String> = Key::list()
            .unwrap_or_default()
            .into_iter()
            .filter(|w| w.within.as_ref() == Some(&k.name))
            .map(|w| w.name)
            .collect();
        if let Some(first) = within.first() {
            hint(&format!(
                "Keys made within it keep their own ends: {}. `arbor key renew {first}` moves one later.",
                within.join(", ")
            ));
        }
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
    let mut refused = Vec::new();
    let res = recorded("take-back", "-", |note| {
        note.key = Some(name.clone());
        let keys = key::take_back(&name)?;
        let gone: Vec<String> = keys.iter().map(|k| k.name.clone()).collect();
        // A cell that started under one of them before it went is stopped; one starting now sees it gone.
        let mut stopped = 0;
        for c in cell::running()? {
            if c.key.as_ref().is_some_and(|k| gone.contains(k)) && cell::stop(&c, STOP_GRACE)? {
                stopped += 1;
            }
        }
        // And what they asked for that still waits is refused.
        refused = refuse_waiting(
            |e| e.keys.iter().any(|k| gone.contains(&k.name)),
            &format!("key {} was taken back", gone[0]),
        );
        // Nothing runs under them any more, so their own home folders go with them.
        let mut homes = 0;
        for k in &keys {
            match k.remove_home() {
                Ok(removed) => homes += usize::from(removed),
                Err(e) => eprintln!("arbor: warning: {e}"),
            }
        }
        let mut what = format!("key {}", gone[0]);
        match &gone[1..] {
            [] => {}
            [one] => what += &format!(", and key {one} made within it"),
            more => what += &format!(", and keys {} made within it", more.join(", ")),
        }
        what += &format!("; stopped {}", count(stopped, "command"));
        if !refused.is_empty() {
            what += &format!("; refused {}", count(refused.len(), "effect"));
        }
        if homes > 0 {
            what += &format!("; removed {}", count(homes, "home folder"));
        }
        println!("took back {what}");
        note.what = what;
        Ok(0)
    });
    record_refused(&refused);
    res
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
    // A key's own home folder shows what it takes on disk, and one it shares, whose it is.
    let home = |k: &Key| match &k.home {
        None => "-".into(),
        Some(owner) if !k.owns_home() => format!("of {owner}"),
        Some(_) => k
            .home_dir()
            .ok()
            .flatten()
            .map_or("-".into(), |dir| cgroup::size(key::disk_usage(&dir))),
    };
    let effects = Outbox::open()?.list()?;
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|k| {
            let spent = outbox::spent(&effects, &k.id());
            let spend = match k.spend {
                Some(most) => format!("{} of {}", outbox::money(spent), outbox::money(most)),
                None if spent > 0 => outbox::money(spent),
                None => "-".into(),
            };
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
                spend,
                or_dash(k.env.join(", ")),
                or_dash(k.hosts()),
                home(k),
                or_dash(k.reads()),
            ]
        })
        .collect();
    table::print(
        &[
            "name", "until", "within", "opens", "limits", "spend", "env", "net", "home", "reads",
        ],
        &rows,
    )?;
    Ok(0)
}

/// `arbor look`: several reads in one call, each a short table. Alone, it says what it answers.
fn look_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let args: Vec<OsString> = p.raw_args()?.collect();
    if args.is_empty() || matches!(args[0].to_str(), Some("-h" | "--help" | "help")) {
        print!("{}", look::ABOUT);
        return Ok(0);
    }
    let queries = look::parse(args)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let all = look::answer(&queries, &mut out)?;
    out.flush()?;
    Ok(if all { 0 } else { 1 })
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
                match (&s.limits, s.effect) {
                    (Some(limits), _) => format!("{} [{limits}]", s.what),
                    (None, Some(id)) => format!("#{id} {}", s.what),
                    (None, None) => s.what.clone(),
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

/// Refuses the waiting effects that `which` picks, because `why`, and returns them for `record_refused`. What
/// refuses them goes ahead without them, so a failure here is a warning.
fn refuse_waiting(which: impl Fn(&Effect) -> bool, why: &str) -> Vec<Effect> {
    Outbox::open()
        .and_then(|o| o.refuse_waiting(which, why))
        .unwrap_or_else(|err| {
            eprintln!("arbor: warning: effects that would be refused still wait: {err}");
            Vec::new()
        })
}

/// Records effects refused by a step, after that step's own record.
fn record_refused(refused: &[Effect]) {
    for e in refused {
        append(ledger::New {
            act: "refuse",
            by: "you",
            key: e.key(),
            world: &e.world,
            cell: Some(&e.cell),
            effect: Some(e.id),
            what: &e.shown(),
            limits: None,
            out: "ok",
            cost: Cost::default(),
        });
    }
}

/// Notes which effect a step is about.
fn noted(note: &mut Note, e: &Effect) {
    note.world = e.world.clone();
    note.key = e.key().map(Into::into);
    note.cell = Some(e.cell.clone());
    note.effect = Some(e.id);
    note.what = e.shown();
}

/// Reads an effect's number: `3` or `#3`.
fn effect_id(text: &str) -> Result<u64> {
    text.strip_prefix('#').unwrap_or(text).parse().map_err(|_| {
        Error::usage(format!(
            "'{text}' is not an effect's number (see `arbor outbox`)"
        ))
    })
}

/// An exit code for arbor itself: one that arbor could not learn is a failure, never a success.
fn exit_code(exit: i32) -> i32 {
    if exit < 0 { 1 } else { exit }
}

/// Reads an effect's input, up to the most one may carry.
fn read_input(r: impl Read) -> Result<Vec<u8>> {
    let mut input = Vec::new();
    r.take(outbox::MAX_INPUT as u64 + 1)
        .read_to_end(&mut input)?;
    if input.len() > outbox::MAX_INPUT {
        return Err(Error::usage(format!(
            "the input is larger than {}",
            cgroup::size(outbox::MAX_INPUT as u64)
        )));
    }
    Ok(input)
}

/// `arbor effect`, inside a world: asks for a command to run outside, once a person releases it.
fn effect(p: &mut lexopt::Parser) -> Result<i32> {
    let socket = env::var_os(outbox::SOCKET_VAR)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::usage(
                "`arbor effect` asks from inside a world (`arbor shell` or `arbor run`); outside one, run \
                 the command yourself",
            )
        })?;
    let (mut cost, mut why, mut input, mut wait) = (None, String::new(), None, false);
    let mut cmd = Vec::new();
    while let Some(arg) = p.next()? {
        match arg {
            Long("cost") => cost = Some(outbox::parse_money(&p.value()?.string()?)?),
            Long("why") => why = p.value()?.string()?,
            Long("input") => input = Some(PathBuf::from(p.value()?)),
            Long("wait") => wait = true,
            Value(v) => {
                cmd.push(v);
                cmd.extend(p.raw_args()?);
            }
            arg => return Err(arg.unexpected().into()),
        }
    }
    if cmd.is_empty() {
        return Err(Error::usage(
            "which command? usage: arbor effect [--cost AMOUNT] [--why TEXT] [--input FILE] [--wait] -- CMD...",
        ));
    }
    let cmd = cmd
        .into_iter()
        .map(|a| {
            a.into_string()
                .map_err(|_| Error::usage("an effect's command must be text (UTF-8)"))
        })
        .collect::<Result<Vec<_>>>()?;
    let input = match input {
        None => Vec::new(),
        Some(f) if f.as_os_str() == "-" => read_input(io::stdin().lock())?,
        Some(f) => read_input(File::open(&f).ctx(|| format!("cannot read {}", f.display()))?)?,
    };
    let dir = env::current_dir()
        .ok()
        .and_then(|d| d.into_os_string().into_string().ok())
        .unwrap_or_default();
    let ask = outbox::Ask {
        cmd,
        dir,
        cost,
        why,
        input,
        wait,
    };
    let mut asked = 0;
    let answer = outbox::ask(Path::new(&socket), &ask, &mut |id| {
        asked = id;
        // With --wait, what arbor prints on its output is the effect's own.
        if wait {
            eprintln!("effect {id} waiting");
        } else {
            println!("effect {id} waiting");
        }
        hint(&format!(
            "A person answers it outside the world: `arbor release {id}` runs it, once; `arbor refuse {id}` says no."
        ));
    })?;
    match answer {
        None => Ok(0),
        Some(Answer::Done { exit, output }) => {
            io::stdout().write_all(&output)?;
            Ok(exit_code(exit))
        }
        Some(Answer::Refused { why }) if why.is_empty() => {
            Err(Error::refused(format!("effect {asked} was refused")))
        }
        Some(Answer::Refused { why }) => Err(Error::refused(format!(
            "effect {asked} was refused: {}",
            table::escaped(&why)
        ))),
    }
}

/// `waiting`, `running`, `unknown` (its arbor stopped while it ran), `exit N` or `refused`.
fn effect_state(outbox: &Outbox, e: &Effect) -> String {
    match e.state {
        outbox::State::Waiting => "waiting".into(),
        outbox::State::Running if outbox.still_running(e.id) => "running".into(),
        outbox::State::Running => "unknown".into(),
        outbox::State::Done => e
            .receipt
            .as_ref()
            .map_or("done".into(), |r| format!("exit {}", r.exit)),
        outbox::State::Refused => "refused".into(),
    }
}

fn outbox_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut id, mut all) = (None, false);
    while let Some(arg) = p.next()? {
        match arg {
            Long("all") => all = true,
            Value(v) if id.is_none() => id = Some(effect_id(&v.string()?)?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let outbox = Outbox::open()?;
    if let Some(id) = id {
        return show_effect(&outbox, &outbox.load(id)?);
    }
    let effects: Vec<Effect> = outbox
        .list()?
        .into_iter()
        .filter(|e| all || matches!(e.state, outbox::State::Waiting | outbox::State::Running))
        .collect();
    if effects.is_empty() {
        hint(if all {
            "Nothing was asked for yet. Inside a world, `arbor effect -- CMD` asks."
        } else {
            "Nothing waits for you."
        });
        return Ok(0);
    }
    let now = world::now_ns();
    let rows: Vec<Vec<String>> = effects
        .iter()
        .map(|e| {
            vec![
                e.id.to_string(),
                effect_state(&outbox, e),
                e.world.clone(),
                e.key().unwrap_or("-").into(),
                e.cost.map_or("-".into(), outbox::money),
                key::clock(e.asked_ns, now),
                e.shown(),
            ]
        })
        .collect();
    table::print(
        &["id", "state", "world", "key", "cost", "asked", "what"],
        &rows,
    )?;
    Ok(0)
}

/// Everything about one effect, for the person who answers it: the facts first, the asker's words last.
fn show_effect(outbox: &Outbox, e: &Effect) -> Result<i32> {
    let now = world::now_ns();
    let at = key::clock(e.answered_ns.unwrap_or(e.asked_ns), now);
    let state = match effect_state(outbox, e).as_str() {
        "waiting" => format!("waiting since {}", key::clock(e.asked_ns, now)),
        "running" => format!("released at {at}, running"),
        "unknown" => {
            format!("released at {at}; arbor stopped while it ran, so what it did is not known")
        }
        "refused" => format!("refused at {at}"),
        ended => format!("released at {at}: {ended}"),
    };
    let mut out = io::stdout().lock();
    writeln!(out, "effect {}, {state}", e.id)?;
    let mut line = |label: &str, text: String| writeln!(out, "  {label:<8}{text}");
    line("runs", e.shown())?;
    line(
        "in",
        format!(
            "{}, the real folder, as you, with the network",
            table::escaped(&paths::tilde(&e.folder()))
        ),
    )?;
    if let Some(n) = e.input {
        line(
            "input",
            format!(
                "{}, in {}",
                cgroup::size(n),
                paths::tilde(&outbox.input_file(e.id))
            ),
        )?;
    }
    let mut cost = e.cost.map_or("none given".into(), |c| {
        format!("{}, by the asker's word", outbox::money(c))
    });
    let all = outbox.list()?;
    for k in &e.keys {
        if let Ok(key) = Key::load(&k.name)
            && key.made_ns == k.made_ns
            && let Some(most) = key.spend
        {
            cost += &format!(
                "; key {} has spent {} of {}",
                key.name,
                outbox::money(outbox::spent(&all, k)),
                outbox::money(most)
            );
        }
    }
    line("cost", cost)?;
    line(
        "asked",
        format!(
            "by cell {} in world {}{}",
            e.cell,
            e.world,
            e.key().map_or(String::new(), |k| format!(", with key {k}"))
        ),
    )?;
    if e.back {
        line(
            "back",
            "what it prints goes back to the command that asked".into(),
        )?;
    }
    if let Some(r) = &e.receipt {
        line(
            "output",
            format!(
                "{}, SHA-256 {}, in {}",
                cgroup::size(r.output),
                &r.sha256[..16],
                paths::tilde(&outbox.output_file(e.id))
            ),
        )?;
        line("took", format!("{}ms, cpu {}ms", r.ms, r.cpu_ms))?;
    }
    if !e.refusal.is_empty() {
        line("refused", table::escaped(&e.refusal))?;
    }
    if !e.why.is_empty() {
        line(
            "why",
            format!(
                "(the asker's words, not checked) {}",
                table::escaped(&e.why)
            ),
        )?;
    }
    if e.state == outbox::State::Waiting {
        hint(&format!(
            "`arbor release {0}` runs it, once; `arbor refuse {0}` says no.",
            e.id
        ));
    }
    Ok(0)
}

fn release(p: &mut lexopt::Parser) -> Result<i32> {
    let mut id = None;
    while let Some(arg) = p.next()? {
        match arg {
            Value(v) if id.is_none() => id = Some(effect_id(&v.string()?)?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let id = id.ok_or_else(|| Error::usage("which effect? (see `arbor outbox`)"))?;
    recorded("release", "-", |note| {
        let outbox = Outbox::open()?;
        noted(note, &outbox.load(id)?);
        let (e, out) = outbox.begin(id)?;
        // An effect leaves only once the ledger says so.
        let begun = Ledger::open().and_then(|l| {
            l.append(ledger::New {
                act: "release",
                by: "you",
                key: e.key(),
                world: &e.world,
                cell: Some(&e.cell),
                effect: Some(id),
                what: &note.what,
                limits: None,
                out: "running",
                cost: Cost::default(),
            })
        });
        if let Err(err) = begun {
            outbox.unbegin(id);
            return Err(Error::new(format!(
                "effect {id} was not released, because the ledger cannot record it: {err}"
            )));
        }
        hint(&format!(
            "Releasing effect {id}: {} in {}",
            note.what,
            table::escaped(&paths::tilde(&e.folder()))
        ));
        let receipt = match outbox::run(&outbox, &e, out, &mut io::stdout()) {
            Ok(receipt) => receipt,
            Err(err) => {
                outbox.unbegin(id);
                return Err(err);
            }
        };
        // It ran, so it cost what it said.
        (note.out, note.cpu_ms, note.cents) = (
            Some(format!("exit {}", receipt.exit)),
            Some(receipt.cpu_ms),
            e.cost,
        );
        let code = exit_code(receipt.exit);
        if let Err(err) = outbox.finish(id, receipt) {
            eprintln!("arbor: warning: effect {id} ran, but its receipt was not kept: {err}");
        }
        hint(&format!(
            "Effect {id} ran, once. Its receipt is in the ledger and in `arbor outbox {id}`."
        ));
        Ok(code)
    })
}

fn refuse(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut id, mut why) = (None, String::new());
    while let Some(arg) = p.next()? {
        match arg {
            Long("why") => why = p.value()?.string()?,
            Value(v) if id.is_none() => id = Some(effect_id(&v.string()?)?),
            arg => return Err(arg.unexpected().into()),
        }
    }
    let id = id.ok_or_else(|| Error::usage("which effect? (see `arbor outbox`)"))?;
    recorded("refuse", "-", |note| {
        let outbox = Outbox::open()?;
        noted(note, &outbox.load(id)?);
        outbox.refuse(id, &why)?;
        println!("refused effect {id}: {}", note.what);
        Ok(0)
    })
}
