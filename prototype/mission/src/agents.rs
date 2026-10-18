//! Start an agent safely: the agent programs (harnesses) found on this computer, and what Start runs for one. It
//! makes a try of the folder (`arbor new`) and a key for that try alone (`arbor key new --world --home`), which reads
//! only the harness's own program and reaches only the hosts it needs, for a set time, then opens a terminal with the
//! harness running in a room of the try (`arbor run --key`). Like every button, it runs the commands a person would
//! type, so the ledger records each step with you as the actor. The key's own home folder holds any login made
//! inside; the person's real logins stay hidden, as every cell already hides them. Nothing here touches GTK.

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use arbor::world::{self, World};

use crate::act::{arbor, run};

/// How long a key made by Start lasts, in hours.
pub const HOURS: u64 = 8;

/// A harness arbor knows how to start: where it is usually installed, and the hosts it needs.
struct Known {
    id: &'static str,
    name: &'static str,
    /// Its program's name on the PATH.
    command: &'static str,
    /// Where its own installer puts it, under the home folder.
    in_home: &'static [&'static str],
    /// Its folder, rather than its program alone, is what the key reads: the program needs what lies beside it.
    folder: bool,
    hosts: &'static [&'static str],
}

/// OpenCode's hosts are the two it reached in #54's 44 commands, and #69's and #74's whole sessions. Claude Code's
/// are its API and its two sign-in hosts, from its network requirements; its optional telemetry is left out, and
/// arbor names it when refused. Codex's are its API and its two ChatGPT sign-in hosts, from its source; they are not
/// checked by running it.
const KNOWN: &[Known] = &[
    Known {
        id: "opencode",
        name: "OpenCode",
        command: "opencode",
        in_home: &[".opencode/bin/opencode"],
        folder: true,
        hosts: &["opencode.ai", "models.opencode.ai"],
    },
    Known {
        id: "claude",
        name: "Claude Code",
        command: "claude",
        in_home: &[".local/bin/claude", ".claude/local/claude"],
        folder: false,
        hosts: &["api.anthropic.com", "claude.ai", "platform.claude.com"],
    },
    Known {
        id: "codex",
        name: "Codex",
        command: "codex",
        in_home: &[".local/bin/codex", ".codex/bin/codex"],
        folder: false,
        hosts: &["api.openai.com", "auth.openai.com", "chatgpt.com"],
    },
];

/// A harness on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    pub id: String,
    pub name: String,
    /// The program, resolved: the room runs it by this path.
    pub program: PathBuf,
    /// What the key reads so the room sees the program: none when it lies outside the home folder and the drives,
    /// which every cell sees already, read-only.
    pub read: Option<PathBuf>,
    pub hosts: Vec<String>,
}

/// The harnesses found, in the order of `KNOWN`: each one's usual place in `home`, else the first on `path`.
pub fn found(home: Option<&Path>, path: Option<&OsStr>) -> Vec<Harness> {
    let on_path = |cmd: &str| -> Option<PathBuf> {
        std::env::split_paths(path?)
            .map(|d| d.join(cmd))
            .find(|p| runnable(p))
    };
    let hidden = |p: &Path| {
        ["/home", "/media", "/mnt"].iter().any(|h| p.starts_with(h))
            || home.is_some_and(|h| p.starts_with(h))
    };
    KNOWN
        .iter()
        .filter_map(|k| {
            let at = home
                .into_iter()
                .flat_map(|h| k.in_home.iter().map(move |p| h.join(p)))
                .find(|p| runnable(p))
                .or_else(|| on_path(k.command))?;
            let program = std::fs::canonicalize(&at).ok()?;
            let read = hidden(&program).then(|| match k.folder {
                true => program.parent().map_or(program.clone(), Path::to_path_buf),
                false => program.clone(),
            });
            Some(Harness {
                id: k.id.into(),
                name: k.name.into(),
                program,
                read,
                hosts: k.hosts.iter().map(|h| h.to_string()).collect(),
            })
        })
        .collect()
}

fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A name for the try: the folder's own, made usable, and not one in use (`shop`, then `shop-2`).
pub fn try_name(folder: &Path, taken: &dyn Fn(&str) -> bool) -> String {
    let raw = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut name: String = raw
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '.' => c,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(48)
        .collect();
    if name.is_empty() {
        name = "agent".into();
    }
    (1..)
        .map(|n| match n {
            1 => name.clone(),
            n => format!("{name}-{n}"),
        })
        .find(|n| world::check_name(n).is_ok() && !taken(n))
        .unwrap_or(name)
}

