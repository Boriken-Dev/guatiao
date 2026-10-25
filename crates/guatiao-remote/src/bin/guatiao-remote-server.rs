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
use guatiao_remote::config::trust_of;
use guatiao_remote::{Described, LinkOptions, Server, Trust};

const USAGE: &str = "\
guatiao-remote-server (--stdio | --listen tcp://ADDRESS:PORT) [options]

  --stdio               serve one link on stdin and stdout (a host's child)
  --listen URL          serve every link to URL (tcp:// is the trusted level,
                        local://name the local one: a socket path or a pipe name)
  --trust LEVEL         local | trusted | secure (default: local on --stdio)
  --range CIDR          an address range peers are accepted from, others
                        closed unread (trusted; repeatable; required)
  --key-file PATH       the trusted level's pre-shared key
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
    key_file: Option<PathBuf>,
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
        key_file: None,
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
            "--key-file" => a.key_file = Some(need(&mut it, &flag)?.into()),
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
    if a.stdio == a.listen.is_some() {
        return Err(format!("name one of --stdio and --listen\n\n{USAGE}"));
    }
    Ok(a)
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
    let mut registry = Registry::new("guatiao-remote", env!("CARGO_PKG_VERSION"));
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

fn run() -> Result<(), String> {
    let a = parse()?;
    let (registry, described) = registry(&a)?;
    let trust = a.trust.unwrap_or(Trust::Local);
    let server = Server::new(registry, described, options(&a, trust)?);
    let refreshed = server.refresh();
    for r in &refreshed.refused {
        eprintln!("guatiao-remote: {}: {}", r.source, r.reason);
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
            .serve(guatiao_remote::transport::child::stdio())
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
        "tcp" => serve_tcp(&server, address, &a.ranges),
        #[cfg(feature = "local")]
        "local" => serve_local(&server, address),
        #[cfg(feature = "shm")]
        "shm" => serve_shm(&server, address),
        other => Err(format!("`{other}://` is not a transport this build has")),
    }
}

#[cfg(feature = "tcp")]
fn serve_tcp(server: &Server, address: &str, ranges: &[String]) -> Result<(), String> {
    use guatiao_remote::transport::tcp::{Accepted, Range, TcpListen};
    if server.options().trust() != Trust::Trusted {
        return Err("tcp:// serves the trusted level: pass --trust trusted".to_string());
    }
    let ranges = ranges
        .iter()
        .map(|r| Range::parse(r))
        .collect::<Result<Vec<_>, _>>()?;
    let listener = TcpListen::bind(address, &ranges).map_err(|e| e.to_string())?;
    eprintln!(
        "guatiao-remote: serving tcp://{} to {}",
        listener.local_addr().map_err(|e| e.to_string())?,
        ranges
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    let server = alloc::sync::Arc::new(server.clone());
    loop {
        match listener.accept() {
            Ok(Accepted::Connection(conn)) => {
                let server = alloc::sync::Arc::clone(&server);
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote: a link was refused: {e}");
                    }
                });
            }
            Ok(Accepted::Refused(peer)) => {
                eprintln!("guatiao-remote: {peer} is outside the configured ranges");
            }
            Ok(_) => {}
            Err(e) => eprintln!("guatiao-remote: accept: {e}"),
        }
    }
}

#[cfg(feature = "local")]
fn serve_local(server: &Server, name: &str) -> Result<(), String> {
    use guatiao_remote::transport::local::LocalListen;
    if server.options().trust() != Trust::Local {
        return Err("local:// serves the local level".to_string());
    }
    let listener = LocalListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote: serving local://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                let server = server.clone();
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote: a link was refused: {e}");
                    }
                });
            }
            Err(e) => eprintln!("guatiao-remote: accept: {e}"),
        }
    }
}

#[cfg(feature = "shm")]
fn serve_shm(server: &Server, name: &str) -> Result<(), String> {
    use guatiao_remote::transport::shm::ShmListen;
    if server.options().trust() != Trust::Local {
        return Err("shm:// serves the local level".to_string());
    }
    let listener = ShmListen::bind(name).map_err(|e| format!("{name}: {e}"))?;
    eprintln!("guatiao-remote: serving shm://{name}");
    loop {
        match listener.accept() {
            Ok(conn) => {
                let server = server.clone();
                std::thread::spawn(move || {
                    if let Err(e) = server.serve(conn) {
                        eprintln!("guatiao-remote: a link was refused: {e}");
                    }
                });
            }
            Err(e) => eprintln!("guatiao-remote: accept: {e}"),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("guatiao-remote: {why}");
            ExitCode::FAILURE
        }
    }
}
