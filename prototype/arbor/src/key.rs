//! Keys: what a command in a world may open beyond the world, until when, within which limits, and what the effects
//! it asks for may cost. A cell starts sealed (see `cell`): the home folders are empty, the environment is cleared
//! and there is no network. A key opens folders and files there, passes variables in by name, opens the network to
//! the hosts it names (see `net`), can give the cell a home folder of its own that is kept between runs, and caps the
//! cell's limits, until a set time, like a valet key. A key can be narrowed into another, never widened, and taking
//! one back takes back every key made within it.
//!
//! A key holds names and paths, never a secret's value. Each is a file in the state folder's `keys`. A key's own
//! home folder is in its `homes`: it holds what the commands under the key write there, such as an agent's settings
//! and login, and lives as long as the key.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
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
/// or its key includes the folder they are in, unless that world or key is the file or folder itself.
pub const SECRETS: [&str; 30] = [
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

/// The password and key files of the home folder `home` that a cell would see, with whether each is a folder. A
/// cell sees what lies outside its `hidden` folders, and what lies inside `shown`: its world's folder and what its
/// key opens. Each is found where it really is, so one linked from elsewhere counts too. One that is itself among
/// `shown` was opened on purpose and stays.
pub fn secrets_in(home: &Path, hidden: &[PathBuf], shown: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    let mut found: Vec<(PathBuf, bool)> = Vec::new();
    for rel in SECRETS {
        let Ok(path) = fs::canonicalize(home.join(rel)) else {
            continue;
        };
        let seen = !hidden.iter().any(|h| paths::contains(h, &path))
            || shown
                .iter()
                .any(|s| s != &path && paths::contains(s, &path));
        let named = shown.contains(&path);
        let covered = found.iter().any(|(f, _)| paths::contains(f, &path));
        if seen && !named && !covered {
            let dir = path.is_dir();
            found.push((path, dir));
        }
    }
    found
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
                return Err(Error::refused(format!(
                    "key '{}' ended at {}",
                    outer.name,
                    clock(outer.until_ns, now)
                )));
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
        key.save_new()?;
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

    /// Writes the key's file, mode 600, refusing a name that is taken.
    fn save_new(&self) -> Result<()> {
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
            sys::rename_noreplace(&tmp, &path)
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
            return Err(Error::refused(format!(
                "key '{}' ended at {}",
                self.name,
                clock(self.until_ns, now)
            )));
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

/// Takes back the key `name` and every key made within it, at any depth. Returns them, `name` first. Their home
/// folders stay until the caller has stopped what runs under them (see `Key::remove_home`).
pub fn take_back(name: &str) -> Result<Vec<Key>> {
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
        let found = |shown: &[PathBuf]| secrets_in(&home, std::slice::from_ref(&home), shown);

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