/// The terminal that opens the room: ARBOR_TERMINAL, a command and its options before the command it runs, else the
/// first one found of the desktop's own and the usual ones. `env` in ARBOR_TERMINAL runs the room with no terminal,
/// as the suite does.
pub fn terminal(path: Option<&OsStr>) -> Option<Vec<String>> {
    if let Some(t) = std::env::var("ARBOR_TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
    {
        return Some(t.split_whitespace().map(String::from).collect());
    }
    const TERMINALS: &[&[&str]] = &[
        &["xdg-terminal-exec"],
        &["ptyxis", "--"],
        &["gnome-terminal", "--"],
        &["kgx", "-e"],
        &["konsole", "-e"],
        &["xfce4-terminal", "-x"],
        &["kitty"],
        &["foot"],
        &["alacritty", "-e"],
        &["xterm", "-e"],
    ];
    let dirs: Vec<PathBuf> = std::env::split_paths(path?).collect();
    TERMINALS
        .iter()
        .find(|t| dirs.iter().any(|d| runnable(&d.join(t[0]))))
        .map(|t| t.iter().map(|s| s.to_string()).collect())
}

/// What Start did, or why it did not.
#[derive(Debug, Clone)]
pub struct Started {
    pub world: String,
    pub key: String,
    pub harness: Harness,
    pub folder: PathBuf,
    pub until_ns: u64,
    pub failed: Option<String>,
    pub ms: u64,
}

/// The name the next try of `folder` gets: one no world and no key has.
pub fn next_name(folder: &Path) -> String {
    try_name(folder, &|n| World::load(n).is_ok() || arbor::key::exists(n))
}

/// Makes try `world` of `folder` and a key of the same name for it alone, then opens the terminal `term` with the
/// harness in a room of the try. A key that cannot be made leaves no try behind.
pub fn start(world: &str, folder: &Path, harness: &Harness, term: &[String]) -> Started {
    let started = Instant::now();
    let (world, key) = (world.to_string(), world.to_string());
    let mut out = Started {
        world: world.clone(),
        key: key.clone(),
        harness: harness.clone(),
        folder: folder.to_path_buf(),
        until_ns: world::now_ns() + HOURS * 3600 * 1_000_000_000,
        failed: None,
        ms: 0,
    };
    let failed = |ran: crate::act::Ran, what: &str| match ran.said.as_str() {
        "" => Some(format!(
            "arbor stopped with exit code {} while making the {what}",
            ran.code.unwrap_or(-1)
        )),
        said => Some(said.to_string()),
    };
    let folder_s = folder.to_string_lossy();
    let made = run(&["new", &world, "--from", &folder_s]);
    if made.code != Some(0) {
        out.failed = failed(made, "try");
        out.ms = started.elapsed().as_millis() as u64;
        return out;
    }
    let hours = format!("{HOURS}h");
    let mut args: Vec<String> = [
        "key", "new", &key, "--world", &world, "--home", "--for", &hours,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(r) = &harness.read {
        args.extend(["--read".into(), r.to_string_lossy().into_owned()]);
    }
    for h in &harness.hosts {
        args.extend(["--net".into(), h.clone()]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let keyed = run(&args);
    if keyed.code != Some(0) {
        out.failed = failed(keyed, "key");
        run(&["discard", &world]);
        out.ms = started.elapsed().as_millis() as u64;
        return out;
    }
    if let Ok(k) = arbor::key::Key::load(&key) {
        out.until_ns = k.until_ns;
    }
    // The terminal is a process of its own, in a process group of its own, so it outlives the window.
    let (prog, opts) = term.split_first().expect("a terminal command");
    let spawned = Command::new(prog)
        .args(opts)
        .arg(arbor())
        .args(["run", &world, "--key", &key, "--"])
        .arg(&harness.program)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(e) = spawned {
        out.failed = Some(format!("the terminal ({prog}) did not open: {e}"));
    }
    out.ms = started.elapsed().as_millis() as u64;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn program(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn only_the_harnesses_found_are_listed() {
        let root = std::env::temp_dir().join(format!("mission-agents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let bin = root.join("bin");
        program(&home.join(".opencode/bin/opencode"));
        // Claude Code's link in ~/.local/bin points to the version it runs.
        program(&home.join(".local/share/claude/versions/9.9.9"));
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::os::unix::fs::symlink(
            home.join(".local/share/claude/versions/9.9.9"),
            home.join(".local/bin/claude"),
        )
        .unwrap();
        // Not runnable: not listed.
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("codex"), "").unwrap();
        let home = fs::canonicalize(&home).unwrap();
        let found = found(Some(&home), Some(bin.as_os_str()));
        let names: Vec<&str> = found.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["OpenCode", "Claude Code"]);
        assert_eq!(found[0].read, Some(home.join(".opencode/bin")));
        assert_eq!(found[0].hosts, ["opencode.ai", "models.opencode.ai"]);
        assert_eq!(
            found[1].program,
            home.join(".local/share/claude/versions/9.9.9")
        );
        assert_eq!(found[1].read.as_ref(), Some(&found[1].program));
        // One on the PATH outside the home folder is seen by every cell already: the key reads nothing for it.
        let sys = root.join("usr/bin");
        program(&sys.join("codex"));
        let found = super::found(None, Some(sys.as_os_str()));
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].id.as_str(), found[0].read.as_ref()),
            ("codex", None)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_try_is_named_after_its_folder() {
        let none = |_: &str| false;
        assert_eq!(try_name(Path::new("/x/Shop Front"), &none), "shop-front");
        assert_eq!(try_name(Path::new("/x/.hidden"), &none), "hidden");
        assert_eq!(try_name(Path::new("/"), &none), "agent");
        let taken = |n: &str| n == "shop" || n == "shop-2";
        assert_eq!(try_name(Path::new("/x/shop"), &taken), "shop-3");
    }

    #[test]
    fn the_terminal_is_one_found() {
        let root = std::env::temp_dir().join(format!("mission-term-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        program(&root.join("gnome-terminal"));
        program(&root.join("xterm"));
        if std::env::var_os("ARBOR_TERMINAL").is_none() {
            assert_eq!(
                terminal(Some(root.as_os_str())),
                Some(vec!["gnome-terminal".into(), "--".into()])
            );
        }
        assert_eq!(
            terminal(Some(OsStr::new("/nowhere"))).is_some(),
            std::env::var_os("ARBOR_TERMINAL").is_some()
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
