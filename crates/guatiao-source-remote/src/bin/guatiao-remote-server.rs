// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao-remote-server`: a registry's providers, served to hosts.

#![forbid(unsafe_code)]

extern crate alloc;

use core::time::Duration;
use std::path::PathBuf;
use std::process::ExitCode;

use guatiao::library::Registry;
use guatiao::{Alloc, Map, Value};
use guatiao_source_remote::config::trust_of;
use guatiao_source_remote::{Described, LinkOptions, Server, Trust};

#[cfg(any(feature = "tcp", feature = "local"))]
use front::Front;

/// Connections in their handshake at once, unless `--max-handshakes` says.
const MAX_HANDSHAKES: usize = 64;

const USAGE: &str = "\
guatiao-remote-server (--stdio | --listen URL) [options]
guatiao-remote-server --state-dir DIR (--waiting | --accept FINGERPRINT)

  --stdio               serve one link on stdin and stdout (a host's child)
  --listen URL          serve every link to URL: tcp://ADDRESS:PORT is the
                        trusted level, ws://ADDRESS:PORT the same over
                        WebSocket (a browser), tls://ADDRESS:PORT the secure
                        level, quic://ADDRESS:PORT the same over QUIC (UDP),
                        local://name the local one (a socket path or a
                        pipe name), rdma://ADDRESS:PORT RDMA (experimental,
                        Linux only)
  --trust LEVEL         local | trusted | secure (default: local on --stdio)
  --range CIDR          admit peers from this address range (repeatable): a
                        permit rule of the access control list, which denies
                        what no rule permits
  --acl PATH            the access control list, a TOML or JSON file
                        ([[rule]] action, source, destination, port,
                        fingerprint; default); trusted needs it or --range
  --key-file PATH       the trusted level's pre-shared key
  --state-dir DIR       secure: this server's identity (DIR/identity.pem, made
                        when absent) and its trust store (DIR/peers): unless
                        --pin is given, clients it does not know wait there
                        until accepted
  --identity PATH       secure: the identity file instead (made when absent)
  --pin FINGERPRINT     secure: trust this client (repeatable); with any, the
                        trust store no longer decides
  --policy PATH         secure: the TLS policy, a TOML or JSON file (default
                        strict); its [acl] is the access control list
  --waiting             list the clients waiting in --state-dir, then exit
  --accept FINGERPRINT  accept a waiting client in --state-dir, then exit
  --rdma-device NAME    the RDMA device rdma:// goes through (default: the first)
  --kinds PATH          a kind description document (repeatable)
  --load PATH           a library to load (repeatable)
  --scan DIR            a directory of libraries to load (repeatable)
  --source PATH[=JSON]  a source file and its configuration (repeatable)
  --refresh-ms N        refresh the sources every N milliseconds
  --max-calls N         calls one link may have running at once
  --max-frame N         the largest frame read, in bytes
  --max-handshakes N    connections in their handshake at once (default 64);
                        past it a new connection is closed unread";

