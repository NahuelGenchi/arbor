//! The network a key opens: the hosts it names, over HTTPS, and nothing else.
//!
//! A cell has a network of its own with nothing in it but loopback. Under a key with `--net`, arbor itself is the
//! cell's first process: it opens `127.0.0.1:3128` in that network, hands the open socket out to the arbor that
//! started the cell (`open_port`), and then becomes the command, so no extra process stays in the cell. That arbor
//! answers each connection as an HTTPS proxy (`Proxy`): it reads the `CONNECT` request, checks the host against the
//! key, connects from outside, and passes the bytes both ways without ever seeing inside the encryption. The cell's
//! variables point tools there (`vars`); a tool that ignores them gets no network at all.
//!
//! A host name never reaches this computer or its local network, even when it resolves there, as in DNS rebinding
//! or the cloud's metadata address: only an address the key names itself does. Inside a world `localhost` is the
//! world's own, so a key cannot open it.
//!
//! The ledger gets a step for each host refused, the first time in a cell, and one when the cell ends that sums up
//! what it reached.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cgroup;
use crate::error::{Context, Error, Result};
use crate::ledger::{self, Cost, Ledger};
use crate::sys;

/// The port of a cell's proxy, on the cell's own loopback.
pub const PORT: u16 = 3128;
/// The port a host opens when none is named: HTTPS's.
const HTTPS: u16 = 443;
/// The longest request head arbor reads, and how long it waits for one.
const MAX_HEAD: usize = 8 * 1024;
const HEAD_TIME: Duration = Duration::from_secs(10);
/// How long arbor waits for each address of a host to answer.
const CONNECT_TIME: Duration = Duration::from_secs(10);
/// Connections a cell may have open at once.
const MAX_OPEN: usize = 128;
/// The hosts refused in one cell that get a ledger step of their own; the rest are only counted.
const MAX_REFUSED_STEPS: usize = 16;
/// Each copy between a cell and a host moves at most a TLS record.
const CHUNK: usize = 16 * 1024;
/// The stack of a thread that only copies: its buffer is on the heap.
const PUMP_STACK: usize = 64 * 1024;
/// How long the end of a cell waits for connections still being made.
const LAST_WAIT: Duration = Duration::from_secs(1);

/// The variables that point a cell's tools at its proxy. They skip it for the cell's own `localhost`.
pub fn vars() -> Vec<(&'static str, String)> {
    let url = format!("http://127.0.0.1:{PORT}");
    let local = "localhost,127.0.0.1,::1".to_string();
    vec![
        ("HTTPS_PROXY", url.clone()),
        ("https_proxy", url.clone()),
        ("HTTP_PROXY", url.clone()),
        ("http_proxy", url),
        ("NO_PROXY", local.clone()),
        ("no_proxy", local),
        // Node's own `fetch` and `https` follow the variables only when asked to.
        ("NODE_USE_ENV_PROXY", "1".into()),
    ]
}

/// A host a key opens: a name, every name below a domain, or an address; and a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Host {
    pattern: Pattern,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    Name(String),
    /// `*.example.com`: every name that ends in `.example.com`.
    Below(String),
    Addr(IpAddr),
}

impl Host {
    /// Reads a host as `--net` takes it: `api.example.com`, `*.example.com`, an address, each with `:PORT` if not
    /// 443, or a pasted `https://` address.
    pub fn parse(text: &str) -> Result<Host> {
        let bad = |why: &str| Error::usage(format!("'{text}' is not a host: {why}"));
        let mut t = text.trim();
        if let Some((scheme, rest)) = t.split_once("://") {
            if !scheme.eq_ignore_ascii_case("https") {
                return Err(bad(
                    "only https goes out of a world; name the host, as in example.com",
                ));
            }
            t = rest.split(['/', '?', '#']).next().unwrap_or_default();
        }
        if t.contains('@') {
            return Err(bad(
                "a key holds names, never a password; leave out what comes before @",
            ));
        }
        let (host, port) =
            split_port(t).ok_or_else(|| bad("its port is not a number from 1 to 65535"))?;
        let port = port.unwrap_or(HTTPS);
        let pattern = if let Some(domain) = host.strip_prefix("*.") {
            let domain = name(domain).ok_or_else(|| bad("what follows *. is not a name"))?;
            if !domain.contains('.') {
                return Err(bad(
                    "a key opens the names below a domain, as *.example.com, not a whole top-level domain",
                ));
            }
            Pattern::Below(domain)
        } else if let Ok(ip) = host.parse::<IpAddr>() {
            let ip = ip.to_canonical();
            if ip.is_loopback() || ip.is_unspecified() {
                return Err(own_services(host));
            }
            Pattern::Addr(ip)
        } else {
            let n = name(host).ok_or_else(|| {
                bad("a host is a name of letters, digits, hyphens and dots, or an address")
            })?;
            if n == "localhost" || n.ends_with(".localhost") {
                return Err(own_services(host));
            }
            Pattern::Name(n)
        };
        Ok(Host { pattern, port })
    }

