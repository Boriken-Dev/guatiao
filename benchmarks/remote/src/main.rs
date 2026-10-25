// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! guatiao-source-remote measured across a link.
//!
//! ```text
//! guatiao-remote-bench serve URL [options]   a server of a provider made for measuring
//! guatiao-remote-bench run URL [options]     five metrics against it, one JSON line
//! ```
//!
//! URL is `tcp://`, `local://`, `shm://` or, built with the `rdma`
//! feature, `rdma://` (the address of its TCP set-up). Options:
//! `--key TEXT` (tcp, rdma),
//! `--mss N` (tcp, Unix: caps the segment size, as on a path without
//! jumbo frames), `--spin MICROSECONDS` (how long a waiting thread polls before it
//! sleeps), `--label TEXT`, `--seconds N`, `--out FILE` (appends the line).
//!
//! The metrics: a small call's p50 and p99; calls a second with 32
//! threads calling at once; the throughput of a 4 MiB argument answered
//! back; protocol bytes per small call (frames, counted at the link) and,
//! for TCP on Linux, bytes on the wire from the kernel's own counters;
//! process CPU per small call on each end.

extern crate alloc;

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::time::Duration;
use std::io;
use std::net::{TcpListener, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use greeter_kind::{Counter, Greeter, Wide};
use guatiao::library::{ProviderError, Registry};
use guatiao::{Map, Status};
use guatiao_source_remote::transport::{self, Connect, Connection, FrameRead, FrameWrite};
use guatiao_source_remote::{Described, LinkOptions, RemoteSource, Server, Trust};

/// Process CPU time, user and system, in nanoseconds.
fn cpu_ns() -> u64 {
    #[cfg(unix)]
    {
        // SAFETY: getrusage writes the struct it is given.
        let mut u: libc::rusage = unsafe { core::mem::zeroed() };
        // SAFETY: as above.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let t = |tv: libc::timeval| tv.tv_sec as u64 * 1_000_000_000 + tv.tv_usec as u64 * 1000;
        t(u.ru_utime) + t(u.ru_stime)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let z = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut a, mut b, mut k, mut u) = (z, z, z, z);
        // SAFETY: GetProcessTimes writes the four structs it is given.
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) };
        let f =
            |t: FILETIME| (u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)) * 100;
        f(k) + f(u)
    }
}

/// The provider measured: an echo, a counter, and a way to read the
/// server's CPU time (`echo_u64(0)`) or end it (`echo_u64(1)`).
#[derive(Default, guatiao::Provider)]
#[provider(Greeter, Counter, Wide)]
pub struct Bench {
    n: Mutex<i64>,
}

impl Greeter for Bench {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        let mut m = Map::new();
        m.set("greeting", name);
        Ok(m)
    }
}

impl Counter for Bench {
    fn count(&self) -> i64 {
        let mut n = self.n.lock().unwrap_or_else(|e| e.into_inner());
        *n += 1;
        *n
    }
}

impl Wide for Bench {
    fn echo_i64(&self, x: i64) -> Result<i64, ProviderError> {
        Ok(x)
    }

    fn echo_u64(&self, x: u64) -> Result<u64, ProviderError> {
        match x {
            0 => Ok(cpu_ns()),
            1 => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(200));
                    std::process::exit(0);
                });
                Ok(1)
            }
            _ => Err(ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "0 or 1")),
        }
    }
}

guatiao::local_providers!(id = "bench", version = "1.0.0", providers = [Bench]);

struct Args {
    url: String,
    key: Option<String>,
    mss: Option<u32>,
    spin: Option<u64>,
    label: String,
    seconds: u64,
    out: Option<PathBuf>,
}

fn parse() -> Result<(String, Args), String> {
    let mut it = std::env::args().skip(1);
    let mode = it.next().ok_or("serve or run")?;
    let url = it.next().ok_or("a URL")?;
    let mut a = Args {
        url,
        key: None,
        mss: None,
        spin: None,
        label: String::new(),
        seconds: 3,
        out: None,
    };
    while let Some(flag) = it.next() {
        let v = it.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--key" => a.key = Some(v),
            "--mss" => a.mss = Some(v.parse().map_err(|_| "--mss")?),
            "--spin" => a.spin = Some(v.parse().map_err(|_| "--spin")?),
            "--label" => a.label = v,
            "--seconds" => a.seconds = v.parse().map_err(|_| "--seconds")?,
            "--out" => a.out = Some(v.into()),
            other => return Err(format!("`{other}` is not an option")),
        }
    }
    Ok((mode, a))
}

