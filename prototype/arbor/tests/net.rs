//! The network a key opens, end to end: a cell reaches the hosts its key names, through arbor's proxy, and nothing
//! else. None of the tests needs the internet: the host a cell reaches is a small server on this computer's own
//! network address. They skip like `common` says, and where `curl` is missing.

mod common;

use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, ToSocketAddrs, UdpSocket};
use std::process::Command;

use common::{Env, new_world, skip};

/// This computer's address on its network, which a key may name, unlike `127.0.0.1`.
fn own_address() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    // TEST-NET-1. Connecting a UDP socket sends nothing: it only picks the address a packet would leave from.
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// A server on this computer's network address that answers every request with `pong`, for as long as the test
/// runs. Returns its `ADDRESS:PORT`.
fn upstream() -> Option<String> {
    let listener = TcpListener::bind((own_address()?, 0)).ok()?;
    let addr = listener.local_addr().ok()?.to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { continue };
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match c.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = c.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\npong\n",
                );
            });
        }
    });
    Some(addr)
}

/// Whether `curl` is here to stand for the tools in a cell.
fn curl() -> bool {
    let found = Command::new("curl")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        skip("curl is not installed");
    }
    found
}

/// Runs `cmd` in world demo, under `key` if given. Returns its exit code, what it printed, and its errors.
fn run(e: &Env, key: Option<&str>, cmd: &[&str]) -> (Option<i32>, String, String) {
    let mut args = vec!["run", "demo"];
    args.extend(key.map(|k| ["--key", k]).into_iter().flatten());
    args.push("--");
    args.extend(cmd);
    let out = e.command(&args).output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The ledger's rows of the network.
fn net_rows(e: &Env) -> Vec<String> {
    e.ledger_rows()
        .into_iter()
        .filter(|r| r.split_whitespace().nth(3) == Some("net"))
        .collect()
}

#[test]
fn a_key_opens_the_hosts_it_names_and_nothing_else() {
    let Some(e) = Env::new("net") else { return };
    if !curl() {
        return;
    }
    let Some(server) = upstream() else {
        skip("this computer has no network address to serve on");
        return;
    };
    new_world(&e);
    e.ok(&["key", "new", "lan", "--net", &server]);
    let url = format!("http://{server}/");

    // Through the cell's proxy, in a tunnel (`-p`), the way HTTPS goes.
    let (code, out, err) = run(&e, Some("lan"), &["curl", "-sS", "-p", &url]);
    assert_eq!((code, out.as_str()), (Some(0), "pong\n"), "{err}");

    // A host the key does not name gets no connection, and arbor says why once the command ends.
    let (code, _, err) = run(
        &e,
        Some("lan"),
        &["curl", "-sS", "https://not-on-key.example/"],
    );
    assert_ne!(code, Some(0));
    assert!(
        err.contains(
            "key 'lan' does not open not-on-key.example, so the command could not reach it"
        ),
        "{err}"
    );

    // Plain HTTP is refused, in arbor's words.
    let (code, out, err) = run(&e, Some("lan"), &["curl", "-sS", &url]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("only https goes out of a world"), "{out}");
    assert!(
        err.contains(&format!("plain http to {server} was refused")),
        "{err}"
    );

    // The variables point tools at the proxy. arbor, which opened the proxy's port, became the command: it is the
    // cell's second process, right after bubblewrap's.
    let (_, out, _) = run(&e, Some("lan"), &["sh", "-c", "echo $HTTPS_PROXY $$"]);
    assert_eq!(out, "http://127.0.0.1:3128 2\n");

    // Without the key, there is neither a proxy nor any network.
    let (_, out, _) = run(&e, None, &["sh", "-c", "echo \"[$HTTPS_PROXY]\" $$"]);
    assert_eq!(out, "[] 2\n");
    let (code, _, _) = run(&e, None, &["curl", "-sS", "-m", "5", "-p", &url]);
    assert_ne!(code, Some(0));

    // The ledger: each refused host as it happened, and a step for each cell that used the network.
    let rows = net_rows(&e);
    let row = |what: &str| rows.iter().any(|r| r.contains(what));
    assert!(row(&format!("{server}: ")) && row(" in, "), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|r| r.contains(" refused ") && r.ends_with(" not-on-key.example")),
        "{rows:#?}"
    );
    assert!(row(&format!("{server}: plain http")), "{rows:#?}");
    assert!(e.ok(&["ledger", "--verify"]).contains("ok"));
}

#[test]
fn a_key_opens_no_host_its_outer_key_does_not() {
    let Some(e) = Env::new("netkeys") else { return };
    let refused = |args: &[&str], code: i32, msg: &str| {
        let out = e.arbor(args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {err}");
        assert!(err.contains(msg), "{args:?}: {err}");
    };
    e.ok(&[
        "key",
        "new",
        "outer",
        "--net",
        "*.example.com",
        "--net",
        "https://api.example.org/v1",
    ]);
    // A narrower key's hosts lie within the outer key's; one that names none inherits them.
    e.ok(&[
        "key",
        "new",
        "inner",
        "--within",
        "outer",
        "--net",
        "api.example.com",
    ]);
    e.ok(&["key", "new", "same", "--within", "outer"]);
    refused(
        &[
            "key",
            "new",
            "wider",
            "--within",
            "outer",
            "--net",
            "example.com",
        ],
        3,
        "key 'wider' would open more than key 'outer', which does not open example.com",
    );
    let keys = e.ok(&["keys"]);
    let hosts = |name: &str| {
        keys.lines()
            .find(|l| l.starts_with(&format!("{name} ")))
            .unwrap_or_default()
            .to_string()
    };
    assert!(hosts("inner").contains("api.example.com"), "{keys}");
    assert!(
        hosts("same").contains("*.example.com, api.example.org"),
        "{keys}"
    );

    // Inside a world localhost is the world's own; and only HTTPS goes out.
    for own in ["localhost", "127.0.0.1:8080", "[::1]"] {
        refused(
            &["key", "new", "own", "--net", own],
            3,
            "this computer's own services stay closed",
        );
    }
    refused(
        &["key", "new", "plain", "--net", "http://example.com"],
        2,
        "only https goes out of a world",
    );
}

#[test]
fn a_host_name_never_reaches_this_computer() {
    let Some(e) = Env::new("netlocal") else {
        return;
    };
    if !curl() {
        return;
    }
    // Debian's and Ubuntu's /etc/hosts name ::1 `ip6-localhost`.
    let name = "ip6-localhost";
    let local = (name, 443)
        .to_socket_addrs()
        .map(|a| a.collect::<Vec<_>>())
        .is_ok_and(|a| !a.is_empty() && a.iter().all(|a| a.ip().is_loopback()));
    if !local {
        skip("ip6-localhost does not name this computer here");
        return;
    }
    new_world(&e);
    e.ok(&["key", "new", "k", "--net", name]);
    let (code, _, err) = run(&e, Some("k"), &["curl", "-sS", "https://ip6-localhost/"]);
    assert_ne!(code, Some(0));
    assert!(
        err.contains("ip6-localhost resolves to an address of this computer or its network"),
        "{err}"
    );
    assert!(
        net_rows(&e)
            .iter()
            .any(|r| r.contains("ip6-localhost: resolves to a local address")),
        "{:#?}",
        net_rows(&e)
    );
}
