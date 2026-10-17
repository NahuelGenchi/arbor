//! `arbor`: the command people and agents use.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use arbor::cell::{self, Options};
use arbor::error::{Error, Result};
use arbor::world::{State, World};
use arbor::{paths, table, tree};
use lexopt::prelude::*;

const HELP: &str = "\
arbor: try changes on a copy of a folder, then keep them or throw them away.

  arbor new [NAME] [--from DIR]      make a world: an instant copy of DIR (default: here)
  arbor worlds                       list the worlds
  arbor shell NAME [--net]           open a shell inside a world
  arbor run NAME [--net] -- CMD...   run one command inside a world
  arbor diff NAME [--patch]          list what the world changed (--patch: show the lines)
  arbor keep NAME [--force]          apply the world's changes to the real folder
  arbor undo NAME [--force]          take kept changes back out of the real folder
  arbor discard NAME                 throw a world away

The real folder does not change until `arbor keep`. Inside a world the rest of the computer is read-only,
changes elsewhere in your home folder vanish when the command ends, and there is no network unless --net.
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
    let world = World::create(name.as_deref(), &from)?;
    println!(
        "made world {} over {}",
        world.name,
        paths::tilde(&world.base)
    );
    hint(&format!(
        "Work in it with `arbor shell {}`; nothing in the real folder changes until `arbor keep {}`.",
        world.name, world.name
    ));
    Ok(0)
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

fn shell(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, flags) = name_and_flags(p, &["net"])?;
    let world = World::load(&name)?;
    let sh = env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    eprintln!(
        "Inside world {} over {}. Changes stay in the world; `exit` to leave.",
        world.name,
        paths::tilde(&world.base)
    );
    let code = cell::run(
        &world,
        &[sh],
        &Options {
            net: flags.iter().any(|f| f == "net"),
            interactive: true,
        },
    )?;
    hint(&format!(
        "Back outside. See the changes with `arbor diff {}`.",
        world.name
    ));
    Ok(code)
}

fn run_cmd(p: &mut lexopt::Parser) -> Result<i32> {
    let (mut name, mut net, mut cmd) = (None, false, Vec::new());
    while let Some(arg) = p.next()? {
        match arg {
            Long("net") => net = true,
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
    let world = World::load(&name)?;
    cell::run(
        &world,
        &cmd,
        &Options {
            net,
            interactive: io::stdin().is_terminal(),
        },
    )
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
    let mut world = World::load(&name)?;
    let _lock = world.lock()?;
    let kept = tree::keep(&mut world, flags.iter().any(|f| f == "force"))?;
    let applied: Vec<_> = kept
        .changes
        .iter()
        .filter(|c| c.mark != '?')
        .cloned()
        .collect();
    println!(
        "kept world {} in {}: {}",
        world.name,
        paths::tilde(&world.base),
        summary(&applied)
    );
    if kept.skipped > 0 {
        println!(
            "skipped {} special file(s) (pipes, sockets or devices)",
            kept.skipped
        );
    }
    hint(&format!("`arbor undo {}` takes them back out.", world.name));
    Ok(0)
}

fn undo(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, flags) = name_and_flags(p, &["force"])?;
    let mut world = World::load(&name)?;
    let _lock = world.lock()?;
    tree::undo(&mut world, flags.iter().any(|f| f == "force"))?;
    let changes = tree::changes(&world, &tree::entries(&world)?)?;
    println!(
        "undid world {}: {} taken back out of {}; the world is open again",
        world.name,
        summary(&changes),
        paths::tilde(&world.base)
    );
    Ok(0)
}

fn discard(p: &mut lexopt::Parser) -> Result<i32> {
    let (name, _) = name_and_flags(p, &[])?;
    let world = World::load(&name)?;
    let lock = world.lock()?;
    let (state, base) = (world.state, paths::tilde(&world.base));
    world.discard(&lock)?;
    match state {
        State::Open => println!("threw away world {name}; {base} is as it was"),
        State::Kept => {
            println!("removed world {name} and its undo copy; the kept changes stay in {base}")
        }
    }
    Ok(0)
}