    /// True when this opens `t`.
    fn opens(&self, t: &Target) -> bool {
        self.port == t.port
            && match (&self.pattern, &t.host) {
                (Pattern::Name(n), Dest::Name(h)) => n == h,
                (Pattern::Below(d), Dest::Name(h)) => below(h, d),
                (Pattern::Addr(a), Dest::Addr(b)) => a == b,
                _ => false,
            }
    }

    /// True when this opens everything `other` opens, as a key made within another must.
    pub fn covers(&self, other: &Host) -> bool {
        self.port == other.port
            && match (&self.pattern, &other.pattern) {
                (a, b) if a == b => true,
                (Pattern::Below(d), Pattern::Name(n) | Pattern::Below(n)) => below(n, d),
                _ => false,
            }
    }
}

fn own_services(host: &str) -> Error {
    Error::refused(format!(
        "a key cannot open {host}: inside a world it is the world's own, and this computer's own services stay closed"
    ))
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pattern {
            Pattern::Name(n) => f.write_str(n)?,
            Pattern::Below(d) => write!(f, "*.{d}")?,
            Pattern::Addr(IpAddr::V6(a)) => write!(f, "[{a}]")?,
            Pattern::Addr(a) => write!(f, "{a}")?,
        }
        if self.port != HTTPS {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

impl TryFrom<String> for Host {
    type Error = Error;
    fn try_from(s: String) -> Result<Host> {
        Host::parse(&s)
    }
}

impl From<Host> for String {
    fn from(h: Host) -> String {
        h.to_string()
    }
}

/// Splits `host:port`, `[v6]:port` and a bare host or address. `None` when the port is not a number from 1 to
/// 65535.
fn split_port(t: &str) -> Option<(&str, Option<u16>)> {
    let port = |p: &str| p.parse::<u16>().ok().filter(|&p| p != 0);
    if let Some(rest) = t.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(port(after.strip_prefix(':')?)?))),
        };
    }
    match t.rsplit_once(':') {
        // A bare IPv6 address has several colons and no port.
        Some((host, _)) if host.contains(':') => Some((t, None)),
        Some((host, p)) => Some((host, Some(port(p)?))),
        None => Some((t, None)),
    }
}

/// A host name in lower case and without a final dot, if `text` is one: labels of 1 to 63 letters, digits and
/// hyphens, 253 characters in all, the last not a number, which would read as an address.
fn name(text: &str) -> Option<String> {
    let n = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
    let label = |l: &str| {
        (1..=63).contains(&l.len())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let last = n.rsplit('.').next().unwrap_or_default();
    (n.len() <= 253 && n.split('.').all(label) && !last.bytes().all(|b| b.is_ascii_digit()))
        .then_some(n)
}

/// True when `name` is a name below `domain`.
fn below(name: &str, domain: &str) -> bool {
    name.len() > domain.len() + 1
        && name.ends_with(domain)
        && name.as_bytes()[name.len() - domain.len() - 1] == b'.'
}

/// True for an address of this computer or of a local network, or one no host on the internet has: loopback,
/// private, link-local (the cloud's metadata address among them), shared (carrier-grade NAT, and Tailscale),
/// multicast, broadcast and reserved.
pub fn is_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(a) => {
            let [x, y, ..] = a.octets();
            a.is_loopback()
                || a.is_private()
                || a.is_link_local()
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_multicast()
                || x == 0
                || (x == 100 && (64..128).contains(&y))
                || (x == 198 && (18..20).contains(&y))
                || x >= 240
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                // Unique local (fc00::/7), link-local (fe80::/10) and the old site-local (fec0::/10).
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                // NAT64 (64:ff9b::/96) carries an IPv4 address in its last 32 bits.
                || (s[..6] == [0x64, 0xff9b, 0, 0, 0, 0]
                    && is_local(IpAddr::V4(Ipv4Addr::from(
                        (u32::from(s[6]) << 16) | u32::from(s[7]),
                    ))))
        }
    }
}

