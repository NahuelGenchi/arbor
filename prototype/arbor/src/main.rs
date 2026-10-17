//! `arbor`: the command people and agents use.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use arbor::cell::{self, Options};
use arbor::error::{self, Error, Result};
use arbor::ledger::{self, Cost, Ledger};
use arbor::world::{State, World};
use arbor::{paths, table, tree};
use lexopt::prelude::*;

const HELP: &str = "\
arbor: try changes on a copy of a folder, then keep them or throw them away.

  arbor new [NAME] [--from DIR]      make a world: an instant copy of DIR (default: here)
  arbor worlds                       list the worlds
  arbor shell NAME                   open a shell inside a world
  arbor run NAME -- CMD...           run one command inside a world
  arbor diff NAME [--patch]          list what the world changed (--patch: show the lines)
  arbor keep NAME [--force]          apply the world's changes to the real folder
  arbor undo NAME [--force]          take kept changes back out of the real folder
  arbor discard NAME                 throw a world away
  arbor ledger [--last N] [--verify] the record of every step (--verify: check that none was changed)

The real folder does not change until `arbor keep`. Inside a world the rest of the computer is read-only,
changes elsewhere in your home folder vanish when the command ends, and there is no network.
Read the diff before keeping: kept files such as .git/hooks run later on your computer.
";

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
        "ledger" => ledger_cmd(&mut p),
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
    what: String,
    out: Option<String>,
    cpu_ms: Option<u64>,
    mem_kib: Option<u64>,
}

/// Takes a step and records it in the ledger however it ends, with its outcome, its detail (or the first line of
/// its error) and its cost. A mistake in how arbor was called changes nothing and is not recorded. The step has
/// happened by then, so a ledger that cannot be written is a warning, not a failure.
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
    let new = ledger::New {
        act,
        world: &note.world,
        what: &note.what,
        out: &out,
        cost: Cost {
            ms: started.elapsed().as_millis() as u64,
            cpu_ms: note.cpu_ms,
            mem_kib: note.mem_kib,
        },
    };
    if let Err(e) = Ledger::open().and_then(|l| l.append(new)) {
        eprintln!("arbor: warning: this step is not in the ledger: {e}");
    }
    res
}

/// A command as a shell would take it back: arguments with spaces or quotes are quoted.
fn shown(cmd: &[OsString]) -> String {
    let word = |a: &OsString| {
        let a = a.to_string_lossy();
        let plain = !a.is_empty()
            && a.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
        if plain {
            a.into_owned()
        } else {
            format!("'{}'", a.replace('\'', "'\\''"))
        }
    };
    cmd.iter().map(word).collect::<Vec<_>>().join(" ")
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

/// Runs `cmd` in a world and notes its exit code and cost.
fn run_noted(
    note: &mut Note,
    name: &str,
    cmd: &[OsString],
    opts: &Options,
    shell: bool,
) -> Result<i32> {
    let (world, lock) = World::load_locked(name)?;
    note.what = shown(cmd);
    if shell {
        eprintln!(
            "Inside world {} over {}. Changes stay in the world; `exit` to leave.",
            world.name,
            paths::tilde(&world.base)
        );
    }
    let ran = cell::run(&world, &lock, cmd, opts)?;
    note.out = Some(format!("exit {}", ran.code));
    (note.cpu_ms, note.mem_kib) = (Some(ran.cpu_ms), Some(ran.mem_kib));
    if shell {
        hint(&format!(
            "Back outside. See the changes with `arbor diff {}`.",
            world.name
        ));
    }
    Ok(ran.code)
}

fn shell(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, _) = name_and_flags(p, &[])?;
    let sh = env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    let opts = Options { interactive: true };
    recorded("shell", &name, |note| {
        run_noted(note, &name, &[sh], &opts, true)
    })
}

fn run_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut name, mut cmd) = (None, Vec::new());
    while let Some(arg) = p.next()? {
        match arg {
            Value(v) if name.is_none() => name = Some(v.string()?),
            Value(v) => {
                cmd.push(v);
                cmd.extend(p.raw_args()?);
            }
            arg => return Err(arg.unexpected().into()),
        }
    }
    let name = name.ok_or_else(|| Error::usage("which world? usage: arbor run NAME -- CMD..."))?;
    if cmd.is_empty() {
        return Err(Error::usage(
            "which command? usage: arbor run NAME -- CMD...",
        ));
    }
    let opts = Options {
        interactive: io::stdin().is_terminal(),
    };
    recorded("run", &name, |note| {
        run_noted(note, &name, &cmd, &opts, false)
    })
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

/// Wall time, then CPU time and peak memory when a command ran.
fn cost(c: &Cost) -> String {
    let mut s = format!("{}ms", c.ms);
    if let Some(cpu) = c.cpu_ms {
        s += &format!(" cpu {cpu}ms");
    }
    if let Some(kib) = c.mem_kib {
        s += &format!(" {}MiB", kib.div_ceil(1024));
    }
    s
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
        hint("Nothing recorded yet: every new, run, shell, keep, undo and discard will be.");
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
                s.out.clone(),
                cost(&s.cost),
                s.what.clone(),
            ]
        })
        .collect();
    table::print(
        &["step", "time", "act", "world", "result", "cost", "what"],
        &rows,
    )?;
    Ok(0)
}
