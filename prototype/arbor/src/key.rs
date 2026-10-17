//! Keys: what a command in a world may open beyond the world, until when, within which limits, and what the effects
//! it asks for may cost. A cell starts sealed (see `cell`): the home folders are empty, the environment is cleared
//! and there is no network. A key opens folders and files there, passes variables in by name, opens the network to
//! the hosts it names (see `net`), can give the cell a home folder of its own that is kept between runs, and caps the
//! cell's limits, until a set time, like a valet key. A key can be narrowed into another, never widened, and taking
//! one back takes back every key made within it.
//!
//! A key holds names and paths, never a secret's value. Each is a file in the state folder's `keys`. A key's own
//! home folder is in its `homes`: it holds what the commands under the key write there, such as an agent's settings
//! and login, and lives as long as the key. Renewing a key moves its end later, never sooner, so its home, and what
//! the agent keeps there, can outlive the end it was made with.

use std::env;
use std::ffi::{CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cgroup::Limits;
use crate::error::{Context, Error, Result};
use crate::net::Host;
use crate::outbox::{self, Cents};
use crate::{paths, sys, world};

/// How long a key lasts when nobody says: a working day.
pub const DEFAULT_LIFE: Duration = Duration::from_secs(8 * 3600);

/// The language toolchains `--toolchains` opens where they exist, relative to the home folder.
pub const TOOLCHAINS: [&str; 19] = [
    // Rust
    ".cargo",
    ".rustup",
    // JavaScript
    ".nvm",
    ".npm",
    // npm's own folder for tools installed with `npm install -g` without root
    ".npm-global",
    ".bun",
    ".deno",
    ".volta",
    ".local/share/pnpm",
    // Python
    ".pyenv",
    ".local/bin",
    ".local/lib",
    ".cache/pip",
    // Go, Ruby and Java
    "go",
    ".rbenv",
    ".gem",
    ".sdkman",
    ".gradle",
    ".m2",
];

/// Password and key files and folders, relative to the home folder. They stay hidden in a cell even when its world
/// or its key includes the folder they are in, unless that world or key is the file or folder itself. One missing
/// when the cell starts stays missing in it.
pub const SECRETS: [&str; 67] = [
    // Keys and passwords
    ".ssh",
    ".gnupg",
    ".password-store",
    ".local/share/keyrings",
    ".pki",
    // Cloud and service logins
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".docker/config.json",
    ".config/gh",
    ".git-credentials",
    ".netrc",
    ".vault-token",
    ".terraform.d/credentials.tfrc.json",
    // Package registry tokens
    ".npmrc",
    ".pypirc",
    ".gem/credentials",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gradle/gradle.properties",
    ".m2/settings.xml",
    ".m2/settings-security.xml",
    // Browser and mail profiles: saved passwords and signed-in sessions
    ".mozilla",
    "snap/firefox",
    ".thunderbird",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
    // AI agents' logins. OpenCode keeps its own in a database, next to every session, and writes the latest changes to
    // files beside it that come and go, so its whole folder stays hidden.
    ".local/share/opencode",
    ".claude/.credentials.json",
    // Claude Code's own file, which holds its sign-in session and personal MCP servers' settings, and its copies
    ".claude.json",
    ".claude/backups",
    // Anthropic's profiles, where Claude Code keeps a Console login made without an API key
    ".config/anthropic/credentials",
    ".codex/auth.json",
    // Codex's logins to MCP servers, where there is no keyring
    ".codex/.credentials.json",
    // Codex's logins, encrypted, where it keeps their key in the keyring
    ".codex/secrets",
    // Whole folders, of the agents that replace a login by renaming a new file over it, a file they first write under
    // another name that no list names, or that keep logins in a database or in files that change between versions:
    // Gemini CLI, Qwen Code, Copilot in editors, Amp, Goose, Crush, Factory's Droid, Cline, Kilo, OpenHands and Amazon
    // Q, and Cursor's editor, whose database holds the login with all the rest of its state.
    ".gemini",
    ".qwen",
    ".config/github-copilot",
    ".local/share/amp",
    ".config/goose",
    ".local/share/crush",
    ".factory",
    ".cline/data",
    ".local/share/kilo",
    ".openhands",
    ".local/share/amazon-q",
    ".config/Cursor",
    // The browser profile Gemini CLI's browser agent signs in to sites with, which stays hidden from a key that opens
    // Gemini's folder for its own login
    ".gemini/cli-browser-profile",
    // Kimi Code's logins and the API keys in its settings, and those of Kimi CLI before it
    ".kimi-code/credentials",
    ".kimi-code/config.toml",
    ".kimi/credentials",
    ".kimi/config.toml",
    // The Copilot CLI's login, and its MCP servers' logins and secrets, where there is no keyring
    ".copilot/config.json",
    ".copilot/mcp-oauth-config",
    ".copilot/mcp-secrets",
    // Cursor's CLI, and its MCP servers' logins, one file per project
    ".config/cursor/auth.json",
    ".cursor/projects",
    // Aider's OpenRouter key, and the API keys in its settings
    ".aider/oauth-keys.env",
    ".aider.conf.yml",
    // Augment's Auggie, and Pi
    ".augment/session.json",
    ".pi/agent/auth.json",
    // API keys that Gemini CLI, Qwen Code and Aider read from the home folder, that Mistral Vibe keeps where there is
    // no keyring, and that Continue's settings refer to
    ".env",
    ".vibe/.env",
    ".continue/.env",
];

/// What AI agents keep of your sessions, relative to the home folder: the transcripts, with each file read and each
/// command's output, the prompts you typed, their notes from past sessions, and their copies of files, pastes and
/// logs. They stay hidden like `SECRETS`, by the same rules. A `*` in an entry's last part stands for any run of
/// characters, for names that change between versions and the files SQLite keeps beside a database.
pub const RECORDS: [&str; 35] = [
    // Claude Code: transcripts, with each project's notes; your prompts; files as they were before an edit; pastes
    // and attachments; plans and task lists; debug logs; transcripts packed for feedback, and drafts; `/insights`
    // reports; background sessions; subagents' notes
    ".claude/projects",
    ".claude/history.jsonl",
    ".claude/file-history",
    ".claude/paste-cache",
    ".claude/image-cache",
    ".claude/uploads",
    ".claude/plans",
    ".claude/tasks",
    ".claude/todos",
    ".claude/debug",
    ".claude/feedback-bundles",
    ".claude/feedback",
    ".claude/usage-data",
    ".claude/jobs",
    ".claude/agent-memory",
    // Codex: transcripts, archived ones too; the messages you sent; sessions' names; logs; memories of past chats
    ".codex/sessions",
    ".codex/archived_sessions",
    ".codex/history.jsonl",
    ".codex/session_index.jsonl",
    ".codex/log",
    ".codex/memories",
    ".codex/memories_v2",
    ".codex/memories_extensions",
    // Codex's databases, with the threads' history and the queue of your messages, named by version, and the files
    // SQLite keeps beside them
    ".codex/*.sqlite*",
    // OpenCode's prompt history, stashed prompts and recently opened files. Its sessions are in its data folder,
    // among `SECRETS`.
    ".local/state/opencode",
    // Gemini CLI: for each project, transcripts, your prompts and shell commands, saved chats, checkpoints, plans,
    // long tool outputs and its notes; and the git repositories with each project's snapshots for its checkpoints.
    // Its whole folder is among `SECRETS`, so these stay hidden from a key that opens that folder by name.
    ".gemini/tmp",
    ".gemini/history",
    // The Copilot CLI: each session's events, plan, reports and copies of files before an edit; the database of all
    // sessions and the files SQLite keeps beside it; your prompts, in a folder, or a file before; logs; and sessions
    // from versions before 1.0
    ".copilot/session-state",
    ".copilot/session-store.db*",
    ".copilot/command-history-state*",
    ".copilot/logs",
    ".copilot/history-session-state",
    // Aider: the chat, the lines you typed, and, when turned on, the messages to and from the model. It keeps them in
    // the project's folder, so they are found here only where it ran in the home folder, and in a project by
    // `PROJECT_RECORDS`.
    ".aider.chat.history.md",
    ".aider.input.history",
    ".aider.llm.history",
];

/// What AI agents keep of your sessions in a project's folder rather than the home folder, relative to that folder.
/// They stay hidden like `RECORDS`, by the same rules, in the folders where an agent would have written them (see
/// `in_project`), but only those there when the cell starts: keeping the others missing would take a whiteout in
/// every folder a cell looks in.
pub const PROJECT_RECORDS: [&str; 4] = [
    // Aider: the chat, the lines you typed, and, when turned on, the messages to and from the model, at the top of
    // the project's git repository, or in the folder it started in where there is none
    ".aider.chat.history.md",
    ".aider.input.history",
    ".aider.llm.history",
    // Crush: its database of every session's messages, with copies of the files it edited, its logs and the output
    // of long commands, in the nearest of its folders between the one it starts in and the git top, else in a new one
    // where it starts. SQLite cannot open a database hidden as a file, so the whole folder is hidden, and the
    // project's settings and commands for Crush with it.
    ".crush",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub name: String,
    /// The key it was made within: it opens nothing that one does not, and ends with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// The worlds it opens; none means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worlds: Vec<String>,
    /// The folders and files it opens, absolute and resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<PathBuf>,
    /// The variables it passes in, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// The hosts a cell under it may reach, over HTTPS; none means no network.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net: Vec<Host>,
    /// The key whose home folder a cell under it gets, kept between runs: its own name, or that of the key it was
    /// made within. None means an empty home folder each time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    #[serde(default)]
    pub limits: Limits,
    /// What the effects released under it, and under the keys made within it, may cost in total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend: Option<Cents>,
    /// When it was made and when it ends, in nanoseconds since 1970.
    pub made_ns: u64,
    pub until_ns: u64,
}

/// A key as an effect remembers it: its name and when it was made, so that a key made later under the same name is
/// another key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRef {
    pub name: String,
    pub made_ns: u64,
}

