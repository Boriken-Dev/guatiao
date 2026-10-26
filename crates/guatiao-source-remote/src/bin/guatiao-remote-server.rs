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

const USAGE: &str = "\
guatiao-remote-server (--stdio | --listen URL) [options]
guatiao-remote-server --state-dir DIR (--waiting | --accept FINGERPRINT)

  --stdio               serve one link on stdin and stdout (a host's child)
  --listen URL          serve every link to URL: tcp://ADDRESS:PORT is the
                        trusted level, ws://ADDRESS:PORT the same over
                        WebSocket (a browser), tls://ADDRESS:PORT the secure
                        level, local://name the local one (a socket path or a
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
                        when absent) and its trust store (DIR/peers): clients
                        it does not know wait there until accepted
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
  --max-frame N         the largest frame read, in bytes";

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
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" => serve_tcp(&server, address, &a),
        #[cfg(feature = "ws")]
        "ws" => serve_ws(&server, address, &a),
        #[cfg(feature = "tls")]
        "tls" => serve_tls(&server, address, &a),
        #[cfg(feature = "rdma")]
        "rdma" => serve_rdma(&server, address, &a),
        #[cfg(feature = "local")]
        "local" => serve_local(&server, address),
        #[cfg(feature = "shm")]
        "shm" => serve_shm(&server, address),
        other => Err(format!("`{other}://` is not a transport this build has")),
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

/// Serves every link `accept` hands over, each on a thread of its own.
#[cfg(feature = "tcp")]
fn serve_accepted(
    server: &Server,
    accept: impl Fn() -> std::io::Result<guatiao_source_remote::transport::tcp::Accepted>,
) -> Result<(), String> {
    use guatiao_source_remote::transport::tcp::Accepted;
    let server = alloc::sync::Arc::new(server.clone());
    loop {
        match accept() {
            Ok(Accepted::Connection(conn)) => {
                let server = alloc::sync::Arc::clone(&server);
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote-server: a link was refused: {e}");
                    }
                });
            }
            Ok(Accepted::Refused(peer)) => {
                eprintln!("guatiao-remote-server: the access control list denies {peer}");
            }
            Ok(_) => {}
            Err(e) => eprintln!("guatiao-remote-server: accept: {e}"),
        }
    }
}

#[cfg(feature = "tcp")]
fn serve_tcp(server: &Server, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::tcp::TcpListen;
    let acl = trusted_acl(server, "tcp", a)?;
    let said = listing(&acl);
    let listener = TcpListen::bind(address, acl).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving tcp://{} admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(server, || listener.accept())
}

#[cfg(feature = "ws")]
fn serve_ws(server: &Server, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::ws::WsListen;
    let acl = trusted_acl(server, "ws", a)?;
    let said = listing(&acl);
    let listener = WsListen::bind(address, acl).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving ws://{} admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
    );
    serve_accepted(server, || listener.accept())
}

#[cfg(feature = "tls")]
fn serve_tls(server: &Server, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::tls::{Identity, PeerId, TlsConfig, TlsListen};
    if server.options().trust() != Trust::Secure {
        return Err("tls:// serves the secure level: pass --trust secure".to_string());
    }
    let mut config = match (&a.state_dir, &a.identity) {
        (Some(dir), None) => TlsConfig::open(dir).map_err(|e| e.to_string())?,
        (None, Some(path)) => TlsConfig::new(
            Identity::open_or_create(path).map_err(|e| format!("{}: {e}", path.display()))?,
        ),
        (Some(_), Some(_)) => {
            return Err("name --state-dir or --identity, not both".to_string());
        }
        (None, None) => {
            return Err("tls:// needs an identity: --state-dir DIR or --identity PATH".to_string());
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
    let listener = TlsListen::bind(address, &config).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote-server: serving tls://{} as {}, trusting {trusting}, admitting {said}",
        listener.local_addr().map_err(|e| e.to_string())?,
        config.fingerprint(),
    );
    for w in config.weakened() {
        eprintln!(
            "guatiao-remote-server: weaker than strict: {} = {} (strict: {})",
            w.setting, w.value, w.strict
        );
    }
    serve_accepted(server, || listener.accept())
}

#[cfg(feature = "rdma")]
fn serve_rdma(server: &Server, address: &str, a: &Args) -> Result<(), String> {
    use guatiao_source_remote::transport::rdma::{RdmaListen, RdmaPort};
    let acl = trusted_acl(server, "rdma", a)?;
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
    serve_accepted(server, || listener.accept())
}

#[cfg(feature = "local")]
fn serve_local(server: &Server, name: &str) -> Result<(), String> {
    use guatiao_source_remote::transport::local::LocalListen;
    if server.options().trust() != Trust::Local {
        return Err("local:// serves the local level".to_string());
    }
    let listener = LocalListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving local://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                let server = server.clone();
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote-server: a link was refused: {e}");
                    }
                });
            }
            Err(e) => eprintln!("guatiao-remote-server: accept: {e}"),
        }
    }
}

#[cfg(feature = "shm")]
fn serve_shm(server: &Server, name: &str) -> Result<(), String> {
    use guatiao_source_remote::transport::shm::ShmListen;
    if server.options().trust() != Trust::Local {
        return Err("shm:// serves the local level".to_string());
    }
    let listener = ShmListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote-server: serving shm://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                let server = server.clone();
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote-server: a link was refused: {e}");
                    }
                });
            }
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
