// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A replaced peer whose successor cannot be opened keeps serving. The
//! successor is a scripted peer listing a kind it does not describe, which
//! no `Server` does.

use alloc::sync::Arc;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::time::Instant;

use greeter_kind::Greeter;
use guatiao::library::{Refreshed, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status, Value};

use super::RemoteSource;
use crate::acl::{Acl, Action, Rule};
use crate::link::{Handler, Inner, Link, LinkOptions, Proxies, Setup};
use crate::protocol::Trust;
use crate::protocol::catalog::{Catalog, LibraryEntry, ProviderEntry, TableEntry};
use crate::server::{Described, Server};
use crate::transport::Connection;
use crate::transport::tcp::{Accepted, TcpListen};
use guatiao::library::ProviderError;

const KEY: &str = "a key every device here holds";

#[derive(Default, guatiao::Provider)]
#[provider(Greeter)]
struct Hello;

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        let mut map = Map::new();
        map.set("greeting", format!("hello, {name}").as_str());
        Ok(map)
    }
}

guatiao::local_providers!(id = "shapes", version = "1.0.0", providers = [Hello]);

/// Takes every connection a loopback listener admits; answers its port.
fn listen(take: impl Fn(Connection) + Send + 'static) -> u16 {
    let mut acl = Acl::default();
    acl.push(Rule::new(Action::Permit, "127.0.0.1".parse().unwrap()));
    let listener = TcpListen::bind("127.0.0.1:0", acl).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                take(conn);
            }
        }
    });
    port
}

/// A device serving `Hello`; answers its port and its server.
fn device() -> (u16, Server) {
    let mut registry = Registry::new("server", "1.0");
    registry
        .register_local("shapes", library)
        .unwrap()
        .loaded()
        .unwrap();
    let mut described = Described::new();
    described
        .load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/greeter_kind/include/greeter_kind.kinds.json"),
        )
        .unwrap();
    let server = Server::new(
        registry,
        described,
        LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()),
    );
    let serving = server.clone();
    let port = listen(move |conn| {
        let server = serving.clone();
        std::thread::spawn(move || {
            let _ = server.serve(conn);
        });
    });
    (port, server)
}

/// Lists `shapes` with a `greeter` table and describes no kind.
struct Undescribed;

impl Handler for Undescribed {
    fn list(&self, _link: &Arc<Inner>) -> Result<Value, ProviderError> {
        let table = TableEntry {
            kind: "greeter".to_string(),
            table: 1,
            slots: vec!["greet".to_string()],
        };
        let provider = ProviderEntry {
            id: "hello".to_string(),
            name: "Hello".to_string(),
            meta: None,
            tables: vec![table],
        };
        let library = LibraryEntry {
            id: "shapes".to_string(),
            version: "1.0.0".to_string(),
            stamp: "scripted".to_string(),
            providers: vec![provider],
        };
        let catalog = Catalog {
            kinds: Vec::new(),
            libraries: vec![library],
        };
        Ok(catalog.to_value())
    }

    fn open(&self, _link: &Arc<Inner>, _library: &str, _stamp: &str) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// A scripted peer whose library no host can open; answers its port.
fn undescribed() -> u16 {
    listen(|conn| {
        std::thread::spawn(move || {
            let setup = Setup {
                handler: Arc::new(Undescribed),
                proxies: Proxies::new(),
                kinds: Arc::default(),
                name: "scripted".to_string(),
            };
            let options = LinkOptions::new(Trust::Trusted).key(KEY.as_bytes());
            if let Ok(link) = Link::server(conn, &options, setup) {
                while link.alive() {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gt-leaving-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn manifest(dir: &Path, port: u16) {
    let text = format!(
        "type = \"remote\"\nname = \"lab\"\nconnect = \"tcp://127.0.0.1:{port}\"\ntrust = \"trusted\"\n"
    );
    std::fs::write(dir.join("lab.guatiao.toml"), text).unwrap();
}

fn source(dir: &Path) -> RemoteSource {
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    config.set("key", KEY);
    RemoteSource::from_config(&config).unwrap()
}

fn keys(refreshed: &[guatiao::library::Served]) -> Vec<&str> {
    refreshed.iter().map(|s| s.key.as_str()).collect()
}

fn greeting(greeter: &dyn Greeter, name: &str) -> Result<String, Status> {
    let answer = greeter.greet(name).map_err(|e| e.status)?;
    Ok(answer
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string())
}

/// Waits until `server` has no link left.
fn links_end(server: &Server) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while server.links() > 0 {
        if Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn refused_words(refreshed: &Refreshed) -> String {
    refreshed
        .refused
        .iter()
        .map(|r| r.reason.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn a_replaced_peer_serves_on_while_its_successor_will_not_open() {
    let dir = scratch("open");
    let (first, old) = device();
    manifest(&dir, first);
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["lab/shapes"], "{refreshed:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();

    // The successor lists the library and cannot open it: the old link
    // stays, and so does the old library.
    manifest(&dir, undescribed());
    let refreshed = registry.refresh();
    assert!(
        refreshed.loaded.is_empty() && refreshed.retired.is_empty(),
        "{refreshed:?}"
    );
    let words = refused_words(&refreshed);
    assert!(words.contains("without describing it"), "{words}");
    assert_eq!(greeting(&*greeter, "ana"), Ok("hello, ana".to_string()));

    // A successor that opens replaces it, and the old link ends once the
    // old library is let go.
    let (third, _new) = device();
    manifest(&dir, third);
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["lab/shapes"], "{refreshed:?}");
    assert_eq!(keys(&refreshed.retired), ["lab/shapes"]);
    drop(greeter);
    registry.refresh();
    assert!(links_end(&old), "the replaced peer's link was kept");
    let _ = std::fs::remove_dir_all(&dir);
}