fn split(url: &str) -> Result<(&str, &str), String> {
    url.split_once("://")
        .ok_or_else(|| format!("`{url}` is not scheme://address"))
}

fn options(scheme: &str, a: &Args) -> LinkOptions {
    let trust = match scheme {
        "tcp" | "rdma" => Trust::Trusted,
        _ => Trust::Local,
    };
    let mut o = LinkOptions::new(trust);
    if let Some(k) = &a.key {
        o = o.key(k.as_bytes());
    }
    if let Some(us) = a.spin {
        o = o.spin(Duration::from_micros(us));
    }
    o
}

fn socket_for(addr: SocketAddr, mss: Option<u32>) -> io::Result<socket2::Socket> {
    let s = socket2::Socket::new(
        socket2::Domain::for_address(addr),
        socket2::Type::STREAM,
        None,
    )?;
    #[cfg(unix)]
    if let Some(mss) = mss {
        s.set_mss(mss)?;
    }
    #[cfg(not(unix))]
    let _ = mss;
    Ok(s)
}

fn tcp_listener(address: &str, mss: Option<u32>) -> io::Result<TcpListener> {
    let addr = address
        .to_socket_addrs()?
        .next()
        .ok_or(io::ErrorKind::NotFound)?;
    let s = socket_for(addr, mss)?;
    s.set_reuse_address(true)?;
    s.bind(&addr.into())?;
    s.listen(64)?;
    Ok(s.into())
}

fn serve(a: &Args) -> Result<(), String> {
    let (scheme, address) = split(&a.url)?;
    let mut registry = Registry::new("bench-server", "1.0");
    registry
        .register_local("bench", library)
        .map_err(|e| e.to_string())?;
    let mut described = Described::new();
    for d in [
        greeter_kind::GreeterVtable::description(),
        greeter_kind::CounterVtable::description(),
        greeter_kind::WideVtable::description(),
        greeter_kind::ListenerVtable::description(),
        greeter_kind::ConversationVtable::description(),
    ] {
        described
            .add_json(&d)
            .map_err(|e| e.message().to_string())?;
    }
    let server = Server::new(registry, described, options(scheme, a));
    let serve_one = |conn: io::Result<Connection>| {
        let server = server.clone();
        std::thread::spawn(move || match conn {
            Ok(c) => {
                if let Err(e) = server.serve(c) {
                    eprintln!("refused: {e}");
                }
            }
            Err(e) => eprintln!("handshake: {e}"),
        });
    };
    eprintln!("serving {}", a.url);
    match scheme {
        "tcp" => {
            use guatiao_source_remote::transport::tcp::{Accepted, Range, TcpListen};
            let listener = tcp_listener(address, a.mss).map_err(|e| e.to_string())?;
            let ip = listener.local_addr().map_err(|e| e.to_string())?.ip();
            // Every address of the listener's own family: a bench on a
            // network its owner isolates.
            let any = Range::parse(if ip.is_ipv4() { "0.0.0.0/0" } else { "::/0" })?;
            let listen = TcpListen::from_listener(listener, &[any]).map_err(|e| e.to_string())?;
            loop {
                match listen.accept() {
                    Ok(Accepted::Connection(c)) => serve_one(Ok(c)),
                    Ok(_) => {}
                    Err(e) => eprintln!("accept: {e}"),
                }
            }
        }
        #[cfg(feature = "rdma")]
        "rdma" => {
            use guatiao_source_remote::transport::tcp::{Accepted, Range};
            let listener = tcp_listener(address, None).map_err(|e| e.to_string())?;
            let ip = listener.local_addr().map_err(|e| e.to_string())?.ip();
            let any = Range::parse(if ip.is_ipv4() { "0.0.0.0/0" } else { "::/0" })?;
            let listen = transport::rdma::RdmaListen::from_listener(listener, &[any])
                .map_err(|e| e.to_string())?;
            loop {
                match listen.accept() {
                    Ok(Accepted::Connection(c)) => serve_one(Ok(c)),
                    Ok(_) => {}
                    Err(e) => eprintln!("accept: {e}"),
                }
            }
        }
        "local" => {
            let listen = transport::local::LocalListen::bind(address).map_err(|e| e.to_string())?;
            loop {
                serve_one(listen.accept());
            }
        }
        "shm" => {
            let listen = transport::shm::ShmListen::bind(address).map_err(|e| e.to_string())?;
            loop {
                serve_one(listen.accept());
            }
        }
        other => Err(format!("`{other}://` is not a transport here")),
    }
}

