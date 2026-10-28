// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A server serving a greeter of every slot shape, and a host reaching it
//! over an in-memory link.

#![allow(dead_code)]

extern crate alloc;

pub mod procs;

#[path = "../../../guatiao/tests/support/c_compiler.rs"]
pub mod c_compiler;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use core::time::Duration;
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use greeter_kind::{Conversation, Counter, Greeter, Listener, Wide};
use guatiao::library::{Object, ProviderError, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};
use guatiao_source_remote::transport::{Connect, Connection, memory};
use guatiao_source_remote::{Kinds, LinkOptions, RemoteSource, Server, Trust};

/// An access control list permitting `sources` alone.
#[cfg(feature = "tcp")]
pub fn permit(sources: &[&str]) -> guatiao_source_remote::acl::Acl {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    let mut acl = Acl::default();
    for s in sources {
        acl.push(Rule::new(
            Action::Permit,
            s.parse().expect("an address range"),
        ));
    }
    acl
}

/// Conversations alive on the server, in every test of a binary.
pub static CHATS: AtomicIsize = AtomicIsize::new(0);

/// A greeter that fills every slot.
#[derive(Default, guatiao::Provider)]
#[provider(Greeter, Counter, Wide)]
pub struct Hello {
    asked: Mutex<i64>,
    /// Milliseconds `start` waits before answering.
    slow_start: AtomicU64,
}

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        let mut map = Map::new();
        map.set("greeting", format!("hello, {name}").as_str());
        Ok(map)
    }

    fn shout(&self, name: &str) -> String {
        format!("HELLO, {}", name.to_uppercase())
    }

    fn start(
        &self,
        listener: Object<dyn Listener>,
    ) -> Result<Object<dyn Conversation>, ProviderError> {
        let wait = self.slow_start.load(Ordering::Relaxed);
        if wait > 0 {
            std::thread::sleep(Duration::from_millis(wait));
        }
        CHATS.fetch_add(1, Ordering::SeqCst);
        Ok(Chat {
            listener,
            said: String::new(),
            turns: 0,
        }
        .into_object())
    }

    fn greet_into(&self, name: &str, listener: &mut dyn Listener) -> Result<(), ProviderError> {
        listener.heard(&format!("hello, {name}"));
        listener.heard("twice");
        Ok(())
    }

    fn greet_from(&self, config: Map) -> Result<Map, ProviderError> {
        let name = config
            .get("name")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or_default()
            .to_string();
        self.greet(&name)
    }

    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        let name = config
            .get("name")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or("world")
            .to_string();
        config.set("greeting", format!("hello, {name}").as_str());
        Ok(true)
    }
}

impl Counter for Hello {
    fn count(&self) -> i64 {
        let mut asked = self.asked.lock().unwrap();
        *asked += 1;
        *asked
    }
}

impl Wide for Hello {
    /// From 1000 up, waits `x - 1000` milliseconds first.
    fn echo_i64(&self, x: i64) -> Result<i64, ProviderError> {
        if x >= 1000 {
            std::thread::sleep(Duration::from_millis((x - 1000) as u64));
        }
        Ok(x)
    }

    /// From 2^40 up, sets how long `start` waits, in milliseconds.
    fn echo_u64(&self, x: u64) -> Result<u64, ProviderError> {
        if x >= 1 << 40 {
            self.slow_start.store(x - (1 << 40), Ordering::Relaxed);
        }
        Ok(x)
    }
}

/// One conversation: the listener the host handed over.
struct Chat {
    listener: Object<dyn Listener>,
    said: String,
    turns: i64,
}

impl Drop for Chat {
    fn drop(&mut self) {
        CHATS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Conversation for Chat {
    fn say(&mut self, what: &str) -> Result<(), ProviderError> {
        if what.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nothing to say",
            ));
        }
        self.said.push_str(what);
        self.turns += 1;
        self.listener.heard(what);
        Ok(())
    }

    fn read(&mut self, dst: &mut [u8]) -> i64 {
        let n = self.said.len().min(dst.len());
        dst[..n].copy_from_slice(&self.said.as_bytes()[..n]);
        n as i64
    }

    fn turns(&self) -> i64 {
        self.turns
    }
}