/// Where a connection asks to go.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    host: Dest,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Dest {
    Name(String),
    Addr(IpAddr),
}

impl Target {
    /// `host:port` as a request names it; a plain-HTTP request may leave the port out.
    fn parse(authority: &str, default_port: Option<u16>) -> Option<Target> {
        let (host, port) = split_port(authority)?;
        let host = match host.parse::<IpAddr>() {
            Ok(ip) => Dest::Addr(ip.to_canonical()),
            Err(_) => Dest::Name(name(host)?),
        };
        Some(Target {
            host,
            port: port.or(default_port)?,
        })
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Dest::Name(n) => f.write_str(n)?,
            Dest::Addr(IpAddr::V6(a)) => write!(f, "[{a}]")?,
            Dest::Addr(a) => write!(f, "{a}")?,
        }
        if self.port != HTTPS {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

enum Request {
    Connect(Target),
    Plain(Target),
}

/// Reads a request's first line: `CONNECT host:port HTTP/1.1`, or a plain-HTTP request that names its whole
/// address, as tools send them to a proxy.
fn parse_request(head: &[u8]) -> std::result::Result<Request, &'static str> {
    let line = head.split(|&b| b == b'\r').next().unwrap_or_default();
    let line = std::str::from_utf8(line).map_err(|_| "the request is not text")?;
    let mut words = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err("a request starts with a line such as `CONNECT example.com:443 HTTP/1.1`");
    };
    if !version.starts_with("HTTP/1.") {
        return Err("only HTTP/1 requests are read");
    }
    if method == "CONNECT" {
        return Target::parse(target, None)
            .map(Request::Connect)
            .ok_or("CONNECT names a host and its port, as in example.com:443");
    }
    let rest = target
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("http://"))
        .map(|_| &target[7..])
        .ok_or("https goes through CONNECT, as tools send it with HTTPS_PROXY")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    Target::parse(authority, Some(80))
        .map(Request::Plain)
        .ok_or("the address does not name a host")
}

/// Why a connection was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The key does not open the host.
    NotOpened,
    /// The host's name resolves only to local addresses.
    Local,
    /// Plain HTTP, not HTTPS.
    PlainHttp,
}

/// What a cell sent and received from one host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traffic {
    pub conns: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// What went through a cell's proxy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    /// Each host reached, as `example.com` or `example.com:8443`.
    pub reached: BTreeMap<String, Traffic>,
    /// Each host refused: why, and how many times.
    pub refused: BTreeMap<String, (Why, u64)>,
    /// Each host that could not be found or reached: how many times.
    pub unreachable: BTreeMap<String, u64>,
    /// How long the proxy ran.
    pub ms: u64,
}

/// `a`, `a and b`, `a, b and 3 more`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

impl Tally {
    pub fn is_empty(&self) -> bool {
        self.reached.is_empty() && self.refused.is_empty() && self.unreachable.is_empty()
    }

    /// For the ledger: `api.example.com ×3, cdn.example.com ×1: 1.2MiB in, 40KiB out; refused example.org ×2`.
    pub fn summary(&self) -> String {
        let times = |n: u64| {
            if n == 1 {
                String::new()
            } else {
                format!(" ×{n}")
            }
        };
        let mut parts = Vec::new();
        if !self.reached.is_empty() {
            let hosts: Vec<String> = self
                .reached
                .iter()
                .map(|(h, t)| format!("{h}{}", times(t.conns)))
                .collect();
            let (bytes_in, bytes_out) = self
                .reached
                .values()
                .fold((0, 0), |(i, o), t| (i + t.bytes_in, o + t.bytes_out));
            parts.push(format!(
                "{}: {} in, {} out",
                hosts.join(", "),
                cgroup::size(bytes_in),
                cgroup::size(bytes_out)
            ));
        }
        if !self.refused.is_empty() {
            let hosts: Vec<String> = self
                .refused
                .iter()
                .map(|(h, (_, n))| format!("{h}{}", times(*n)))
                .collect();
            parts.push(format!("refused {}", hosts.join(", ")));
        }
        if !self.unreachable.is_empty() {
            let hosts: Vec<String> = self
                .unreachable
                .iter()
                .map(|(h, n)| format!("{h}{}", times(*n)))
                .collect();
            parts.push(format!("could not reach {}", hosts.join(", ")));
        }
        parts.join("; ")
    }