/// True while `k` names a key that has not been taken back.
pub fn current(k: &KeyRef) -> bool {
    Key::load(&k.name).is_ok_and(|key| key.made_ns == k.made_ns)
}

/// What `arbor key new` asks for.
#[derive(Debug, Default)]
pub struct Spec {
    pub name: String,
    pub within: Option<String>,
    pub worlds: Vec<String>,
    pub read: Vec<PathBuf>,
    pub toolchains: bool,
    pub env: Vec<String>,
    /// The hosts, as `--net` takes them.
    pub net: Vec<String>,
    /// A home folder of its own. Unset is none, or, within another key, that key's.
    pub home: bool,
    pub limits: Limits,
    pub spend: Option<Cents>,
    /// When it ends. Unset is 8 hours from now, or, within another key, that key's end.
    pub until_ns: Option<u64>,
}

pub fn keys_dir() -> Result<PathBuf> {
    Ok(paths::state_dir()?.join("keys"))
}

/// Where the keys' own home folders are, one folder per key that has one, named after it.
pub fn homes_dir() -> Result<PathBuf> {
    Ok(paths::state_dir()?.join("homes"))
}

/// Makes the home folder `dir`, in `homes`, so that only its owner may enter either. Fails with `AlreadyExists`
/// when `dir` is there.
fn new_home(dir: &Path) -> io::Result<()> {
    let private = |d: &Path| fs::set_permissions(d, fs::Permissions::from_mode(0o700));
    let homes = dir.parent().unwrap_or(dir);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(homes)?;
    private(homes)?;
    fs::DirBuilder::new().mode(0o700).create(dir)?;
    private(dir)
}

/// What the folder `dir` takes on disk: the blocks of all it holds that can be read, without following links. A
/// folder's descriptor closes before its contents are read, so a deep tree needs no more than one at a time.
pub fn disk_usage(dir: &Path) -> u64 {
    let (mut used, mut todo) = (0, vec![dir.to_path_buf()]);
    while let Some(p) = todo.pop() {
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        used += meta.blocks() * 512;
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(&p)
        {
            todo.extend(entries.flatten().map(|e| e.path()));
        }
    }
    used
}

fn file(name: &str) -> Result<PathBuf> {
    Ok(keys_dir()?.join(format!("{name}.json")))
}

/// True while the key named `name` has not been taken back.
pub fn exists(name: &str) -> bool {
    file(name).is_ok_and(|f| f.exists())
}

/// Takes the lock that renewals and take-backs take turns under, until the returned file is dropped: a renewal never
/// brings back a key taken back meanwhile, and two renewals never undo each other.
fn lock() -> Result<File> {
    let dir = keys_dir()?;
    fs::create_dir_all(&dir).ctx(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join("lock");
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

fn check_env_name(name: &str) -> Result<()> {
    let ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::usage(format!("'{name}' is not a variable name")))
    }
}

/// Whether a key may open `path`, absolute and resolved: not the whole computer, not the folders a cell gets fresh
/// copies of, not arbor's own folder with every world's changes, and not the session's sockets.
pub fn check_grant(path: &Path) -> Result<()> {
    let why = if path == Path::new("/") {
        Some("that is the whole computer")
    } else if ["/proc", "/dev", "/sys", "/run"]
        .iter()
        .any(|live| path.starts_with(live))
    {
        Some("a cell gets a fresh one")
    } else if paths::contains(&resolved(paths::state_dir()?), path) {
        Some("it holds every world's changes")
    } else if env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|r| paths::contains(&resolved(PathBuf::from(r)), path))
    {
        Some("it holds the session's sockets")
    } else {
        None
    };
    match why {
        Some(why) => Err(Error::refused(format!(
            "a key cannot open {}: {why}",
            paths::tilde(path)
        ))),
        None => Ok(()),
    }
}

fn resolved(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

/// A path for `--read`, resolved, that a key may open.
fn grant(path: &Path) -> Result<PathBuf> {
    let p = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(Error::usage(format!("there is no {}", path.display())));
        }
        Err(e) => return Err(e).ctx(|| format!("cannot open {}", path.display())),
    };
    check_grant(&p)?;
    Ok(p)
}

/// The toolchain folders that exist in the home folder.
pub fn toolchains() -> Vec<PathBuf> {
    let Some(home) = paths::home() else {
        return Vec::new();
    };
    TOOLCHAINS
        .iter()
        .filter_map(|t| fs::canonicalize(home.join(t)).ok())
        .filter(|p| p.is_dir() && check_grant(p).is_ok())
        .collect()
}

/// The files SQLite keeps beside a database while it is open, which come and go as it runs.
const BESIDE: [&str; 3] = ["-wal", "-shm", "-journal"];