guatiao::local_providers!(id = "shapes", version = "1.0.0", providers = [Hello]);

/// The kinds `greeter_kind` describes, found above whichever package
/// includes this file.
pub fn described() -> Kinds {
    let kinds = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|dir| dir.join("examples/greeter_kind/include/greeter_kind.kinds.json"))
        .find(|path| path.is_file())
        .expect("greeter_kind's descriptions, in the checkout");
    let mut described = Kinds::new();
    described.load(&kinds).expect("greeter_kind's descriptions");
    described
}

/// A server of [`Hello`], at `trust`.
pub fn server(options: LinkOptions) -> Server {
    let mut registry = Registry::new("server", "1.0");
    registry
        .register_local("shapes", library)
        .expect("registers")
        .loaded()
        .expect("loaded");
    Server::new(registry, described(), options)
}

/// Connects to a server in this process.
pub struct Memory(pub Server);

impl Connect for Memory {
    fn connect(&self) -> io::Result<Connection> {
        let (host, device) = memory::pair();
        let server = self.0.clone();
        std::thread::spawn(move || {
            let _ = server.serve(device);
        });
        Ok(host)
    }

    fn describe(&self) -> String {
        "memory".to_string()
    }
}

/// Serves `conn`, whose client stays silent, and ends `server`'s links
/// while its handshake waits, every 50 ms from 300 ms on (a serving thread
/// slow to start is not missed); answers how long the handshake lasted,
/// which failed. The handshake's own bound is the 10 s default.
pub fn ended_in_handshake(server: &Server, conn: Connection) -> Duration {
    let (tx, rx) = std::sync::mpsc::channel();
    let serving = server.clone();
    let started = Instant::now();
    std::thread::spawn(move || {
        let failed = serving.serve(conn).is_err();
        let _ = tx.send(failed);
    });
    std::thread::sleep(Duration::from_millis(300));
    let failed = loop {
        server.end_links();
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(failed) => break failed,
            Err(_) if started.elapsed() < Duration::from_secs(15) => {}
            Err(e) => panic!("the handshake never ended: {e}"),
        }
    };
    assert!(failed, "a silent client never links");
    started.elapsed()
}

/// A host registry with `source` added and refreshed.
pub fn host(source: &RemoteSource) -> Registry {
    let mut registry = Registry::new("host", "1.0");
    registry
        .add_source("remote", source.clone())
        .expect("the source is added");
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{:?}", refreshed.refused);
    assert_eq!(refreshed.loaded.len(), 1, "{refreshed:?}");
    registry
}

/// A host over memory to a fresh server.
pub fn pair(timeout: Option<Duration>) -> (Server, RemoteSource, Registry) {
    let server = server(LinkOptions::new(Trust::Local));
    let source = RemoteSource::new(
        Box::new(Memory(server.clone())),
        LinkOptions::new(Trust::Local).timeout(timeout),
    );
    let registry = host(&source);
    (server, source, registry)
}

/// A listener that keeps what it heard, and counts its own drop.
pub struct Ear {
    pub heard: Arc<Mutex<Vec<String>>>,
    pub dropped: Arc<Mutex<usize>>,
}

impl Ear {
    pub fn new() -> Ear {
        Ear {
            heard: Arc::new(Mutex::new(Vec::new())),
            dropped: Arc::new(Mutex::new(0)),
        }
    }

    pub fn twin(&self) -> Ear {
        Ear {
            heard: Arc::clone(&self.heard),
            dropped: Arc::clone(&self.dropped),
        }
    }
}

impl Listener for Ear {
    fn heard(&mut self, what: &str) {
        self.heard.lock().unwrap().push(what.to_string());
    }
}

impl Drop for Ear {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() += 1;
    }
}

/// Waits until `f` holds, for at most five seconds.
pub fn eventually(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