/// Frame bytes through a connection, both ways.
#[derive(Default)]
struct Tally {
    bytes: Mutex<u64>,
}

struct CountRead(Box<dyn FrameRead>, Arc<Tally>);
struct CountWrite(Box<dyn FrameWrite>, Arc<Tally>);

impl FrameRead for CountRead {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let got = self.0.read_frame(buf, max)?;
        if got {
            *self.1.bytes.lock().unwrap() += buf.len() as u64 + 4;
        }
        Ok(got)
    }

    fn set_timeout(&mut self, t: Option<Duration>) -> io::Result<()> {
        self.0.set_timeout(t)
    }
}

impl FrameWrite for CountWrite {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        *self.1.bytes.lock().unwrap() += frame.len() as u64;
        self.0.write_frame(frame)
    }

    fn shutdown(&mut self) {
        self.0.shutdown();
    }
}

/// Wraps another `Connect`, counting what crosses.
struct Counted {
    inner: Box<dyn Connect>,
    tally: Arc<Tally>,
}

impl Connect for Counted {
    fn connect(&self) -> io::Result<Connection> {
        let c = self.inner.connect()?;
        Ok(Connection {
            reader: Box::new(CountRead(c.reader, Arc::clone(&self.tally))),
            writer: Box::new(CountWrite(c.writer, Arc::clone(&self.tally))),
            peer: c.peer,
            closer: c.closer,
        })
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }
}

/// TCP with a capped segment size.
struct CappedTcp {
    address: String,
    mss: Option<u32>,
}

impl Connect for CappedTcp {
    fn connect(&self) -> io::Result<Connection> {
        let addr = self
            .address
            .to_socket_addrs()?
            .next()
            .ok_or(io::ErrorKind::NotFound)?;
        let s = socket_for(addr, self.mss)?;
        s.connect(&addr.into())?;
        transport::tcp::from_stream(s.into())
    }

    fn describe(&self) -> String {
        format!("tcp {}", self.address)
    }
}

fn connector(scheme: &str, address: &str, a: &Args) -> Result<Box<dyn Connect>, String> {
    Ok(match scheme {
        "tcp" => Box::new(CappedTcp {
            address: address.to_string(),
            mss: a.mss,
        }),
        "local" => Box::new(transport::local::LocalConnect::new(address)),
        "shm" => Box::new(transport::shm::ShmConnect::new(address)),
        #[cfg(feature = "rdma")]
        "rdma" => Box::new(transport::rdma::RdmaConnect::new(address)),
        other => return Err(format!("`{other}://` is not a transport here")),
    })
}

/// The kernel's counters for one TCP connection, on Linux: bytes and
/// segments each way. `None` elsewhere.
fn tcp_counters(peer: &str) -> Option<(u64, u64, u64, u64)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let out = std::process::Command::new("ss")
        .args(["-tinH", "dst", peer])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |name: &str| {
        text.split_whitespace()
            .find_map(|w| w.strip_prefix(name))
            .and_then(|v| v.parse::<u64>().ok())
    };
    Some((
        field("bytes_sent:").unwrap_or(0),
        field("bytes_received:").unwrap_or(0),
        field("segs_out:")?,
        field("segs_in:")?,
    ))
}

fn pct(v: &mut [u64], p: f64) -> f64 {
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize] as f64 / 1000.0
}

