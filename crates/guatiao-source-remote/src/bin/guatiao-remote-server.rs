// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao-remote-server`: a registry's providers, served to hosts.

#![forbid(unsafe_code)]

extern crate alloc;

use core::time::Duration;
use std::path::PathBuf;
use std::process::ExitCode;

use guatiao::library::{ImportSearch, Registry};
use guatiao::{Alloc, Map, Value};
use guatiao_source_remote::config::{Security, options_of, security_of};
use guatiao_source_remote::{Kinds, LinkOptions, Server, Trust};

#[cfg(any(feature = "tcp", feature = "local"))]
use front::Front;

/// Connections in their handshake at once, unless `--max-handshakes` says.
const MAX_HANDSHAKES: usize = 64;

const USAGE: &str = "\
guatiao-remote-server (--stdio | --listen URL) [options]
guatiao-remote-server --state-dir DIR (--waiting | --accept FINGERPRINT)

  --stdio               serve one link on stdin and stdout (a host's child)
  --listen URL          serve every link to URL: tcp://ADDRESS:PORT (TCP;
                        tls://ADDRESS:PORT the same, TLS required),
                        quic://ADDRESS:PORT (QUIC, UDP), ws://ADDRESS:PORT and
                        wss://ADDRESS:PORT (WebSocket, a browser's, without
                        and with TLS), https://ADDRESS:PORT (WebTransport,
                        HTTP/3 over QUIC, a browser's), local://name (a
                        socket path or a pipe name), shm://name,
                        rdma://ADDRESS:PORT (RDMA, experimental, Linux only)
  --security MODE       none | server-tls | mutual-tls: none on --stdio,
                        local:// and shm:// (the only one they run), mutual-tls
                        on the network transports unless told; ws:// and
                        rdma:// run none alone, wss:// and tls:// TLS alone,
                        quic:// and https:// any; server-tls proves clients
                        by --key-file or --token-file
  --range CIDR          admit peers from this address range (repeatable): a
                        permit rule of the access control list, which denies
                        what no rule permits
  --acl PATH            the access control list, a TOML or JSON file
                        ([[rule]] action, source, destination, port,
                        fingerprint; default); none needs it or --range
  --key-file PATH       the pre-shared key clients prove (none, server-tls)
  --token-file PATH     the token every client presents (server-tls,
                        mutual-tls): the file's text, a last line end left out
  --state-dir DIR       TLS: this server's identity (DIR/identity.pem, made
                        when absent) and its trust store (DIR/peers): unless
                        --pin is given, clients it does not know wait there
                        until accepted
  --identity PATH       TLS: the identity file instead (made when absent);
                        https:// presents certificates it makes itself, a
                        new one every six days, which a page pins by hash,
                        unless this or --state-dir names one
  --hashes-file PATH    https://: write the hashes a page pins (a JSON list
                        of sha256:<hex>, the certificate presented and the
                        next) here, again at every new certificate
  --pin FINGERPRINT     TLS: trust this client (repeatable); with any, the
                        trust store no longer decides
  --policy PATH         TLS: the TLS policy, a TOML or JSON file (default
                        modern, TLS 1.3 only); its [acl] is the access
                        control list
  --waiting             list the clients waiting in --state-dir, then exit
  --accept FINGERPRINT  accept a waiting client in --state-dir, then exit
  --rdma-device NAME    the RDMA device rdma:// goes through (default: the first)
  --kinds PATH          a kind description document (repeatable): the
                        descriptions served for the kinds it names, needed
                        for providers that describe none themselves
  --load PATH           a library to load (repeatable)
  --scan DIR            a directory of libraries to load (repeatable)
  --source PATH[=JSON]  a source file and its configuration (repeatable)
  --import-search WORD  library | path: where Windows finds what a loaded
                        file imports; path adds the current directory and
                        PATH, as the Python bridge's python3.dll needs
                        (default library: beside the file, beside this
                        program, the system directory)
  --refresh-ms N        refresh the sources every N milliseconds
  --max-calls N         calls one link may have running at once
  --max-frame N         the largest frame read, in bytes
  --max-handshakes N    connections in their handshake at once (default 64);
                        past it a new connection is closed unread";