/// What a cell must not see of the password files and agents' records, as `secrets_in` finds them.
#[derive(Debug, Default, PartialEq)]
pub struct Hidden {
    /// Those there when the cell starts, each with whether it is a folder: a cell mounts an empty one over each.
    pub masks: Vec<(PathBuf, bool)>,
    /// Of those missing, the first part of each one's way that is missing, in name order: a cell keeps each missing,
    /// even if a program outside makes it while the cell runs (see `cell`).
    pub missing: Vec<PathBuf>,
}

/// The password files and agents' records that a cell would see: those on `SECRETS` and `RECORDS` in the home folder
/// `home`, and `in_projects`, those on `PROJECT_RECORDS` that a cell found in a project's folder. A cell sees what lies
/// outside its `hidden` folders, and what lies inside `shown`: its world's folder and what its key opens. Each is
/// found where it really is, through links, so one linked from elsewhere counts too, and so does one that is missing
/// from the home folder, where a program would make it. One that is itself among `shown` was opened on purpose and
/// stays.
pub fn secrets_in(
    home: Option<&Path>,
    hidden: &[PathBuf],
    shown: &[PathBuf],
    in_projects: &[PathBuf],
) -> Hidden {
    let seen = |p: &Path| {
        !hidden.iter().any(|h| paths::contains(h, p))
            || shown.iter().any(|s| s != p && paths::contains(s, p))
    };
    let mut found = Hidden::default();
    let in_home = home.into_iter().flat_map(|home| {
        SECRETS
            .iter()
            .chain(&RECORDS)
            .flat_map(move |rel| named(home, rel))
            .filter_map(move |rel| place(home, &rel))
    });
    let in_project = in_projects
        .iter()
        .filter_map(|p| place(Path::new("/"), p))
        .filter(|place| matches!(place, Place::There(_)));
    for place in in_home.chain(in_project) {
        match place {
            Place::There(path) => {
                let named = shown.contains(&path);
                let covered = found.masks.iter().any(|(f, _)| paths::contains(f, &path));
                if seen(&path) && !named && !covered {
                    let dir = path.is_dir();
                    found.masks.push((path, dir));
                }
            }
            Place::Missing(part) if seen(&part) => found.missing.push(part),
            Place::Missing(_) => {}
        }
    }
    let masks = &found.masks;
    found
        .missing
        .retain(|p| !masks.iter().any(|(m, _)| paths::contains(m, p)));
    found.missing.sort();
    found.missing.dedup();
    found
}

/// The records on `PROJECT_RECORDS` there are in the folder `dir`, and, with `inside`, in each folder right inside it,
/// folder by folder in name order. A link to a folder is not followed there, but a record that is a link counts.
pub fn in_project(dir: &Path, inside: bool) -> Vec<PathBuf> {
    let mut folders = vec![dir.to_path_buf()];
    if inside {
        let mut more: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .map(|entry| entry.path())
            .collect();
        more.sort();
        folders.extend(more);
    }
    let names: Vec<CString> = PROJECT_RECORDS
        .iter()
        .map(|rel| CString::new(*rel).unwrap_or_default())
        .collect();
    folders
        .iter()
        .flat_map(|folder| {
            let there = sys::present(folder, &names);
            there.into_iter().map(|i| folder.join(PROJECT_RECORDS[i]))
        })
        .collect()
}

/// What the list entry `rel` names in the home folder `home`, relative to it: that path, or, when its last part has a
/// `*`, each file or folder in that folder whose name matches, in name order, with the files SQLite would keep beside
/// each file among them. Where that folder is missing, the entry stands for what may be made there.
fn named(home: &Path, rel: &str) -> Vec<PathBuf> {
    let Some((dir, pattern)) = rel.rsplit_once('/').filter(|(_, name)| name.contains('*')) else {
        return vec![rel.into()];
    };
    let entries = match fs::read_dir(home.join(dir)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return vec![rel.into()],
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !matches(pattern, &name) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| !t.is_dir()) && !BESIDE.iter().any(|b| name.ends_with(b))
        {
            let beside = BESIDE.iter().map(|b| format!("{name}{b}"));
            names.extend(beside.filter(|b| matches(pattern, b)));
        }
        names.push(name);
    }
    names.sort();
    names.dedup();
    names.iter().map(|name| Path::new(dir).join(name)).collect()
}

/// Where a listed path is.
#[derive(Debug, PartialEq)]
enum Place {
    /// There, at this path, resolved.
    There(PathBuf),
    /// Missing: this is the first part of its way that is, at its real place.
    Missing(PathBuf),
}

/// Where `rel`, a path in the home folder `home`, resolved, leads, following links as a program would. None where it
/// leads nowhere: past a file, round links, or into a folder arbor may not read.
fn place(home: &Path, rel: &Path) -> Option<Place> {
    let mut at = home.to_path_buf();
    // The parts still to follow, the next one last.
    let mut todo: Vec<OsString> = Vec::new();
    follow(&mut todo, &mut at, rel);
    let mut links = 0;
    while let Some(part) = todo.pop() {
        if part == ".." {
            at.pop();
            continue;
        }
        let next = at.join(&part);
        match fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                links += 1;
                if links > 40 {
                    return None;
                }
                follow(&mut todo, &mut at, &fs::read_link(&next).ok()?);
            }
            Ok(meta) if meta.is_dir() || todo.is_empty() => at = next,
            Ok(_) => return None,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Some(Place::Missing(next)),
            Err(_) => return None,
        }
    }
    Some(Place::There(at))
}

/// Puts the parts of `path` on `todo`, to be followed from `at` before the rest, or from `/` when it is absolute.
fn follow(todo: &mut Vec<OsString>, at: &mut PathBuf, path: &Path) {
    if path.has_root() {
        *at = PathBuf::from("/");
    }
    let start = todo.len();
    todo.extend(path.components().filter_map(|part| match part {
        Component::Normal(name) => Some(name.to_os_string()),
        Component::ParentDir => Some("..".into()),
        _ => None,
    }));
    todo[start..].reverse();
}

/// Whether `name` matches `pattern`, in which each `*` stands for any run of characters.
fn matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(mut rest) = name.strip_prefix(parts.next().unwrap_or_default()) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for part in parts {
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    rest.ends_with(last)
}