    /// The step's outcome for the ledger.
    pub fn outcome(&self) -> &'static str {
        if self.refused.is_empty() {
            "ok"
        } else if self.reached.is_empty() && self.unreachable.is_empty() {
            "refused"
        } else {
            "some refused"
        }
    }

    /// Lines for the person when key `key` kept the command from a host, or none.
    pub fn refusals(&self, key: &str) -> Vec<String> {
        let hosts = |why: Why| -> Vec<String> {
            self.refused
                .iter()
                .filter(|(_, (w, _))| *w == why)
                .map(|(h, _)| h.clone())
                .collect()
        };
        let mut lines = Vec::new();
        let not = hosts(Why::NotOpened);
        if !not.is_empty() {
            lines.push(format!(
                "key '{key}' does not open {}, so the command could not reach {}. A key opens a host with `--net HOST`.",
                listed(&not),
                if not.len() == 1 { "it" } else { "them" }
            ));
        }
        let local = hosts(Why::Local);
        if !local.is_empty() {
            lines.push(format!(
                "{} resolve{} to an address of this computer or its network, which a host name never opens; a key that names the address itself does.",
                listed(&local),
                if local.len() == 1 { "s" } else { "" }
            ));
        }
        let plain = hosts(Why::PlainHttp);
        if !plain.is_empty() {
            lines.push(format!(
                "plain http to {} was refused: only https goes out of a world.",
                listed(&plain)
            ));
        }
        lines
    }
}

/// Who a cell's connections are recorded for.
pub struct Who {
    pub world: String,
    pub cell: String,
    pub key: String,
}

/// A cell's proxy, answering on threads of its own until `finish`.
pub struct Proxy {
    stop: Option<OwnedFd>,
    accepting: Option<JoinHandle<Vec<JoinHandle<()>>>>,
    shared: Arc<Shared>,
    started: Instant,
}

struct Shared {
    hosts: Vec<Host>,
    who: Who,
    tally: Mutex<Tally>,
    open: Mutex<Open>,
}