struct Args {
    stdio: bool,
    listen: Option<String>,
    security: Option<Security>,
    ranges: Vec<String>,
    acl: Option<PathBuf>,
    key_file: Option<PathBuf>,
    token_file: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    identity: Option<PathBuf>,
    hashes_file: Option<PathBuf>,
    pins: Vec<String>,
    policy: Option<PathBuf>,
    waiting: bool,
    accept: Option<String>,
    rdma_device: Option<String>,
    kinds: Vec<PathBuf>,
    load: Vec<PathBuf>,
    scan: Vec<PathBuf>,
    sources: Vec<(PathBuf, Option<String>)>,
    import_search: ImportSearch,
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
        security: None,
        ranges: Vec::new(),
        acl: None,
        key_file: None,
        token_file: None,
        state_dir: None,
        identity: None,
        hashes_file: None,
        pins: Vec::new(),
        policy: None,
        waiting: false,
        accept: None,
        rdma_device: None,
        kinds: Vec::new(),
        load: Vec::new(),
        scan: Vec::new(),
        sources: Vec::new(),
        import_search: ImportSearch::Library,
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
            "--security" => {
                a.security =
                    Some(security_of(&need(&mut it, &flag)?).map_err(|e| e.message().to_string())?);
            }
            "--range" => a.ranges.push(need(&mut it, &flag)?),
            "--acl" => a.acl = Some(need(&mut it, &flag)?.into()),
            "--key-file" => a.key_file = Some(need(&mut it, &flag)?.into()),
            "--token-file" => a.token_file = Some(need(&mut it, &flag)?.into()),
            "--state-dir" => a.state_dir = Some(need(&mut it, &flag)?.into()),
            "--identity" => a.identity = Some(need(&mut it, &flag)?.into()),
            "--hashes-file" => a.hashes_file = Some(need(&mut it, &flag)?.into()),
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
            "--import-search" => {
                a.import_search = match need(&mut it, &flag)?.as_str() {
                    "library" => ImportSearch::Library,
                    "path" => ImportSearch::Path,
                    other => return Err(format!("{flag} is library or path, not `{other}`")),
                };
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

fn registry(a: &Args) -> Result<(Registry, Kinds), String> {
    let mut kinds = Kinds::new();
    for path in &a.kinds {
        // The refusal names the file.
        kinds.load(path).map_err(|e| e.to_string())?;
    }
    let mut registry = Registry::new("guatiao-remote-server", env!("CARGO_PKG_VERSION"));
    registry.set_import_search(a.import_search);
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
    Ok((registry, kinds))
}

/// `path` as a configuration's text.
fn path_text(path: &std::path::Path) -> String {
    path.display().to_string()
}

/// The link options the flags name, read as a source's configuration
/// reads its `key_file`, `token_file`, `max_calls` and `max_frame`.
fn options(a: &Args, trust: Trust) -> Result<LinkOptions, String> {
    let mut map = Map::new();
    if let Some(path) = &a.key_file {
        map.set("key_file", path_text(path).as_str());
    }
    if let Some(path) = &a.token_file {
        map.set("token_file", path_text(path).as_str());
    }
    if let Some(n) = a.max_calls {
        map.set("max_calls", n as u64);
    }
    if let Some(n) = a.max_frame {
        map.set("max_frame", n as u64);
    }
    options_of(&map, trust).map_err(|e| e.message().to_string())
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
    let (registry, kinds) = registry(&a)?;
    let scheme = match a.listen.as_deref() {
        Some(url) => url
            .split_once("://")
            .ok_or_else(|| format!("--listen takes scheme://address, not `{url}`"))?
            .0
            .to_string(),
        None => "stdio".to_string(),
    };
    let security = a.security.unwrap_or_else(|| Security::default_for(&scheme));
    let local = matches!(scheme.as_str(), "stdio" | "local" | "shm");
    if local && security != Security::None {
        return Err(format!(
            "{scheme} runs on this machine with no TLS, and --security is {}: leave it out",
            security.name()
        ));
    }
    match (scheme.as_str(), security) {
        ("ws" | "rdma", Security::ServerTls | Security::MutualTls) => {
            return Err(format!(
                "{scheme}:// carries no TLS, and the link is {}: pass --security none{}",
                security.name(),
                if scheme == "ws" {
                    ", or listen on wss://"
                } else {
                    ""
                }
            ));
        }
        ("wss" | "tls", Security::None) => {
            return Err(format!(
                "{scheme}:// runs TLS: pass --security mutual-tls, or listen on {}://",
                if scheme == "wss" { "ws" } else { "tcp" }
            ));
        }
        (_, Security::ServerTls) if a.key_file.is_none() && a.token_file.is_none() => {
            return Err(
                "server TLS proves clients by the key or a token: pass --key-file or --token-file"
                    .to_string(),
            );
        }
        (_, Security::None) if a.token_file.is_some() => {
            return Err(
                "--token-file is for server-tls and mutual-tls: with no TLS a token adds nothing to the key's proof and may cross in the clear; prove clients by --key-file"
                    .to_string(),
            );
        }
        _ => {}
    }
    let server = Server::new(registry, kinds, options(&a, security.trust(local))?);
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
    let tls = security != Security::None;
    #[cfg(not(feature = "tls"))]
    if tls {
        return Err(format!(
            "{} is not in this build (guatiao-source-remote's `tls` feature)",
            security.name()
        ));
    }
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" if !tls => serve_tcp(&front, address, &a),
        #[cfg(feature = "ws")]
        "ws" => serve_ws(&front, address, &a),
        #[cfg(feature = "tls")]
        "tcp" | "tls" => serve_tls(&front, address, &a),
        #[cfg(all(feature = "ws", feature = "tls"))]
        "wss" => serve_wss(&front, address, &a),
        #[cfg(feature = "quic")]
        "quic" if !tls => serve_quic_trusted(&front, address, &a),
        #[cfg(feature = "quic")]
        "quic" => serve_quic(&front, address, &a),
        #[cfg(feature = "webtransport")]
        "https" => serve_wt(&front, address, &a),
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

    /// The first pause after a failed accept; each failure in a row doubles
    /// it, up to [`MOST_PAUSE`].
    const FIRST_PAUSE: Duration = Duration::from_millis(5);

    /// The longest pause between failed accepts.
    const MOST_PAUSE: Duration = Duration::from_secs(1);

    /// Accepts failing in a row (out of descriptors, say): reported once per
    /// interval, each followed by a pause that grows while they keep failing.
    struct Failing {
        throttle: Throttle,
        pause: Duration,
    }

    impl Failing {
        fn new(every: Duration) -> Failing {
            Failing {
                throttle: Throttle::new(every),
                pause: Duration::ZERO,
            }
        }

        /// Whether to report this failure (with how many went unreported
        /// since), and how long to pause before accepting again.
        fn failed(&mut self, now: Instant) -> (Option<u64>, Duration) {
            self.pause = (self.pause * 2).clamp(FIRST_PAUSE, MOST_PAUSE);
            (self.throttle.pass("accept", now), self.pause)
        }

        fn succeeded(&mut self) {
            self.pause = Duration::ZERO;
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
        /// Connections past `max`, and threads that would not start: apart,
        /// so many sources cannot silence them.
        crowded: Mutex<Throttle>,
        /// The listener's own failures.
        failing: Mutex<Failing>,
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
                failing: Mutex::new(Failing::new(REPORT_EVERY)),
            })
        }

        /// After an accept failed: reports it sparingly, then pauses, longer
        /// while accepts keep failing.
        pub(super) fn accept_failed(&self, e: &std::io::Error) {
            let (report, pause) = self
                .failing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .failed(Instant::now());
            if let Some(quiet) = report {
                eprintln!("guatiao-remote-server: accept: {e}{}", since(quiet));
            }
            std::thread::sleep(pause);
        }

        /// After an accept succeeded: the next failure pauses briefly again.
        pub(super) fn accepted(&self) {
            self.failing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .succeeded();
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
                self.say(&self.crowded, "threads", |quiet| {
                    format!("no thread for a connection: {e}{}", since(quiet))
                });
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
        fn accepts_failing_in_a_row_are_reported_once_and_pause_longer_each_time() {
            let mut f = Failing::new(Duration::from_secs(60));
            let start = Instant::now();
            let (report, pause) = f.failed(start);
            assert_eq!((report, pause), (Some(0), FIRST_PAUSE));
            let mut reports = 0;
            let mut last = pause;
            for i in 1..1000 {
                let (report, pause) = f.failed(start + Duration::from_millis(i));
                reports += usize::from(report.is_some());
                assert!(pause >= last && pause <= MOST_PAUSE, "{pause:?}");
                last = pause;
            }
            assert_eq!(reports, 0, "reported once within the interval");
            assert_eq!(last, MOST_PAUSE);
            assert_eq!(
                f.failed(start + Duration::from_secs(60)).0,
                Some(999),
                "the next report counts what was held back"
            );
            f.succeeded();
            assert_eq!(f.failed(start + Duration::from_secs(61)).1, FIRST_PAUSE);
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
            "{scheme}:// with no TLS serves the trusted level: pass --security none"
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
            Ok(accepted) => {
                front.accepted();
                match accepted {
                    Accepted::Connection(conn) => front.take(conn),
                    Accepted::Refused(peer) => front.denied(peer),
                    _ => {}
                }
            }
            Err(e) => front.accept_failed(&e),
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
    use guatiao_source_remote::config::{policy_of, tls_config_of};
    use guatiao_source_remote::transport::tls::{Identity, PeerId, Policy, TlsConfig};
    let mutual = match server.options().trust() {
        Trust::Secure => true,
        Trust::ServerTls => false,
        _ => {
            return Err(format!(
                "{scheme}:// with TLS serves mutual TLS or server TLS: pass --security mutual-tls or server-tls"
            ));
        }
    };
    if !mutual && !a.pins.is_empty() {
        return Err("server TLS asks clients for no certificate: leave out --pin".to_string());
    }
    let mut config = match (&a.state_dir, &a.identity) {
        (Some(_), Some(_)) => {
            return Err("name --state-dir or --identity, not both".to_string());
        }
        // It presents certificates it makes: this one is never shown.
        (None, None) if scheme == "https" => {
            TlsConfig::new(Identity::generate().map_err(|e| e.to_string())?)
        }
        (None, None) => {
            return Err(format!(
                "{scheme}:// needs an identity: --state-dir DIR or --identity PATH"
            ));
        }
        // Read as a source's configuration reads `state_dir` or
        // `identity_file`.
        (dir, file) => {
            let mut map = Map::new();
            if let Some(dir) = dir {
                map.set("state_dir", path_text(dir).as_str());
            }
            if let Some(file) = file {
                map.set("identity_file", path_text(file).as_str());
            }
            tls_config_of(&map, Security::MutualTls).map_err(|e| e.message().to_string())?
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
        Some(path) => policy_of(&document(path)?)
            .map_err(|e| format!("{}: {}", path.display(), e.message()))?,
        None => Policy::modern(),
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
    let trusting = if !mutual {
        config = config.server_tls();
        match (&a.key_file, &a.token_file) {
            (Some(_), Some(_)) => "clients by the key and a token (server TLS)",
            (None, Some(_)) => "clients by a token (server TLS)",
            _ => "clients by the key (server TLS)",
        }
        .to_string()
    } else if !a.pins.is_empty() {
        format!("{} pinned clients", a.pins.len())
    } else {
        "the trust store's clients".to_string()
    };
    let trusting = if mutual && a.token_file.is_some() {
        format!("{trusting} presenting a token")
    } else {
        trusting
    };
    Ok((config, trusting, said))
}

/// The line a secure server starts with, then each setting of its policy
/// looser than the modern one.
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
            "guatiao-remote-server: weaker than modern: {} = {} (modern: {})",
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

#[cfg(feature = "quic")]
fn serve_quic_trusted(
    front: &alloc::sync::Arc<Front>,
    address: &str,
    a: &Args,
) -> Result<(), String> {
    use guatiao_source_remote::transport::quic::QuicListen;
    let acl = trusted_acl(&front.server, "quic", a)?;
    let said = listing(&acl);
    let listener = QuicListen::bind_trusted(address, acl).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving quic://{} with no certificate checked, admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(front, || listener.accept())
}

/// WebTransport at the security `--security` names. Unless `--identity`
/// or `--state-dir` names a certificate, it presents ones it makes, a new
/// one every
/// [`ROTATE_EVERY`](guatiao_source_remote::transport::webtransport::ROTATE_EVERY),
/// and says (and writes to `--hashes-file`) the hashes a page pins.
#[cfg(feature = "webtransport")]
fn serve_wt(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::tls::Identity;
    use guatiao_source_remote::transport::webtransport::{ROTATE_EVERY, WtListen};
    let trusted = front.server.options().trust() == Trust::Trusted;
    let listener = if trusted {
        let acl = trusted_acl(&front.server, "https", a)?;
        let said = listing(&acl);
        let listener = WtListen::bind_trusted(address, acl).map_err(|e| e.to_string())?;
        eprintln!(
            "guatiao-remote-server: serving https://{}/ (WebTransport) with no certificate checked, admitting {said}",
            listener.local_addr().map_err(|e| e.to_string())?,
        );
        listener
    } else {
        let (config, trusting, said) = secure_config(&front.server, "https", a)?;
        let listener = WtListen::bind(address, &config).map_err(|e| e.to_string())?;
        eprintln!(
            "guatiao-remote-server: serving https://{}/ (WebTransport), trusting {trusting}, admitting {said}",
            listener.local_addr().map_err(|e| e.to_string())?,
        );
        for w in config.weakened() {
            eprintln!(
                "guatiao-remote-server: weaker than modern: {} = {} (modern: {})",
                w.setting, w.value, w.strict
            );
        }
        listener
    };
    let listener = match (&a.identity, &a.state_dir) {
        // With TLS the configuration already presents the one named.
        (Some(_), _) | (None, Some(_)) if !trusted => listener,
        (Some(path), _) => {
            let identity =
                Identity::open_or_create(path).map_err(|e| format!("{}: {e}", path.display()))?;
            listener.presenting(identity).map_err(|e| e.to_string())?
        }
        _ => listener.rotating(ROTATE_EVERY).map_err(|e| e.to_string())?,
    };
    let listener = alloc::sync::Arc::new(listener);
    let mut said = Vec::new();
    publish_hashes(&listener, a.hashes_file.as_deref(), &mut said)?;
    let watching = alloc::sync::Arc::clone(&listener);
    let file = a.hashes_file.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(60));
            if let Err(e) = publish_hashes(&watching, file.as_deref(), &mut said) {
                eprintln!("guatiao-remote-server: {e}");
            }
        }
    });
    serve_accepted(front, || listener.accept())
}

/// Says the hashes a page pins when they differ from `said`, and writes
/// them to `file` (whole, through a file beside it).
#[cfg(feature = "webtransport")]
fn publish_hashes(
    listener: &guatiao_source_remote::transport::webtransport::WtListen,
    file: Option<&std::path::Path>,
    said: &mut Vec<String>,
) -> Result<(), String> {
    let hashes: Vec<String> = listener
        .certificate_hashes()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|h| h.canonical())
        .collect();
    if *said == hashes {
        return Ok(());
    }
    if let Some(file) = file {
        let quoted: Vec<String> = hashes.iter().map(|h| format!("\"{h}\"")).collect();
        let fresh = file.with_extension("partial");
        std::fs::write(&fresh, format!("[{}]\n", quoted.join(", ")))
            .and_then(|()| std::fs::rename(&fresh, file))
            .map_err(|e| format!("{}: {e}", file.display()))?;
    }
    // Said once the file holds them, so whoever reads the line finds them.
    eprintln!(
        "guatiao-remote-server: certificate hashes {}",
        hashes.join(" ")
    );
    *said = hashes;
    Ok(())
}

#[cfg(all(feature = "ws", feature = "tls"))]
fn serve_wss(front: &alloc::sync::Arc<Front>, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::ws::WsListen;
    let (config, trusting, said) = secure_config(&front.server, "wss", a)?;
    let listener = WsListen::bind_tls(address, &config).map_err(|e| e.to_string())?;
    let at = listener.local_addr().map_err(|e| e.to_string())?;
    say_serving(&format!("wss://{at}"), &config, &trusting, &said);
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
        return Err("local:// runs with no TLS: leave --security out".to_string());
    }
    let listener = LocalListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving local://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                front.accepted();
                front.take(conn);
            }
            Err(e) => front.accept_failed(&e),
        }
    }
}

#[cfg(feature = "shm")]
fn serve_shm(front: &alloc::sync::Arc<Front>, name: &str) -> Result<(), String> {
    use guatiao_source_remote::transport::shm::ShmListen;
    if front.server.options().trust() != Trust::Local {
        return Err("shm:// runs with no TLS: leave --security out".to_string());
    }
    let listener = ShmListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving shm://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                front.accepted();
                front.take(conn);
            }
            Err(e) => front.accept_failed(&e),
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