fn run(a: &Args) -> Result<(), String> {
    let (scheme, address) = split(&a.url)?;
    let tally = Arc::new(Tally::default());
    let source = RemoteSource::new(
        Box::new(Counted {
            inner: connector(scheme, address, a)?,
            tally: Arc::clone(&tally),
        }),
        options(scheme, a),
    );
    let mut registry = Registry::new("bench-host", "1.0");
    registry
        .add_source("bench", source.clone())
        .map_err(|e| e.to_string())?;
    let refreshed = registry.refresh();
    if !refreshed.refused.is_empty() {
        return Err(format!("{:?}", refreshed.refused));
    }
    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .and_then(|o| o.shared())
        .ok_or("no counter")?;
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .and_then(|o| o.shared())
        .ok_or("no greeter")?;
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .and_then(|o| o.shared())
        .ok_or("no wide")?;
    for _ in 0..2000 {
        counter.count();
    }

    // 1, 4 and 5: latency, bytes and CPU of a small call.
    let n = 20_000u64;
    let peer = (scheme == "tcp").then(|| address.to_string());
    let wire0 = peer.as_deref().and_then(tcp_counters);
    let bytes0 = *tally.bytes.lock().unwrap();
    let (cpu0, server0) = (
        cpu_ns(),
        wide.echo_u64(0).map_err(|e| e.message().to_string())?,
    );
    let mut times = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let t = Instant::now();
        counter.count();
        times.push(t.elapsed().as_nanos() as u64);
    }
    let (cpu1, server1) = (
        cpu_ns(),
        wide.echo_u64(0).map_err(|e| e.message().to_string())?,
    );
    let bytes1 = *tally.bytes.lock().unwrap();
    let wire1 = peer.as_deref().and_then(tcp_counters);
    // The two CPU reads are calls too: two of n + 2.
    let calls = n + 2;
    let protocol = (bytes1 - bytes0) as f64 / calls as f64;
    let wire = match (wire0, wire1) {
        (Some(w0), Some(w1)) => {
            // Payload both ways, and each segment's IPv4 and TCP headers
            // with timestamps (52 bytes).
            let bytes = (w1.0 - w0.0) + (w1.1 - w0.1);
            let segs = (w1.2 - w0.2) + (w1.3 - w0.3);
            Some((bytes + 52 * segs) as f64 / calls as f64)
        }
        _ => None,
    };

    // 2: calls a second with 32 threads at once.
    let deadline = Instant::now() + Duration::from_secs(a.seconds);
    let done = Arc::new(Mutex::new(0u64));
    let started = Instant::now();
    let threads: Vec<_> = (0..32)
        .map(|_| {
            let (c, done) = (counter.clone(), Arc::clone(&done));
            std::thread::spawn(move || {
                let mut k = 0;
                while Instant::now() < deadline {
                    c.count();
                    k += 1;
                }
                *done.lock().unwrap() += k;
            })
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
    let rate = *done.lock().unwrap() as f64 / started.elapsed().as_secs_f64();

    // 3: a 4 MiB argument, answered back.
    let big = "x".repeat(4 << 20);
    greeter.greet(&big).map_err(|e| e.message().to_string())?;
    let started = Instant::now();
    let rounds = 16;
    for _ in 0..rounds {
        greeter.greet(&big).map_err(|e| e.message().to_string())?;
    }
    let mbps = (2 * big.len() * rounds) as f64 / started.elapsed().as_secs_f64() / 1e6;

    let line = format!(
        "{{\"label\":{:?},\"url\":{:?},\"lat_p50_us\":{:.1},\"lat_p99_us\":{:.1},\"calls_per_s_32\":{:.0},\"mb_per_s_4mib\":{:.0},\"protocol_bytes_per_call\":{:.1},\"wire_bytes_per_call\":{},\"cpu_us_per_call\":{{\"client\":{:.2},\"server\":{:.2}}}}}",
        a.label,
        a.url,
        pct(&mut times, 0.5),
        pct(&mut times, 0.99),
        rate,
        mbps,
        protocol,
        wire.map_or("null".to_string(), |w| format!("{w:.1}")),
        (cpu1 - cpu0) as f64 / n as f64 / 1000.0,
        (server1 - server0) as f64 / n as f64 / 1000.0,
    );
    println!("{line}");
    if let Some(out) = &a.out {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
    }
    // Ends the server.
    let _ = wide.echo_u64(1);
    Ok(())
}

fn main() {
    let outcome = parse().and_then(|(mode, a)| match mode.as_str() {
        "serve" => serve(&a),
        "run" => run(&a),
        other => Err(format!("`{other}` is not serve or run")),
    });
    if let Err(why) = outcome {
        eprintln!("guatiao-remote-bench: {why}");
        std::process::exit(1);
    }
}