fn local(ts_ns: u64) -> Option<libc::tm> {
    let secs = (ts_ns / 1_000_000_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (!unsafe { libc::localtime_r(&secs, &mut tm) }.is_null()).then_some(tm)
}

/// A clock time for people, never a countdown: `18:00` today, else `10-18 09:00`.
pub fn clock(ts_ns: u64, now_ns: u64) -> String {
    match (local(ts_ns), local(now_ns)) {
        (Some(t), Some(n)) if (t.tm_year, t.tm_yday) == (n.tm_year, n.tm_yday) => {
            format!("{:02}:{:02}", t.tm_hour, t.tm_min)
        }
        (Some(t), _) => format!(
            "{:02}-{:02} {:02}:{:02}",
            t.tm_mon + 1,
            t.tm_mday,
            t.tm_hour,
            t.tm_min
        ),
        _ => "?".into(),
    }
}

/// The next time the local clock reads `HH:MM`: today, or tomorrow once that has passed.
pub fn parse_until(text: &str, now_ns: u64) -> Result<u64> {
    let bad = || {
        Error::usage(format!(
            "'{text}' is not a time of day: use for example 18:00"
        ))
    };
    let (h, m) = text.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (i32, i32) = match (h.parse(), m.parse()) {
        (Ok(h), Ok(m)) if (0..24).contains(&h) && (0..60).contains(&m) && m_ok(text) => (h, m),
        _ => return Err(bad()),
    };
    let now = (now_ns / 1_000_000_000) as libc::time_t;
    let at = |days: i32| -> Option<libc::time_t> {
        let mut tm = local(now_ns)?;
        tm.tm_mday += days;
        (tm.tm_hour, tm.tm_min, tm.tm_sec, tm.tm_isdst) = (h, m, 0, -1);
        let t = unsafe { libc::mktime(&mut tm) };
        (t != -1).then_some(t)
    };
    let t = match at(0) {
        Some(t) if t > now => t,
        _ => at(1).ok_or_else(bad)?,
    };
    Ok(t as u64 * 1_000_000_000)
}

/// Minutes are two digits, as clocks show them.
fn m_ok(text: &str) -> bool {
    text.trim().rsplit(':').next().is_some_and(|m| m.len() == 2)
}

impl Key {
    pub fn load(name: &str) -> Result<Key> {
        world::check_name_as(name, "key")?;
        let text = match fs::read_to_string(file(name)?) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::usage(format!(
                    "there is no key named '{name}' (see `arbor keys`)"
                )));
            }
            Err(e) => return Err(e).ctx(|| format!("cannot read key '{name}'")),
        };
        serde_json::from_str(&text).ctx(|| format!("key '{name}' is damaged"))
    }

    /// Every key, sorted by name. Files that cannot be read are skipped.
    pub fn list() -> Result<Vec<Key>> {
        let dir = keys_dir()?;
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx(|| format!("cannot read {}", dir.display())),
        };
        let mut keys = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if let Some(name) = name.to_str().and_then(|n| n.strip_suffix(".json"))
                && let Ok(key) = Key::load(name)
            {
                keys.push(key);
            }
        }
        keys.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(keys)
    }

    /// Makes and saves a key. One made within another must lie within it, and inherits what it leaves out.
    pub fn create(spec: Spec) -> Result<Key> {
        world::check_name_as(&spec.name, "key")?;
        for w in &spec.worlds {
            world::check_name(w)?;
        }
        for v in &spec.env {
            check_env_name(v)?;
        }
        let now = world::now_ns();
        let mut read = spec
            .read
            .iter()
            .map(|p| grant(p))
            .collect::<Result<Vec<_>>>()?;
        if spec.toolchains {
            read.extend(toolchains());
        }
        let mut net = spec
            .net
            .iter()
            .map(|h| Host::parse(h))
            .collect::<Result<Vec<_>>>()?;
        net.sort_by_key(Host::to_string);
        net.dedup();
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v.dedup();
            v
        };
        read.sort();
        read.dedup();
        let mut key = Key {
            name: spec.name.clone(),
            within: spec.within.clone(),
            worlds: sorted(spec.worlds),
            read,
            env: sorted(spec.env),
            net,
            home: spec.home.then(|| spec.name.clone()),
            limits: spec.limits,
            spend: spec.spend,
            made_ns: now,
            until_ns: spec
                .until_ns
                .unwrap_or(now + DEFAULT_LIFE.as_nanos() as u64),
        };
        if let Some(outer) = &spec.within {
            let outer = Key::load(outer)?;
            if outer.ended(now) {
                return Err(outer.ended_refusal(now));
            }
            let named_reads = !spec.read.is_empty() || spec.toolchains;
            key.narrow(&outer, named_reads, spec.until_ns.is_some(), now)?;
        }
        if key.until_ns <= now {
            return Err(Error::usage("a key must end later than now"));
        }
        key.check_home(paths::resolved_home().as_deref())?;
        // A new key's home starts empty: one left by an earlier key of the same name is neither handed on nor
        // deleted.
        let home = homes_dir()?.join(&key.name);
        let left = |home: &Path| {
            Error::refused(format!(
                "{} is left from an earlier key named '{}': move it away or remove it, then make the key again",
                paths::tilde(home),
                key.name
            ))
        };
        if key.owns_home() && !exists(&key.name) && fs::symlink_metadata(&home).is_ok() {
            return Err(left(&home));
        }
        key.save(true)?;
        if key.owns_home()
            && let Err(e) = new_home(&home)
        {
            let _ = fs::remove_file(file(&key.name)?);
            if e.kind() == io::ErrorKind::AlreadyExists {
                return Err(left(&home));
            }
            return Err(e).ctx(|| format!("cannot create {}", home.display()));
        }
        Ok(key)
    }

    /// Refuses a key with a home folder that also opens `home`, the home folder, or a folder that holds it: in a
    /// cell, the key's home takes its place.
    pub fn check_home(&self, home: Option<&Path>) -> Result<()> {
        if let (Some(_), Some(home)) = (&self.home, home)
            && let Some(p) = self.read.iter().find(|p| paths::contains(p, home))
        {
            return Err(Error::refused(format!(
                "key '{}' has a home folder, so it cannot also open {}: in a cell, the key's home takes the place of {}",
                self.name,
                paths::tilde(p),
                paths::tilde(home)
            )));
        }
        Ok(())
    }

    /// Fits this key within `outer`: what it names must lie within `outer`'s, and what it leaves out is `outer`'s.
    fn narrow(&mut self, outer: &Key, named_reads: bool, named_end: bool, now: u64) -> Result<()> {
        let refuse = |what: String| {
            Error::refused(format!(
                "key '{}' would open more than key '{}', which {what}",
                self.name, outer.name
            ))
        };
        if self.worlds.is_empty() {
            self.worlds = outer.worlds.clone();
        } else if !outer.worlds.is_empty()
            && let Some(w) = self.worlds.iter().find(|w| !outer.worlds.contains(w))
        {
            return Err(refuse(format!("does not open world '{w}'")));
        }
        if !named_reads {
            self.read = outer.read.clone();
        } else if let Some(p) = self
            .read
            .iter()
            .find(|p| !outer.read.iter().any(|o| paths::contains(o, p)))
        {
            return Err(refuse(format!("does not open {}", paths::tilde(p))));
        }
        if self.env.is_empty() {
            self.env = outer.env.clone();
        } else if let Some(v) = self.env.iter().find(|v| !outer.env.contains(v)) {
            return Err(refuse(format!("does not pass {v}")));
        }
        if self.net.is_empty() {
            self.net = outer.net.clone();
        } else if let Some(h) = self
            .net
            .iter()
            .find(|h| !outer.net.iter().any(|o| o.covers(h)))
        {
            return Err(refuse(format!("does not open {h}")));
        }
        // Without a home of its own, it shares the outer key's, if any; a home of its own needs an outer key that has
        // one, since one kept between runs is more than a key without it opens.
        match (&self.home, &outer.home) {
            (None, home) => self.home = home.clone(),
            (Some(_), None) => return Err(refuse("has no home folder".into())),
            (Some(_), Some(_)) => {}
        }
        self.limits = self
            .limits
            .within(outer.limits)
            .map_err(|what| refuse(format!("allows less {what}")))?;
        match (self.spend, outer.spend) {
            (Some(spend), Some(most)) if spend > most => {
                return Err(refuse(format!("may spend {}", outbox::money(most))));
            }
            (None, most) => self.spend = most,
            _ => {}
        }
        if !named_end {
            self.until_ns = outer.until_ns;
        } else if self.until_ns > outer.until_ns {
            return Err(refuse(format!("ends at {}", clock(outer.until_ns, now))));
        }
        Ok(())
    }

    /// When this key ends once renewed: at `asked`, else 8 hours from `now`, or, for a key made within `outer`, when
    /// that one ends. The end only moves later, and no later than `outer`'s, which must not have ended: so a command
    /// running under the key never has to stop sooner than it was told, and keys made within it stay within it.
    fn renewal(&self, outer: Option<&Key>, asked: Option<u64>, now: u64) -> Result<u64> {
        let end = match (asked, outer) {
            (Some(at), _) => at,
            (None, Some(outer)) => outer.until_ns,
            (None, None) => now.saturating_add(DEFAULT_LIFE.as_nanos() as u64),
        };
        if let Some(outer) = outer {
            let at = clock(outer.until_ns, now);
            if outer.ended(now) {
                return Err(Error::refused(format!(
                    "key '{}' was made within key '{}', which ended at {at}: renew that one first",
                    self.name, outer.name
                )));
            }
            if end > outer.until_ns {
                return Err(Error::refused(format!(
                    "key '{}' would open more than key '{}', which ends at {at}",
                    self.name, outer.name
                )));
            }
        }
        if end <= now {
            return Err(Error::usage("a key must end later than now"));
        }
        if end <= self.until_ns {
            let why = match outer {
                Some(outer) if asked.is_none() => format!(
                    ", as late as key '{}' it was made within: renew that one first",
                    outer.name
                ),
                _ => format!(
                    "; renewing moves an end only later, and `arbor key take-back {}` ends it now",
                    self.name
                ),
            };
            return Err(Error::refused(format!(
                "key '{}' already lasts until {}{why}",
                self.name,
                clock(self.until_ns, now)
            )));
        }
        Ok(end)
    }

    /// Writes the key's file, mode 600: a new one, refusing a name that is taken, or, with `new` false, in place of
    /// its own.
    fn save(&self, new: bool) -> Result<()> {
        let dir = keys_dir()?;
        fs::create_dir_all(&dir).ctx(|| format!("cannot create {}", dir.display()))?;
        let tmp = dir.join(format!(".{}.{}.tmp", self.name, std::process::id()));
        let path = file(&self.name)?;
        let written = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(
                serde_json::to_string_pretty(self)
                    .map_err(io::Error::other)?
                    .as_bytes(),
            )?;
            f.write_all(b"\n")?;
            f.sync_all()?;
            if new {
                sys::rename_noreplace(&tmp, &path)
            } else {
                fs::rename(&tmp, &path)
            }
        })();
        match written {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                if e.kind() == io::ErrorKind::AlreadyExists {
                    Err(Error::refused(format!(
                        "a key named '{}' already exists",
                        self.name
                    )))
                } else {
                    Err(e).ctx(|| format!("cannot write {}", path.display()))
                }
            }
        }
    }

    pub fn ended(&self, now_ns: u64) -> bool {
        now_ns >= self.until_ns
    }

    /// Why an ended key opens nothing, and how to renew it.
    fn ended_refusal(&self, now_ns: u64) -> Error {
        Error::refused(format!(
            "key '{}' ended at {}; `arbor key renew {}` moves its end later",
            self.name,
            clock(self.until_ns, now_ns),
            self.name
        ))
    }

    /// When it ends, read again: a renewal may have moved its end later since this copy was read. Taken back, or
    /// made again under its name, it keeps the end this copy has.
    pub fn ends_at(&self) -> u64 {
        Key::load(&self.name)
            .ok()
            .filter(|k| k.made_ns == self.made_ns)
            .map_or(self.until_ns, |k| k.until_ns)
    }

    pub fn id(&self) -> KeyRef {
        KeyRef {
            name: self.name.clone(),
            made_ns: self.made_ns,
        }
    }

    /// This key, then each key it was made within, outwards.
    pub fn chain(&self) -> Result<Vec<KeyRef>> {
        let mut chain = vec![self.id()];
        let mut within = self.within.clone();
        while let Some(name) = within {
            if chain.len() > 64 || chain.iter().any(|k| k.name == name) {
                return Err(Error::new(format!(
                    "key '{}' is made within itself",
                    self.name
                )));
            }
            let outer = Key::load(&name)?;
            chain.push(outer.id());
            within = outer.within;
        }
        Ok(chain)
    }

    pub fn opens(&self, world: &str) -> bool {
        self.worlds.is_empty() || self.worlds.iter().any(|w| w == world)
    }

    /// Refuses a key that has ended or that does not open `world`.
    pub fn check(&self, world: &str) -> Result<()> {
        let now = world::now_ns();
        if self.ended(now) {
            return Err(self.ended_refusal(now));
        }
        if !self.opens(world) {
            return Err(Error::refused(format!(
                "key '{}' does not open world '{world}'; it opens {}",
                self.name,
                self.worlds.join(", ")
            )));
        }
        Ok(())
    }

    /// What it opens: `any world` or `shop, docs`.
    pub fn opened(&self) -> String {
        if self.worlds.is_empty() {
            "any world".into()
        } else {
            self.worlds.join(", ")
        }
    }

    /// `~/.cargo, ~/.rustup`.
    pub fn reads(&self) -> String {
        self.read
            .iter()
            .map(|p| paths::tilde(p))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `api.example.com, *.example.org`.
    pub fn hosts(&self) -> String {
        self.net
            .iter()
            .map(Host::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The folder a cell under it gets as its home folder, kept between runs, if it has one.
    pub fn home_dir(&self) -> Result<Option<PathBuf>> {
        let Some(owner) = &self.home else {
            return Ok(None);
        };
        world::check_name_as(owner, "key")?;
        Ok(Some(homes_dir()?.join(owner)))
    }

    /// Whether the home folder it gives is its own, not that of the key it was made within.
    pub fn owns_home(&self) -> bool {
        self.home.as_ref() == Some(&self.name)
    }

    /// Makes its home folder again, empty, when it is gone.
    pub fn ensure_home(&self) -> Result<()> {
        if let Some(dir) = self.home_dir()?
            && let Err(e) = new_home(&dir)
            && e.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(e).ctx(|| format!("cannot create {}", dir.display()));
        }
        Ok(())
    }

    /// Removes its own home folder, once nothing runs under it any more. True when there was one.
    pub fn remove_home(&self) -> Result<bool> {
        let dir = match self.home_dir()? {
            Some(dir) if self.owns_home() && fs::symlink_metadata(&dir).is_ok() => dir,
            _ => return Ok(false),
        };
        world::remove_tree(&dir).ctx(|| {
            format!(
                "cannot remove the home folder of key '{}', {}",
                self.name,
                paths::tilde(&dir)
            )
        })?;
        Ok(true)
    }

    /// `opens shop; reads ~/.cargo; env CARGO_HOME; net api.example.com; home ~/.local/share/arbor/homes/dev; mem
    /// 4GiB; spend $5.00; within dev; until 18:00`, for the ledger.
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("opens {}", self.opened())];
        if !self.read.is_empty() {
            parts.push(format!("reads {}", self.reads()));
        }
        if !self.env.is_empty() {
            parts.push(format!("env {}", self.env.join(", ")));
        }
        if !self.net.is_empty() {
            parts.push(format!("net {}", self.hosts()));
        }
        if let Ok(Some(dir)) = self.home_dir() {
            parts.push(format!("home {}", paths::tilde(&dir)));
        }
        let limits = self.limits.describe();
        if !limits.is_empty() {
            parts.push(limits);
        }
        if let Some(spend) = self.spend {
            parts.push(format!("spend {}", outbox::money(spend)));
        }
        if let Some(w) = &self.within {
            parts.push(format!("within {w}"));
        }
        parts.push(format!("until {}", clock(self.until_ns, world::now_ns())));
        parts.join("; ")
    }
}