struct Args {
    stdio: bool,
    listen: Option<String>,
    trust: Option<Trust>,
    ranges: Vec<String>,
    acl: Option<PathBuf>,
    key_file: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    identity: Option<PathBuf>,
    pins: Vec<String>,
    policy: Option<PathBuf>,
    waiting: bool,
    accept: Option<String>,
    rdma_device: Option<String>,
    kinds: Vec<PathBuf>,
    load: Vec<PathBuf>,
    scan: Vec<PathBuf>,
    sources: Vec<(PathBuf, Option<String>)>,
    refresh: Option<Duration>,
    max_calls: Option<usize>,
    max_frame: Option<usize>,
    max_handshakes: usize,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        stdio: false,
        listen: None,
        trust: None,
        ranges: Vec::new(),
        acl: None,
        key_file: None,
        state_dir: None,
        identity: None,
        pins: Vec::new(),
        policy: None,
        waiting: false,
        accept: None,
        rdma_device: None,
        kinds: Vec::new(),
        load: Vec::new(),
        scan: Vec::new(),
        sources: Vec::new(),
        refresh: None,
        max_calls: None,
        max_frame: None,
        max_handshakes: MAX_HANDSHAKES,
    };
    let need = |it: &mut core::iter::Skip<std::env::Args>, flag: &str| {
        it.next()
            .ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}"))
    };
    let num = |v: String, flag: &str| {
        v.parse::<u64>()
            .map_err(|_| format!("{flag} takes a whole number, not `{v}`"))
    };
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--stdio" => a.stdio = true,
            "--listen" => a.listen = Some(need(&mut it, &flag)?),
            "--trust" => {
                a.trust =
                    Some(trust_of(&need(&mut it, &flag)?).map_err(|e| e.message().to_string())?);
            }
            "--range" => a.ranges.push(need(&mut it, &flag)?),
            "--acl" => a.acl = Some(need(&mut it, &flag)?.into()),
            "--key-file" => a.key_file = Some(need(&mut it, &flag)?.into()),
            "--state-dir" => a.state_dir = Some(need(&mut it, &flag)?.into()),
            "--identity" => a.identity = Some(need(&mut it, &flag)?.into()),
            "--pin" => a.pins.push(need(&mut it, &flag)?),
            "--policy" => a.policy = Some(need(&mut it, &flag)?.into()),
            "--waiting" => a.waiting = true,
            "--accept" => a.accept = Some(need(&mut it, &flag)?),
            "--rdma-device" => a.rdma_device = Some(need(&mut it, &flag)?),
            "--kinds" => a.kinds.push(need(&mut it, &flag)?.into()),
            "--load" => a.load.push(need(&mut it, &flag)?.into()),
            "--scan" => a.scan.push(need(&mut it, &flag)?.into()),
            "--source" => {
                let v = need(&mut it, &flag)?;
                a.sources.push(match v.split_once('=') {
                    Some((p, json)) => (p.into(), Some(json.to_string())),
                    None => (v.into(), None),
                });
            }
            "--refresh-ms" => {
                a.refresh = Some(Duration::from_millis(num(need(&mut it, &flag)?, &flag)?));
            }
            "--max-calls" => a.max_calls = Some(num(need(&mut it, &flag)?, &flag)? as usize),
            "--max-frame" => a.max_frame = Some(num(need(&mut it, &flag)?, &flag)? as usize),
            "--max-handshakes" => {
                a.max_handshakes = match num(need(&mut it, &flag)?, &flag)? {
                    0 => return Err(format!("{flag} must be at least 1")),
                    n => n as usize,
                };
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("`{other}` is not an option\n\n{USAGE}")),
        }
    }
    if a.waiting || a.accept.is_some() {
        if a.state_dir.is_none() {
            return Err(format!(
                "--waiting and --accept need --state-dir\n\n{USAGE}"
            ));
        }
        return Ok(a);
    }
    if a.stdio == a.listen.is_some() {
        return Err(format!("name one of --stdio and --listen\n\n{USAGE}"));
    }
    Ok(a)
}

/// A TOML or JSON file as a value: JSON when its name ends in `.json`.
#[cfg(feature = "tcp")]
fn document(path: &std::path::Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let how = guatiao_serde::Presentation::new();
    let read = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        guatiao_serde::text::json::from_str(&text, Alloc::rust(), how)
    } else {
        guatiao_serde::text::toml::from_str(&text, Alloc::rust(), how)
    };
    read.map_err(|e| format!("{}: {e}", path.display()))
}