/// The connections passing bytes now, to close when the cell ends.
#[derive(Default)]
struct Open {
    closing: bool,
    next: u64,
    conns: HashMap<u64, [Arc<TcpStream>; 2]>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Proxy {
    /// Starts answering the connections of a cell under a key that opens `hosts`, once the cell's first process
    /// hands its port over `from`.
    pub fn start(from: UnixStream, hosts: Vec<Host>, who: Who) -> Result<Proxy> {
        let (stopped, stop) = sys::pipe()?;
        let shared = Arc::new(Shared {
            hosts,
            who,
            tally: Mutex::default(),
            open: Mutex::default(),
        });
        let s = shared.clone();
        let accepting = sys::spawn_quiet(move || serve(from, stopped, s))
            .ctx(|| "cannot start the world's proxy".to_string())?;
        Ok(Proxy {
            stop: Some(stop),
            accepting: Some(accepting),
            shared,
            started: Instant::now(),
        })
    }

    /// Stops answering once the cell has ended, closes what is still open, and returns what went through.
    pub fn finish(mut self) -> Tally {
        drop(self.stop.take());
        let threads = self
            .accepting
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        {
            let mut open = lock(&self.shared.open);
            open.closing = true;
            for conn in open.conns.values().flatten() {
                let _ = conn.shutdown(Shutdown::Both);
            }
        }
        // A connection still finding or reaching its host ends on its own; arbor waits a moment for it.
        let until = Instant::now() + LAST_WAIT;
        while threads.iter().any(|t| !t.is_finished()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        for t in threads.into_iter().filter(JoinHandle::is_finished) {
            let _ = t.join();
        }
        let mut tally = std::mem::take(&mut *lock(&self.shared.tally));
        tally.ms = self.started.elapsed().as_millis() as u64;
        tally
    }
}

/// Waits for the cell's port, then takes its connections until `stopped` closes. Returns the threads still
/// answering.
fn serve(from: UnixStream, stopped: OwnedFd, shared: Arc<Shared>) -> Vec<JoinHandle<()>> {
    let mut threads: Vec<JoinHandle<()>> = Vec::new();
    match sys::ready(&[from.as_fd(), stopped.as_fd()], None) {
        Ok(ready) if !ready[1] => {}
        _ => return threads,
    }
    let Ok(Some(port)) = sys::recv_fd(from.as_fd()) else {
        return threads;
    };
    drop(from);
    let listener = TcpListener::from(port);
    if !is_cell_port(&listener) || listener.set_nonblocking(true).is_err() {
        return threads;
    }
    loop {
        match sys::ready(&[listener.as_fd(), stopped.as_fd()], None) {
            Ok(ready) if !ready[1] => {}
            _ => break,
        }
        let conn = match listener.accept() {
            Ok((conn, _)) => conn,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(_) => {
                // Out of descriptors, say: wait instead of spinning.
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        threads.retain(|t| !t.is_finished());
        if conn.set_nonblocking(false).is_err() {
            continue;
        }
        if threads.len() >= MAX_OPEN {
            let _ = reply(
                &conn,
                503,
                "this world has too many connections open at once",
            );
            continue;
        }
        let s = shared.clone();
        if let Ok(t) = sys::spawn_quiet(move || s.answer(conn)) {
            threads.push(t);
        }
    }
    threads
}

/// True for a listening TCP socket at the cell's own `127.0.0.1:3128`: the port its first process opens.
fn is_cell_port(listener: &TcpListener) -> bool {
    let mut listening: libc::c_int = 0;
    let mut len = size_of::<libc::c_int>() as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            (&raw mut listening).cast(),
            &mut len,
        )
    } == 0;
    ok && listening == 1
        && listener
            .local_addr()
            .is_ok_and(|a| a == SocketAddr::from((Ipv4Addr::LOCALHOST, PORT)))
}

/// Why no address of a host can be used.
enum Unusable {
    Local(IpAddr),
    Unknown(io::Error),
}

/// Where to connect for `t`: the address it names, or those of its name that are not local.
fn addresses(t: &Target) -> std::result::Result<Vec<SocketAddr>, Unusable> {
    let name = match &t.host {
        Dest::Addr(ip) => return Ok(vec![SocketAddr::new(*ip, t.port)]),
        Dest::Name(n) => n,
    };
    let all: Vec<SocketAddr> = (name.as_str(), t.port)
        .to_socket_addrs()
        .map_err(Unusable::Unknown)?
        .collect();
    let public: Vec<SocketAddr> = all.iter().copied().filter(|a| !is_local(a.ip())).collect();
    match all.first() {
        None => Err(Unusable::Unknown(io::Error::new(
            io::ErrorKind::NotFound,
            "it has no address",
        ))),
        Some(a) if public.is_empty() => Err(Unusable::Local(a.ip())),
        Some(_) => Ok(public),
    }
}

/// Connects to the first of `addrs` that answers.
fn connect(addrs: &[SocketAddr]) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "it has no address");
    for a in addrs {
        match TcpStream::connect_timeout(a, CONNECT_TIME) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Reads a request head, up to its blank line. Returns it, and whatever the client sent after it.
fn read_head(mut s: &TcpStream) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut head = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match s.read(&mut chunk) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        // The blank line may straddle two reads.
        let from = head.len().saturating_sub(3);
        head.extend_from_slice(&chunk[..n]);
        if let Some(i) = head[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = head.split_off(from + i + 4);
            return Ok((head, rest));
        }
        if head.len() > MAX_HEAD {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
}

/// Answers a request with an error. Its text goes in the status line too, where tools that show nothing else
/// show it.
fn reply(mut s: &TcpStream, code: u16, text: &str) -> io::Result<()> {
    let body = format!("arbor: {text}\n");
    let msg = format!(
        "HTTP/1.1 {code} arbor: {text}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(msg.as_bytes())
}

/// Copies from `from` to `to` until `from` ends, then tells `to` that nothing more comes. Returns the bytes.
fn pump(mut from: &TcpStream, mut to: &TcpStream) -> u64 {
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0;
    loop {
        match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
                total += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = to.shutdown(Shutdown::Write);
    total
}

impl Shared {
    fn tally<T>(&self, f: impl FnOnce(&mut Tally) -> T) -> T {
        f(&mut lock(&self.tally))
    }

    fn answer(&self, cell: TcpStream) {
        let _ = cell.set_nodelay(true);
        let _ = cell.set_read_timeout(Some(HEAD_TIME));
        let Ok((head, early)) = read_head(&cell) else {
            return;
        };
        match parse_request(&head) {
            Err(why) => {
                let _ = reply(&cell, 400, why);
            }
            Ok(Request::Plain(t)) => {
                self.refuse(&t, Why::PlainHttp);
                let _ = reply(
                    &cell,
                    403,
                    &format!("only https goes out of a world; plain http to {t} is refused"),
                );
            }
            Ok(Request::Connect(t)) => self.tunnel(cell, &t, &early),
        }
    }

    fn tunnel(&self, cell: TcpStream, t: &Target, early: &[u8]) {
        if !self.hosts.iter().any(|h| h.opens(t)) {
            self.refuse(t, Why::NotOpened);
            let _ = reply(
                &cell,
                403,
                &format!("key '{}' does not open {t}", self.who.key),
            );
            return;
        }
        let up = match addresses(t) {
            Ok(addrs) => connect(&addrs),
            Err(Unusable::Local(ip)) => {
                self.refuse(t, Why::Local);
                let _ = reply(
                    &cell,
                    403,
                    &format!(
                        "{t} resolves to {ip}, an address of this computer or its network, which a host name never opens"
                    ),
                );
                return;
            }
            Err(Unusable::Unknown(e)) => Err(e),
        };
        let up = match up {
            Ok(up) => up,
            Err(e) => {
                self.tally(|t2| *t2.unreachable.entry(t.to_string()).or_default() += 1);
                let _ = reply(&cell, 502, &format!("cannot reach {t}: {e}"));
                return;
            }
        };
        if (&cell)
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .is_err()
            || cell.set_read_timeout(None).is_err()
            || (&up).write_all(early).is_err()
        {
            return;
        }
        let host = t.to_string();
        self.tally(|t2| t2.reached.entry(host.clone()).or_default().conns += 1);
        let (cell, up) = (Arc::new(cell), Arc::new(up));
        let id = {
            let mut open = lock(&self.open);
            if open.closing {
                return;
            }
            let id = open.next;
            open.next += 1;
            open.conns.insert(id, [cell.clone(), up.clone()]);
            id
        };
        let (from, to) = (up.clone(), cell.clone());
        let down = sys::spawn_quiet_on(
            std::thread::Builder::new().stack_size(PUMP_STACK),
            move || pump(&from, &to),
        );
        let (sent, got) = match down {
            Ok(down) => {
                let sent = pump(&cell, &up) + early.len() as u64;
                (sent, down.join().unwrap_or(0))
            }
            Err(_) => (early.len() as u64, 0),
        };
        lock(&self.open).conns.remove(&id);
        self.tally(|t2| {
            let traffic = t2.reached.entry(host).or_default();
            traffic.bytes_out += sent;
            traffic.bytes_in += got;
        });
    }

    /// Counts a refusal. The first for each host, up to a few hosts, is also a ledger step of its own, so that
    /// watchers see it as it happens.
    fn refuse(&self, t: &Target, why: Why) {
        let host = t.to_string();
        let first = self.tally(|t2| {
            let entry = t2.refused.entry(host.clone()).or_insert((why, 0));
            entry.1 += 1;
            entry.1 == 1 && t2.refused.len() <= MAX_REFUSED_STEPS
        });
        if !first {
            return;
        }
        let what = match why {
            Why::NotOpened => host,
            Why::Local => format!("{host}: resolves to a local address"),
            Why::PlainHttp => format!("{host}: plain http"),
        };
        let _ = Ledger::open().and_then(|l| {
            l.append(ledger::New {
                act: "net",
                by: "cell",
                key: Some(&self.who.key),
                world: &self.who.world,
                cell: Some(&self.who.cell),
                effect: None,
                what: &what,
                limits: None,
                out: "refused",
                cost: Cost::default(),
            })
        });
    }
}

/// The descriptor a cell's first process inherited from arbor to hand its port over, if `fd` is one: an open
/// Unix socket.
pub fn inherited(fd: RawFd) -> Option<OwnedFd> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let open = fd > 2 && unsafe { libc::fstat(fd, &mut st) } == 0;
    (open && (st.st_mode & libc::S_IFMT) == libc::S_IFSOCK)
        .then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// In a cell's first process: opens the proxy's port on the cell's own loopback and hands it out over `channel`,
/// to the arbor that started the cell.
pub fn open_port(channel: OwnedFd) -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT))
        .ctx(|| format!("cannot open port {PORT} inside the world"))?;
    sys::send_fd(channel.as_fd(), listener.as_fd())
        .ctx(|| "cannot hand the port to arbor".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(text: &str) -> Host {
        Host::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn target(text: &str) -> Target {
        Target::parse(text, None).unwrap()
    }

    #[test]
    fn reads_hosts_as_a_key_takes_them() {
        for (text, shown) in [
            ("api.example.com", "api.example.com"),
            ("API.Example.COM.", "api.example.com"),
            ("api.example.com:443", "api.example.com"),
            ("example.com:8443", "example.com:8443"),
            ("*.example.com", "*.example.com"),
            ("https://api.example.com/v1/messages?x=1", "api.example.com"),
            ("HTTPS://example.com:8443/", "example.com:8443"),
            ("192.168.1.20", "192.168.1.20"),
            ("10.0.0.5:8080", "10.0.0.5:8080"),
            ("[2001:db8::1]:8443", "[2001:db8::1]:8443"),
            ("2001:db8::1", "[2001:db8::1]"),
            ("nas", "nas"),
        ] {
            assert_eq!(host(text).to_string(), shown, "{text}");
        }
        for bad in [
            "",
            "http://example.com",
            "ftp://example.com",
            "user:secret@example.com",
            "*",
            "*.com",
            "*.*.example.com",
            "ex ample.com",
            "-bad.example.com",
            "bad_name.example.com",
            "1.2.3",
            "example.com:0",
            "example.com:70000",
            "example.com:https",
            "[2001:db8::1",
        ] {
            let e = Host::parse(bad).unwrap_err();
            assert_eq!(e.code, crate::error::USAGE, "{bad}: {e}");
        }
        // A cell's localhost is its own.
        for own in [
            "localhost",
            "LOCALHOST:8080",
            "app.localhost",
            "127.0.0.1",
            "127.1.2.3:80",
            "[::1]:8080",
            "::1",
            "0.0.0.0",
            "[::ffff:127.0.0.1]",
        ] {
            let e = Host::parse(own).unwrap_err();
            assert_eq!(e.code, crate::error::REFUSED, "{own}: {e}");
            assert!(e.msg.contains("this computer's own services"), "{e}");
        }
        // Keys store hosts as text.
        let h = host("*.example.com:8443");
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, "\"*.example.com:8443\"");
        assert_eq!(serde_json::from_str::<Host>(&json).unwrap(), h);
        assert!(serde_json::from_str::<Host>("\"localhost\"").is_err());
    }

    #[test]
    fn a_host_opens_its_names_on_its_port() {
        let name = host("api.example.com");
        assert!(name.opens(&target("api.example.com:443")));
        assert!(name.opens(&target("API.example.com.:443")));
        assert!(!name.opens(&target("api.example.com:80")));
        assert!(!name.opens(&target("example.com:443")));
        assert!(!name.opens(&target("x.api.example.com:443")));

        let below = host("*.example.com");
        assert!(below.opens(&target("api.example.com:443")));
        assert!(below.opens(&target("a.b.example.com:443")));
        assert!(!below.opens(&target("example.com:443")));
        assert!(!below.opens(&target("badexample.com:443")));
        assert!(!below.opens(&target("api.example.com:8443")));

        let addr = host("192.168.1.20:8443");
        assert!(addr.opens(&target("192.168.1.20:8443")));
        assert!(addr.opens(&target("[::ffff:192.168.1.20]:8443")));
        assert!(!addr.opens(&target("192.168.1.21:8443")));
        // An address opens itself, never a name that resolves there, and the other way round.
        assert!(!name.opens(&target("93.184.215.14:443")));
        assert!(!host("93.184.215.14").opens(&target("api.example.com:443")));
    }

    #[test]
    fn a_narrower_key_opens_no_host_the_wider_does_not() {
        let covers = |outer: &str, inner: &str| host(outer).covers(&host(inner));
        assert!(covers("api.example.com", "api.example.com"));
        assert!(covers("*.example.com", "api.example.com"));
        assert!(covers("*.example.com", "*.api.example.com"));
        assert!(covers("10.0.0.5:8080", "10.0.0.5:8080"));
        assert!(!covers("api.example.com", "*.example.com"));
        assert!(!covers("*.example.com", "example.com"));
        assert!(!covers("*.api.example.com", "*.example.com"));
        assert!(!covers("api.example.com", "api.example.com:8443"));
        assert!(!covers("*.example.com", "api.example.com:8443"));
    }

    #[test]
    fn reads_what_a_tool_asks_its_proxy() {
        let connect = |head: &str| match parse_request(head.as_bytes()) {
            Ok(Request::Connect(t)) => Ok(t.to_string()),
            Ok(Request::Plain(t)) => Err(format!("plain {t}")),
            Err(e) => Err(e.to_string()),
        };
        assert_eq!(
            connect("CONNECT api.example.com:443 HTTP/1.1\r\nHost: api.example.com:443\r\n\r\n"),
            Ok("api.example.com".into())
        );
        assert_eq!(
            connect("CONNECT [2001:db8::1]:8443 HTTP/1.0\r\n\r\n"),
            Ok("[2001:db8::1]:8443".into())
        );
        assert_eq!(
            connect("GET http://example.com/a?b HTTP/1.1\r\n\r\n"),
            Err("plain example.com:80".into())
        );
        assert_eq!(
            connect("GET HTTP://example.com:8080 HTTP/1.1\r\n\r\n"),
            Err("plain example.com:8080".into())
        );
        for bad in [
            "CONNECT api.example.com HTTP/1.1\r\n\r\n",
            "CONNECT localhost:3128\r\n\r\n",
            "CONNECT a b HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\n\r\n",
            "GET https://example.com/ HTTP/1.1\r\n\r\n",
            "CONNECT api.example.com:443 HTTP/2\r\n\r\n",
            "CONNECT user@x:443 HTTP/1.1\r\n\r\n",
            "\u{0}\u{1}\r\n\r\n",
        ] {
            assert!(connect(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn tells_local_addresses_from_public_ones() {
        for local in [
            "127.0.0.1",
            "127.1.2.3",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "198.18.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "fc00::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(is_local(local.parse().unwrap()), "{local}");
        }
        for public in [
            "93.184.215.14",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1111",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
        ] {
            assert!(!is_local(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn reads_a_request_head_and_what_follows_it() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            // The blank line straddles two writes.
            c.write_all(b"CONNECT a.example:443 HTTP/1.1\r\nHost: a\r\n\r")
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
            c.write_all(b"\nhello").unwrap();
        });
        let (s, _) = listener.accept().unwrap();
        let (head, rest) = read_head(&s).unwrap();
        client.join().unwrap();
        assert!(head.ends_with(b"Host: a\r\n\r\n"));
        // What came with the head is kept for the host.
        let mut rest = rest;
        while rest.len() < 5 {
            let mut more = [0u8; 8];
            let n = (&s).read(&mut more).unwrap();
            assert!(n > 0);
            rest.extend_from_slice(&more[..n]);
        }
        assert_eq!(rest, b"hello");
    }

    #[test]
    fn sums_up_what_went_through() {
        let mut t = Tally::default();
        assert!(t.is_empty());
        t.reached.insert(
            "api.example.com".into(),
            Traffic {
                conns: 3,
                bytes_in: 3 << 20,
                bytes_out: 40 << 10,
            },
        );
        t.reached.insert(
            "cdn.example.com:8443".into(),
            Traffic {
                conns: 1,
                bytes_in: 1 << 20,
                bytes_out: 0,
            },
        );
        assert_eq!(t.outcome(), "ok");
        assert!(t.refusals("agent").is_empty());
        let only_refused = Tally {
            refused: BTreeMap::from([("pastebin.com".to_string(), (Why::NotOpened, 1))]),
            ..Tally::default()
        };
        assert_eq!(only_refused.outcome(), "refused");
        t.refused.insert("pastebin.com".into(), (Why::NotOpened, 2));
        t.refused
            .insert("example.org:80".into(), (Why::PlainHttp, 1));
        t.unreachable.insert("gone.example.com".into(), 1);
        assert_eq!(
            t.summary(),
            "api.example.com ×3, cdn.example.com:8443: 4MiB in, 40KiB out; refused example.org:80, pastebin.com ×2; could not reach gone.example.com"
        );
        assert_eq!(t.outcome(), "some refused");
        assert_eq!(
            t.refusals("agent"),
            [
                "key 'agent' does not open pastebin.com, so the command could not reach it. A key opens a host with `--net HOST`.",
                "plain http to example.org:80 was refused: only https goes out of a world.",
            ]
        );
        assert_eq!(
            listed(&["a".into(), "b".into(), "c".into(), "d".into()]),
            "a, b and 2 more"
        );
    }
}