/// Moves the end of the key `name` later (see `Key::renewal`). Returns the key, and the end it had. Nothing else about
/// it changes, nor about the keys made within it.
pub fn renew(name: &str, asked: Option<u64>) -> Result<(Key, u64)> {
    let _lock = lock()?;
    let mut key = Key::load(name)?;
    let outer = key.within.as_deref().map(Key::load).transpose()?;
    let was = key.until_ns;
    key.until_ns = key.renewal(outer.as_ref(), asked, world::now_ns())?;
    key.save(false)?;
    Ok((key, was))
}

/// Takes back the key `name` and every key made within it, at any depth. Returns them, `name` first. Their home
/// folders stay until the caller has stopped what runs under them (see `Key::remove_home`).
pub fn take_back(name: &str) -> Result<Vec<Key>> {
    let _lock = lock()?;
    let first = Key::load(name)?;
    let all = Key::list()?;
    let mut gone = vec![first];
    loop {
        let more: Vec<Key> = all
            .iter()
            .filter(|k| {
                let taken = |n: &String| gone.iter().any(|g| &g.name == n);
                !taken(&k.name) && k.within.as_ref().is_some_and(taken)
            })
            .cloned()
            .collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    for k in &gone {
        match fs::remove_file(file(&k.name)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(e).ctx(|| format!("cannot remove key '{}'", k.name));
            }
            _ => {}
        }
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Key {
        Key {
            name: name.into(),
            within: None,
            worlds: Vec::new(),
            read: Vec::new(),
            env: Vec::new(),
            net: Vec::new(),
            home: None,
            limits: Limits::default(),
            spend: None,
            made_ns: 0,
            until_ns: 10_000,
        }
    }

    #[test]
    fn a_key_within_another_opens_no_more() {
        let mut outer = key("dev");
        outer.worlds = vec!["shop".into()];
        outer.read = vec![PathBuf::from("/home/u/.cargo")];
        outer.env = vec!["CARGO_HOME".into()];
        outer.limits.mem = Some(1 << 30);
        outer.spend = Some(500);

        // What it leaves out, it inherits.
        let mut k = key("tests");
        k.narrow(&outer, false, false, 0).unwrap();
        assert_eq!(
            (
                k.worlds.clone(),
                k.read.clone(),
                k.env.clone(),
                k.limits,
                k.spend,
                k.until_ns
            ),
            (
                outer.worlds.clone(),
                outer.read.clone(),
                outer.env.clone(),
                outer.limits,
                Some(500),
                10_000
            )
        );

        // What it names must lie within.
        let mut k = key("tests");
        k.read = vec![PathBuf::from("/home/u/.cargo/bin")];
        k.limits.mem = Some(512 << 20);
        k.spend = Some(200);
        k.until_ns = 5_000;
        k.narrow(&outer, true, true, 0).unwrap();
        assert_eq!(
            (k.limits.mem, k.spend, k.until_ns),
            (Some(512 << 20), Some(200), 5_000)
        );

        let wider = |change: &dyn Fn(&mut Key)| {
            let mut k = key("tests");
            change(&mut k);
            let named_end = k.until_ns != 10_000;
            k.narrow(&outer, !k.read.is_empty(), named_end, 0)
                .unwrap_err()
                .msg
        };
        assert!(wider(&|k| k.worlds = vec!["bank".into()]).contains("does not open world 'bank'"));
        assert!(
            wider(&|k| k.read = vec![PathBuf::from("/home/u/.ssh")])
                .contains("does not open /home/u/.ssh")
        );
        assert!(wider(&|k| k.env = vec!["TOKEN".into()]).contains("does not pass TOKEN"));
        assert!(wider(&|k| k.limits.mem = Some(2 << 30)).contains("allows less memory"));
        assert!(wider(&|k| k.spend = Some(501)).contains("may spend $5.00"));
        assert!(wider(&|k| k.until_ns = 20_000).contains("ends at"));
    }

    #[test]
    fn a_key_within_another_shares_its_home_or_has_its_own_only_where_that_one_has_one() {
        let mut outer = key("agent");
        // Without a home of its own, it shares the outer key's...
        outer.home = Some("agent".into());
        let mut k = key("short");
        k.narrow(&outer, false, false, 0).unwrap();
        assert_eq!(k.home.as_deref(), Some("agent"));
        assert!(!k.owns_home() && outer.owns_home());
        // ...and with one, it has its own.
        let mut k = key("fresh");
        k.home = Some("fresh".into());
        k.narrow(&outer, false, false, 0).unwrap();
        assert!(k.owns_home());
        // A key without one gives none, and a home of its own would open more.
        outer.home = None;
        let mut k = key("short");
        k.narrow(&outer, false, false, 0).unwrap();
        assert_eq!(k.home, None);
        k.home = Some("short".into());
        let err = k.narrow(&outer, false, false, 0).unwrap_err().msg;
        assert!(
            err.contains("would open more than key 'agent', which has no home folder"),
            "{err}"
        );
    }

    #[test]
    fn a_key_with_a_home_cannot_also_open_the_home_folder() {
        let home = Path::new("/home/u");
        let mut k = key("agent");
        k.read = vec![PathBuf::from("/home/u/.cargo"), PathBuf::from("/usr/share")];
        k.home = Some("agent".into());
        assert!(k.check_home(Some(home)).is_ok());
        for holds in ["/home/u", "/home"] {
            k.read = vec![PathBuf::from(holds)];
            let err = k.check_home(Some(home)).unwrap_err();
            assert_eq!(err.code, crate::error::REFUSED, "{holds}");
            assert!(
                err.msg.contains("the key's home takes the place of"),
                "{}",
                err.msg
            );
        }
        // Without a home, it may.
        k.home = None;
        assert!(k.check_home(Some(home)).is_ok());
        // The ledger names the folder it gives, its own or the outer key's.
        k.read.clear();
        assert!(!k.describe().contains("home"));
        k.home = Some("outer".into());
        let said = k.describe();
        assert!(
            said.contains("; home ") && said.contains("homes/outer;"),
            "{said}"
        );
        // A damaged key file cannot name a folder elsewhere as its home.
        k.home = Some("../../elsewhere".into());
        assert!(k.home_dir().is_err());
    }

    #[test]
    fn disk_usage_counts_files_without_following_links() {
        let root = std::env::temp_dir().join(format!("arbor-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/deep/er")).unwrap();
        // Bytes that do not compress, so a filesystem that compresses stores them whole.
        let noise = |n: usize| {
            let mut x = 0x2545_f491_4f6c_dd1du64;
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect::<Vec<u8>>()
        };
        fs::write(root.join("home/deep/er/file"), noise(64 * 1024)).unwrap();
        fs::write(root.join("big"), noise(1 << 20)).unwrap();
        std::os::unix::fs::symlink(root.join("big"), root.join("home/link")).unwrap();
        let used = disk_usage(&root.join("home"));
        assert!((64 * 1024..1 << 20).contains(&used), "{used}");
        assert_eq!(disk_usage(&root.join("none")), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn opens_its_worlds_until_it_ends() {
        let mut k = key("dev");
        assert!(k.opens("anything") && !k.ended(9_999) && k.ended(10_000));
        k.worlds = vec!["shop".into()];
        assert!(k.opens("shop") && !k.opens("bank"));
        assert_eq!(k.opened(), "shop");
        // Once ended, it says how to renew it.
        let err = k.check("shop").unwrap_err();
        assert!(
            err.msg
                .contains("`arbor key renew dev` moves its end later"),
            "{}",
            err.msg
        );
    }

    #[test]
    fn a_renewal_moves_an_end_only_later_and_never_past_the_key_it_was_made_within() {
        use crate::error::{REFUSED, USAGE};
        let hour = 3600 * 1_000_000_000;
        let now = 100 * hour;
        let refused = |r: Result<u64>, said: &[&str]| {
            let err = r.unwrap_err();
            assert_eq!(err.code, REFUSED, "{}", err.msg);
            for s in said {
                assert!(err.msg.contains(s), "{s} in {}", err.msg);
            }
        };
        // Ended or not, without an end it lasts 8 hours from now; with one, until then.
        let mut k = key("agent");
        k.until_ns = now - hour;
        assert_eq!(k.renewal(None, None, now).unwrap(), now + 8 * hour);
        assert_eq!(
            k.renewal(None, Some(now + 2 * hour), now).unwrap(),
            now + 2 * hour
        );
        // Never sooner than it ends now, nor than now.
        k.until_ns = now + 3 * hour;
        refused(
            k.renewal(None, Some(now + 2 * hour), now),
            &[
                "already lasts until",
                "`arbor key take-back agent` ends it now",
            ],
        );
        refused(k.renewal(None, Some(now + 3 * hour), now), &[]);
        let err = k.renewal(None, Some(now), now).unwrap_err();
        assert_eq!(err.code, USAGE, "{}", err.msg);

        // Within another key, up to that one's end, which is also where it goes without one.
        let mut outer = key("outer");
        outer.until_ns = now + 5 * hour;
        let mut inner = key("inner");
        inner.within = Some("outer".into());
        inner.until_ns = now - hour;
        assert_eq!(
            inner.renewal(Some(&outer), None, now).unwrap(),
            now + 5 * hour
        );
        assert_eq!(
            inner
                .renewal(Some(&outer), Some(now + 4 * hour), now)
                .unwrap(),
            now + 4 * hour
        );
        refused(
            inner.renewal(Some(&outer), Some(now + 6 * hour), now),
            &["key 'inner' would open more than key 'outer', which ends at"],
        );
        // One that lasts as long as it already waits for that one to be renewed...
        inner.until_ns = outer.until_ns;
        refused(
            inner.renewal(Some(&outer), None, now),
            &["as late as key 'outer' it was made within: renew that one first"],
        );
        // ...as does any while that one has ended.
        outer.until_ns = now - hour;
        inner.until_ns = now - 2 * hour;
        refused(
            inner.renewal(Some(&outer), Some(now + hour), now),
            &[
                "was made within key 'outer', which ended at",
                "renew that one first",
            ],
        );
        refused(
            inner.renewal(Some(&outer), None, now),
            &["renew that one first"],
        );
    }

    #[test]
    fn grants_stay_out_of_the_computer_s_live_folders() {
        for bad in ["/", "/proc/self", "/dev", "/sys/fs", "/run/user/1000"] {
            assert!(check_grant(Path::new(bad)).is_err(), "{bad}");
        }
        assert!(check_grant(Path::new("/usr/share")).is_ok());
        assert!(check_env_name("CARGO_HOME").is_ok() && check_env_name("_x1").is_ok());
        for bad in ["", "1X", "A-B", "A=B"] {
            assert!(check_env_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hides_the_password_files_a_cell_would_see_unless_opened_by_name() {
        let root = std::env::temp_dir().join(format!("arbor-secrets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.ssh")).unwrap();
        fs::create_dir_all(root.join("home/.cargo")).unwrap();
        fs::create_dir_all(root.join("data/gnupg")).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let home = root.join("home");
        fs::write(home.join(".cargo/credentials.toml"), "").unwrap();
        fs::write(home.join(".npmrc"), "").unwrap();
        // Linked from outside the home folder, where nothing hides it.
        std::os::unix::fs::symlink(root.join("data/gnupg"), home.join(".gnupg")).unwrap();
        let (gnupg, ssh) = ((root.join("data/gnupg"), true), (home.join(".ssh"), true));
        let npmrc = (home.join(".npmrc"), false);
        let cargo = (home.join(".cargo/credentials.toml"), false);
        let found = |shown: &[PathBuf]| {
            secrets_in(Some(&home), std::slice::from_ref(&home), shown, &[]).masks
        };

        assert_eq!(found(&[]), vec![gnupg.clone()]);
        assert_eq!(
            found(std::slice::from_ref(&home)),
            vec![ssh, gnupg.clone(), npmrc.clone(), cargo.clone()]
        );
        assert_eq!(
            found(&[home.join(".cargo")]),
            vec![gnupg.clone(), cargo.clone()]
        );
        assert_eq!(
            found(&[home.clone(), home.join(".ssh")]),
            vec![gnupg, npmrc, cargo]
        );
        assert!(Key::load("Bad Name").is_err_and(|e| e.code == crate::error::USAGE));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hides_agents_records_too_and_finds_databases_whatever_their_version() {
        let root = std::env::temp_dir().join(format!("arbor-records-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.claude/projects")).unwrap();
        fs::create_dir_all(root.join("home/.codex")).unwrap();
        let home = fs::canonicalize(root.join("home")).unwrap();
        let codex = home.join(".codex");
        for name in [
            "state_5.sqlite",
            "state_5.sqlite-wal",
            "logs_9.sqlite",
            "config.toml",
        ] {
            fs::write(codex.join(name), "").unwrap();
        }
        let found = |shown: &[PathBuf]| {
            secrets_in(Some(&home), std::slice::from_ref(&home), shown, &[]).masks
        };
        let db = |name: &str| (codex.join(name), false);

        // The databases come in name order, and the settings next to them stay.
        assert_eq!(
            found(&[home.join(".claude"), codex.clone()]),
            vec![
                (home.join(".claude/projects"), true),
                db("logs_9.sqlite"),
                db("state_5.sqlite"),
                db("state_5.sqlite-wal")
            ]
        );
        // One that a key names shows.
        assert_eq!(
            found(&[codex.clone(), codex.join("state_5.sqlite")]),
            vec![db("logs_9.sqlite"), db("state_5.sqlite-wal")]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_agent_s_records_stay_hidden_from_a_key_that_opens_its_hidden_folder() {
        let root = std::env::temp_dir().join(format!("arbor-gemini-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            "tmp/shop/chats",
            "history/shop",
            "cli-browser-profile",
            "commands",
        ] {
            fs::create_dir_all(root.join("home/.gemini").join(dir)).unwrap();
        }
        let home = fs::canonicalize(root.join("home")).unwrap();
        let gemini = home.join(".gemini");
        let found = |shown: &[PathBuf]| {
            secrets_in(Some(&home), std::slice::from_ref(&home), shown, &[]).masks
        };

        // Inside the home folder, the folder hidden whole covers what is in it...
        assert_eq!(
            found(std::slice::from_ref(&home)),
            vec![(gemini.clone(), true)]
        );
        // ...and a key that opens it by name opens its settings, but not its records or its browser profile.
        assert_eq!(
            found(std::slice::from_ref(&gemini)),
            vec![
                (gemini.join("cli-browser-profile"), true),
                (gemini.join("tmp"), true),
                (gemini.join("history"), true)
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_one_is_found_where_a_program_would_make_it() {
        let root = std::env::temp_dir().join(format!("arbor-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            "home/.config",
            "home/.claude",
            "home/.codex",
            "home/.gemini",
            "home/dots",
            "data",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let root = fs::canonicalize(&root).unwrap();
        let (home, data) = (root.join("home"), root.join("data"));
        for file in [
            ".codex/state_5.sqlite",
            ".codex/state_5.sqlite-wal",
            ".cline",
        ] {
            fs::write(home.join(file), "").unwrap();
        }
        // Linked to a file not made yet outside the home folder, and to a folder not made yet inside it.
        std::os::unix::fs::symlink(data.join("claude.json"), home.join(".claude.json")).unwrap();
        std::os::unix::fs::symlink("dots/aws", home.join(".aws")).unwrap();
        let found =
            |shown: &[PathBuf]| secrets_in(Some(&home), std::slice::from_ref(&home), shown, &[]);
        let all = found(std::slice::from_ref(&home));
        let missing = |rel: &str| all.missing.contains(&home.join(rel));

        // The first part of each one's way that is missing: a file, a folder in a folder that is there, a folder
        // whose files are listed, once for all of them, and the files SQLite keeps beside a database that is there.
        for rel in [
            ".netrc",
            ".ssh",
            ".config/gh",
            ".claude/.credentials.json",
            ".claude/projects",
            ".kimi-code",
            ".copilot",
            ".codex/state_5.sqlite-shm",
            ".codex/state_5.sqlite-journal",
            // Through links, where they lead
            "dots/aws",
        ] {
            assert!(missing(rel), "{rel}");
        }
        assert!(all.missing.contains(&data.join("claude.json")));
        // Not what is there, nor the way to it, nor past a file, nor inside a folder hidden whole.
        for rel in [
            ".config",
            ".kimi-code/credentials",
            ".aws",
            ".claude.json",
            ".codex/state_5.sqlite-wal",
            ".codex/state_5.sqlite-wal-wal",
            ".cline/data",
            ".gemini/tmp",
        ] {
            assert!(!missing(rel), "{rel}");
        }
        assert_eq!(
            all.missing
                .iter()
                .filter(|p| p.ends_with(".kimi-code"))
                .count(),
            1
        );
        assert!(
            all.masks
                .contains(&(home.join(".codex/state_5.sqlite-wal"), false))
        );
        // A cell that sees only the computer's own folders sees only the one linked out...
        assert_eq!(found(&[]).missing, [data.join("claude.json")]);
        // ...and one under a key that opens a folder hidden whole by name sees what may be made in it among the rest.
        assert!(
            found(&[home.clone(), home.join(".gemini")])
                .missing
                .contains(&home.join(".gemini/tmp"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_agents_records_at_the_top_of_a_project_and_in_each_folder_right_inside_it() {
        let root = std::env::temp_dir().join(format!("arbor-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["work/shop/.crush/commands", "work/notes/deep", "elsewhere"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let root = fs::canonicalize(&root).unwrap();
        let (work, elsewhere) = (root.join("work"), root.join("elsewhere"));
        for file in [
            "work/.aider.chat.history.md",
            "work/shop/.aider.input.history",
            "work/shop/README",
            "work/notes/deep/.aider.llm.history",
            "elsewhere/chat.md",
            "elsewhere/.aider.input.history",
        ] {
            fs::write(root.join(file), "").unwrap();
        }
        // A record linked to a file elsewhere, one linked to a file not made yet, and a link to a folder.
        let link = std::os::unix::fs::symlink;
        link(
            elsewhere.join("chat.md"),
            work.join("shop/.aider.chat.history.md"),
        )
        .unwrap();
        link(
            elsewhere.join("gone.md"),
            work.join("notes/.aider.chat.history.md"),
        )
        .unwrap();
        link(elsewhere.clone(), work.join("linked")).unwrap();

        // At the top only, or there and one level down, but no deeper, nor through a link to a folder.
        assert_eq!(
            in_project(&work, false),
            [work.join(".aider.chat.history.md")]
        );
        let found = in_project(&work, true);
        assert_eq!(
            found,
            [
                work.join(".aider.chat.history.md"),
                work.join("notes/.aider.chat.history.md"),
                work.join("shop/.aider.chat.history.md"),
                work.join("shop/.aider.input.history"),
                work.join("shop/.crush"),
            ]
        );
        // Hidden where they really are, Crush's folder whole, and only those there: what a link leads to that is
        // missing stays out.
        let hidden = |shown: &[PathBuf]| secrets_in(None, &[], shown, &found);
        assert_eq!(
            hidden(std::slice::from_ref(&work)),
            Hidden {
                masks: vec![
                    (work.join(".aider.chat.history.md"), false),
                    (elsewhere.join("chat.md"), false),
                    (work.join("shop/.aider.input.history"), false),
                    (work.join("shop/.crush"), true),
                ],
                missing: Vec::new(),
            }
        );
        // One that a key names shows.
        let named = hidden(&[work.clone(), work.join("shop/.crush")]).masks;
        assert!(!named.contains(&(work.join("shop/.crush"), true)) && named.len() == 3);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_star_in_a_listed_name_stands_for_any_run_of_characters() {
        for (pattern, name) in [
            ("*.sqlite*", "state_5.sqlite"),
            ("*.sqlite*", "logs_2.sqlite-wal"),
            ("session-store.db*", "session-store.db-shm"),
            ("command-history-state*", "command-history-state.json"),
            ("a*", "a"),
            ("*", ""),
            ("a*b*c", "abc"),
            ("a*b*c", "a-b-b-c"),
            ("abc", "abc"),
        ] {
            assert!(matches(pattern, name), "{pattern} should match {name}");
        }
        for (pattern, name) in [
            ("*.sqlite*", "config.toml"),
            ("*.sqlite*", "sqlite"),
            ("session-store.db*", "session-store"),
            ("a*a", "a"),
            ("a*b*c", "acb"),
            ("abc", "abcd"),
        ] {
            assert!(!matches(pattern, name), "{pattern} should not match {name}");
        }
    }

    #[test]
    fn reads_clock_times() {
        let now = world::now_ns();
        let at = parse_until("23:59", now).unwrap();
        assert!(at > now && at - now <= 24 * 3600 * 1_000_000_000);
        assert!(clock(at, now).ends_with(":59"));
        for bad in ["noon", "24:00", "12:60", "12:5", "1200", ""] {
            assert!(parse_until(bad, now).is_err(), "{bad}");
        }
        assert_eq!(clock(now, now).len(), 5);
    }
}