fn config_map(json: Option<&str>) -> Result<Map, String> {
    let Some(json) = json else {
        return Ok(Map::new());
    };
    let v: Value = guatiao_serde::text::json::from_str(
        json,
        Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .map_err(|e| format!("a source's configuration is not JSON: {e}"))?;
    Map::try_from(v).map_err(|_| "a source's configuration is not a JSON object".to_string())
}

fn registry(a: &Args) -> Result<(Registry, Described), String> {
    let mut described = Described::new();
    for path in &a.kinds {
        described
            .load(path)
            .map_err(|e| format!("{}: {}", path.display(), e.message()))?;
    }
    let mut registry = Registry::new("guatiao-remote-server", env!("CARGO_PKG_VERSION"));
    for path in &a.load {
        registry
            .load_file(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    for dir in &a.scan {
        guatiao::library::scan_dir(&mut registry, dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    for (path, json) in &a.sources {
        registry
            .add_source_file(path, &config_map(json.as_deref())?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok((registry, described))
}

fn options(a: &Args, trust: Trust) -> Result<LinkOptions, String> {
    let mut options = LinkOptions::new(trust);
    if let Some(path) = &a.key_file {
        let key = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        options = options.key(&key);
    }
    if let Some(n) = a.max_calls {
        options = options.max_calls(n);
    }
    if let Some(n) = a.max_frame {
        options = options.max_frame(n);
    }
    Ok(options)
}

/// The pairing commands: list the waiting clients, or accept one.
fn pairing(a: &Args) -> Result<(), String> {
    #[cfg(feature = "tls")]
    {
        use guatiao_source_remote::transport::tls::{PeerId, TrustStore};
        let dir = a.state_dir.as_deref().unwrap_or(std::path::Path::new("."));
        if !dir.join("peers").is_dir() {
            return Err(format!("{}: no trust store there", dir.display()));
        }
        let store = TrustStore::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if a.waiting {
            for entry in store.waiting().map_err(|e| e.to_string())? {
                match entry.source {
                    Some(source) => println!("{} from {source}", entry.id),
                    None => println!("{}", entry.id),
                }
            }
        }
        if let Some(text) = &a.accept {
            let id: PeerId = text
                .parse()
                .map_err(|_| format!("`{text}` is not a fingerprint"))?;
            store.accept(&id).map_err(|e| format!("{id}: {e}"))?;
            println!("accepted {id}");
        }
        Ok(())
    }
    #[cfg(not(feature = "tls"))]
    {
        let _ = a;
        Err("--waiting and --accept are the secure level's, which this build lacks".to_string())
    }
}

fn run() -> Result<(), String> {
    let a = parse()?;
    if a.waiting || a.accept.is_some() {
        return pairing(&a);
    }
    let (registry, described) = registry(&a)?;
    let trust = a.trust.unwrap_or(Trust::Local);
    let server = Server::new(registry, described, options(&a, trust)?);
    let refreshed = server.refresh();
    for r in &refreshed.refused {
        eprintln!("guatiao-remote-server: {}: {}", r.source, r.reason);
    }
    if let Some(every) = a.refresh {
        let s = server.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(every);
                let _ = s.refresh();
            }
        });
    }
    if a.stdio {
        let link = server
            .serve(guatiao_source_remote::transport::child::stdio())
            .map_err(|e| e.to_string())?;
        while link.alive() {
            std::thread::sleep(Duration::from_millis(50));
        }
        return Ok(());
    }
    let url = a.listen.as_deref().unwrap_or_default();
    let (scheme, address) = url
        .split_once("://")
        .ok_or_else(|| format!("--listen takes scheme://address, not `{url}`"))?;
    let _ = address;
    #[cfg(any(feature = "tcp", feature = "local"))]
    let front = Front::new(server, a.max_handshakes);
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" => serve_tcp(&front, address, &a),
        #[cfg(feature = "ws")]
        "ws" => serve_ws(&front, address, &a),
        #[cfg(feature = "tls")]
        "tls" => serve_tls(&front, address, &a),
        #[cfg(feature = "quic")]
        "quic" => serve_quic(&front, address, &a),
        #[cfg(feature = "rdma")]
        "rdma" => serve_rdma(&front, address, &a),
        #[cfg(feature = "local")]
        "local" => serve_local(&front, address),
        #[cfg(feature = "shm")]
        "shm" => serve_shm(&front, address),
        other => Err(format!("`{other}://` is not a transport this build has")),
    }
}

/// Every listener's connections: a bounded number in their handshake,
/// refusals reported sparingly.
#[cfg(any(feature = "tcp", feature = "local"))]
mod front {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::time::Duration;
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};
    use std::time::Instant;

    use guatiao_source_remote::Server;
    use guatiao_source_remote::transport::Connection;

    /// How often one source's refusals are reported.
    const REPORT_EVERY: Duration = Duration::from_secs(60);

    /// Sources whose last report is remembered; past it, within one interval,
    /// a new source's refusals go unreported.
    const REMEMBERED: usize = 1024;

    /// Says one thing at most once per key per interval, counting what it
    /// kept quiet in between.
    struct Throttle {
        every: Duration,
        /// When each key was last reported, and how often it was quiet since.
        seen: HashMap<String, (Instant, u64)>,
    }

    impl Throttle {
        fn new(every: Duration) -> Throttle {
            Throttle {
                every,
                seen: HashMap::new(),
            }
        }

        /// `Some(quiet)` when `key` is to be reported now, `quiet` being how
        /// often it was held back since its last report.
        fn pass(&mut self, key: &str, now: Instant) -> Option<u64> {
            if let Some((said, quiet)) = self.seen.get_mut(key) {
                if now.duration_since(*said) < self.every {
                    *quiet += 1;
                    return None;
                }
                let held = *quiet;
                *said = now;
                *quiet = 0;
                return Some(held);
            }
            if self.seen.len() >= REMEMBERED {
                let every = self.every;
                self.seen
                    .retain(|_, (said, _)| now.duration_since(*said) < every);
                if self.seen.len() >= REMEMBERED {
                    return None;
                }
            }
            self.seen.insert(key.to_string(), (now, 0));
            Some(0)
        }
    }

    /// What every listener hands its connections to: a thread each for the
    /// handshake, at most `max` at once, and refusals reported sparingly.
    pub(super) struct Front {
        pub(super) server: Server,
        max: usize,
        running: AtomicUsize,
        /// Refused sources, by address.
        said: Mutex<Throttle>,
        /// Connections past `max`: apart, so many sources cannot silence it.
        crowded: Mutex<Throttle>,
    }

    /// A handshake's place, given back when its thread is done.
    struct Slot(Arc<Front>);

    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.running.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// A peer's words with every `host:port` cut to its host: what a
    /// source's reports are keyed by.
    fn without_port(peer: &str) -> String {
        let host = |word: &'_ str| match word.rsplit_once(':') {
            Some((host, port))
                if !host.is_empty()
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit()) =>
            {
                host.to_string()
            }
            _ => word.to_string(),
        };
        peer.split(' ').map(host).collect::<Vec<_>>().join(" ")
    }

    fn since(quiet: u64) -> String {
        if quiet == 0 {
            String::new()
        } else {
            format!(" ({quiet} more since the last report)")
        }
    }

    impl Front {
        pub(super) fn new(server: Server, max: usize) -> Arc<Front> {
            Arc::new(Front {
                server,
                max,
                running: AtomicUsize::new(0),
                said: Mutex::new(Throttle::new(REPORT_EVERY)),
                crowded: Mutex::new(Throttle::new(REPORT_EVERY)),
            })
        }

        /// Prints `words(quiet)` unless `key` was reported within the interval.
        fn say(&self, to: &Mutex<Throttle>, key: &str, words: impl FnOnce(u64) -> String) {
            let pass = to
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pass(key, Instant::now());
            if let Some(quiet) = pass {
                eprintln!("guatiao-remote-server: {}", words(quiet));
            }
        }

        /// Serves `conn` on a thread of its own, or closes it unread when
        /// `max` handshakes are running.
        pub(super) fn take(self: &Arc<Self>, conn: Connection) {
            if self.running.fetch_add(1, Ordering::AcqRel) >= self.max {
                self.running.fetch_sub(1, Ordering::AcqRel);
                let peer = conn.peer.clone();
                drop(conn);
                self.say(&self.crowded, "handshakes", |quiet| {
                    let max = self.max;
                    format!(
                        "at most {max} handshakes run at once: closed {peer} unread{}",
                        since(quiet)
                    )
                });
                return;
            }
            let slot = Slot(Arc::clone(self));
            let started = std::thread::Builder::new().spawn(move || {
                let front = Arc::clone(&slot.0);
                let peer = conn.peer.clone();
                let served = front.server.serve(conn);
                // The handshake is over either way: its place is free.
                drop(slot);
                if let Err(e) = served {
                    front.say(&front.said, &without_port(&peer), |quiet| {
                        format!("a link from {peer} was refused: {e}{}", since(quiet))
                    });
                }
            });
            if let Err(e) = started {
                eprintln!("guatiao-remote-server: no thread for a connection: {e}");
            }
        }

        /// Reports a source the access control list denied.
        #[cfg(feature = "tcp")]
        pub(super) fn denied(&self, source: core::net::SocketAddr) {
            let ip = source.ip().to_string();
            self.say(&self.said, &ip, |quiet| {
                format!("the access control list denies {ip}{}", since(quiet))
            });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_source_is_reported_once_per_interval_with_what_was_held_back() {
            let mut t = Throttle::new(Duration::from_secs(60));
            let start = Instant::now();
            assert_eq!(t.pass("192.0.2.1", start), Some(0));
            for s in 1..=5 {
                assert_eq!(t.pass("192.0.2.1", start + Duration::from_secs(s)), None);
            }
            assert_eq!(
                t.pass("192.0.2.2", start),
                Some(0),
                "another source is its own"
            );
            assert_eq!(
                t.pass("192.0.2.1", start + Duration::from_secs(60)),
                Some(5)
            );
            assert_eq!(t.pass("192.0.2.1", start + Duration::from_secs(61)), None);
        }

        #[test]
        fn past_the_sources_it_remembers_a_new_one_goes_unreported_until_old_ones_expire() {
            let mut t = Throttle::new(Duration::from_secs(60));
            let start = Instant::now();
            for i in 0..REMEMBERED {
                assert_eq!(t.pass(&format!("s{i}"), start), Some(0));
            }
            assert_eq!(t.pass("late", start + Duration::from_secs(1)), None);
            assert_eq!(t.seen.len(), REMEMBERED);
            assert_eq!(t.pass("late", start + Duration::from_secs(60)), Some(0));
            assert_eq!(t.seen.len(), 1, "the expired sources were let go");
        }

        #[test]
        fn a_report_is_keyed_by_the_address_without_its_port() {
            assert_eq!(without_port("tcp 192.0.2.1:7000"), "tcp 192.0.2.1");
            assert_eq!(without_port("tls [2001:db8::1]:443"), "tls [2001:db8::1]");
            assert_eq!(
                without_port("local /run/guatiao.sock"),
                "local /run/guatiao.sock"
            );
            assert_eq!(without_port(r"local \\.\pipe\x"), r"local \\.\pipe\x");
            assert_eq!(
                without_port("rdma 192.0.2.1:7000 through mlx5_0"),
                "rdma 192.0.2.1 through mlx5_0"
            );
        }
    }
}

/// The access control list `--acl` and `--range` name, if they name one.
#[cfg(feature = "tcp")]
fn named_acl(a: &Args) -> Result<Option<guatiao_source_remote::acl::Acl>, String> {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    let mut acl = match &a.acl {
        Some(path) => Some(
            guatiao_source_remote::config::acl_of(&document(path)?)
                .map_err(|e| format!("{}: {}", path.display(), e.message()))?,
        ),
        None => None,
    };
    if !a.ranges.is_empty() {
        let acl = acl.get_or_insert_with(Acl::default);
        for range in &a.ranges {
            let source = range.parse().map_err(|e| format!("--range {range}: {e}"))?;
            acl.push(Rule::new(Action::Permit, source));
        }
    }
    Ok(acl)
}

/// An access control list on one line, for the line a server starts with.
#[cfg(feature = "tcp")]
fn listing(acl: &guatiao_source_remote::acl::Acl) -> String {
    acl.to_string().trim_end().replace('\n', "; ")
}

/// The trusted level's access control list: one must be named, since the
/// default denies every peer.
#[cfg(feature = "tcp")]
fn trusted_acl(
    server: &Server,
    scheme: &str,
    a: &Args,
) -> Result<guatiao_source_remote::acl::Acl, String> {
    if server.options().trust() != Trust::Trusted {
        return Err(format!(
            "{scheme}:// serves the trusted level: pass --trust trusted"
        ));
    }
    named_acl(a)?.ok_or_else(|| {
        format!("{scheme}:// admits the peers --range or --acl names: with neither, every peer would be closed unread")
    })
}

/// Hands every connection `accept` admits to `front`.
#[cfg(feature = "tcp")]
fn serve_accepted(
    front: &alloc::sync::Arc<Front>,
    accept: impl Fn() -> std::io::Result<guatiao_source_remote::transport::tcp::Accepted>,
) -> Result<(), String> {
    use guatiao_source_remote::transport::tcp::Accepted;
    loop {
        match accept() {
            Ok(Accepted::Connection(conn)) => front.take(conn),
            Ok(Accepted::Refused(peer)) => front.denied(peer),
            Ok(_) => {}
            Err(e) => eprintln!("guatiao-remote-server: accept: {e}"),
        }
    }
}

#[cfg(feature = "tcp")]
fn serve_tcp(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::tcp::TcpListen;
    let server = &front.server;
    let acl = trusted_acl(server, "tcp", a)?;
    let said = listing(&acl);
    let listener = TcpListen::bind(address, acl).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving tcp://{} admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(front, || listener.accept())
}

#[cfg(feature = "ws")]
fn serve_ws(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::ws::WsListen;
    let acl = trusted_acl(&front.server, "ws", a)?;
    let said = listing(&acl);
    let listener = WsListen::bind(address, acl).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving ws://{} admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(front, || listener.accept())
}

/// The secure level's configuration the options name: identity, pins,
/// policy and access control list, and the list in words.
#[cfg(feature = "tls")]
fn secure_config(
    server: &Server,
    scheme: &str,
    a: &Args,
) -> Result<
    (
        guatiao_source_remote::transport::tls::TlsConfig,
        String,
        String,
    ),
    String,
> {
    use guatiao_source_remote::transport::tls::{Identity, PeerId, TlsConfig};
    if server.options().trust() != Trust::Secure {
        return Err(format!(
            "{scheme}:// serves the secure level: pass --trust secure"
        ));
    }
    let mut config = match (&a.state_dir, &a.identity) {
        (Some(dir), None) => TlsConfig::open(dir).map_err(|e| e.to_string())?,
        (None, Some(path)) => TlsConfig::new(Identity::open_or_create(path).map_err(|e| {
            let under = core::error::Error::source(&e)
                .map(|u| format!(": {u}"))
                .unwrap_or_default();
            format!("{}: {e}{under}", path.display())
        })?),
        (Some(_), Some(_)) => {
            return Err("name --state-dir or --identity, not both".to_string());
        }
        (None, None) => {
            return Err(format!(
                "{scheme}:// needs an identity: --state-dir DIR or --identity PATH"
            ));
        }
    };
    let pins = a
        .pins
        .iter()
        .map(|p| {
            p.parse::<PeerId>()
                .map_err(|_| format!("--pin `{p}` is not a fingerprint"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    config = config.pins(pins);
    let mut policy = match &a.policy {
        Some(path) => guatiao_source_remote::config::policy_of(&document(path)?)
            .map_err(|e| format!("{}: {}", path.display(), e.message()))?,
        None => guatiao_source_remote::transport::tls::Policy::strict(),
    };
    if let Some(acl) = named_acl(a)? {
        if policy.acl.is_some() {
            return Err(
                "the policy names an access control list: leave out --acl and --range".to_string(),
            );
        }
        policy.acl = Some(acl);
    }
    let said = policy
        .acl
        .as_ref()
        .map_or_else(|| "every source".to_string(), listing);
    config = config.policy(policy);
    let trusting = if !a.pins.is_empty() {
        format!("{} pinned clients", a.pins.len())
    } else {
        "the trust store's clients".to_string()
    };
    Ok((config, trusting, said))
}

/// The line a secure server starts with, then each setting of its policy
/// looser than strict.
#[cfg(feature = "tls")]
fn say_serving(
    url: &str,
    config: &guatiao_source_remote::transport::tls::TlsConfig,
    trusting: &str,
    said: &str,
) {
    eprintln!(
        "guatiao-remote-server: serving {url} as {}, trusting {trusting}, admitting {said}",
        config.fingerprint(),
    );
    for w in config.weakened() {
        eprintln!(
            "guatiao-remote-server: weaker than strict: {} = {} (strict: {})",
            w.setting, w.value, w.strict
        );
    }
}

#[cfg(feature = "tls")]
fn serve_tls(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::tls::TlsListen;
    let (config, trusting, said) = secure_config(&front.server, "tls", a)?;
    let listener = TlsListen::bind(address, &config).map_err(|e| e.to_string())?;
    let at = listener.local_addr().map_err(|e| e.to_string())?;
    say_serving(&format!("tls://{at}"), &config, &trusting, &said);
    serve_accepted(front, || listener.accept())
}

#[cfg(feature = "quic")]
fn serve_quic(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::quic::QuicListen;
    let (config, trusting, said) = secure_config(&front.server, "quic", a)?;
    let listener = QuicListen::bind(address, &config).map_err(|e| e.to_string())?;
    let at = listener.local_addr().map_err(|e| e.to_string())?;
    say_serving(&format!("quic://{at}"), &config, &trusting, &said);
    serve_accepted(front, || listener.accept())
}

#[cfg(feature = "rdma")]
fn serve_rdma(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::rdma::{RdmaListen, RdmaPort};
    let acl = trusted_acl(&front.server, "rdma", a)?;
    let said = listing(&acl);
    let port = a
        .rdma_device
        .as_deref()
        .map_or_else(RdmaPort::first, RdmaPort::named);
    let listener = RdmaListen::bind(address, acl)
        .map_err(|e| e.to_string())?
        .port(port);
    eprintln!(
        "guatiao-remote-server: serving rdma://{} admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(front, || listener.accept())
}

#[cfg(feature = "local")]
fn serve_local(front: &alloc::sync::Arc<Front>, name: &str) -> Result<(), String> {
    use guatiao_source_remote::transport::local::LocalListen;
    if front.server.options().trust() != Trust::Local {
        return Err("local:// serves the local level".to_string());
    }
    let listener = LocalListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving local://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => front.take(conn),
            Err(e) => eprintln!("guatiao-remote-server: accept: {e}"),
        }
    }
}

#[cfg(feature = "shm")]
fn serve_shm(front: &alloc::sync::Arc<Front>, name: &str) -> Result<(), String> {
    use guatiao_source_remote::transport::shm::ShmListen;
    if front.server.options().trust() != Trust::Local {
        return Err("shm:// serves the local level".to_string());
    }
    let listener = ShmListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving shm://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => front.take(conn),
            Err(e) => eprintln!("guatiao-remote-server: accept: {e}"),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("guatiao-remote-server: {why}");
            ExitCode::FAILURE
        }
    }
}
