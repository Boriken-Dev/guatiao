// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A server takes its kinds from the providers it serves: given no
//! descriptions, a host calls a provider that carries its own, objects both
//! ways included; a description the server is given still decides its
//! kind.

extern crate alloc;

mod common;

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use common::{CHATS, Ear, Memory, eventually};
use greeter_kind::{Counter, Greeter, Listener};
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};
use guatiao_source_remote::{Kinds, LinkOptions, RemoteSource, Server, Trust};

fn server(described: Kinds) -> Server {
    let mut registry = Registry::new("server", "1.0");
    registry
        .register_local("shapes", common::library)
        .expect("registers")
        .loaded()
        .expect("loaded");
    Server::new(registry, described, LinkOptions::new(Trust::Local))
}

fn host(server: &Server) -> Registry {
    let source = RemoteSource::new(
        Box::new(Memory(server.clone())),
        LinkOptions::new(Trust::Local),
    );
    common::host(&source)
}

fn greeting(map: &Map) -> String {
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_server_given_no_descriptions_serves_providers_that_carry_their_own() {
    let server = server(Kinds::new());
    let registry = host(&server);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("the remote greeter is offered")
        .shared()
        .unwrap();
    assert_eq!(greeting(&greeter.greet("ana").unwrap()), "hello, ana");
    assert_eq!(
        greeter.greet("").unwrap_err().status,
        Status::GUATIAO_ERR_BAD_VALUE
    );

    // A listener lent, called back during the call: the server reads it
    // through a kind only the provider's document describes.
    let mut ear = Ear::new();
    greeter.greet_into("di", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, di", "twice"]);

    // A listener handed over and a conversation answered.
    let ear = Ear::new();
    let heard = Arc::clone(&ear.heard);
    let mut conversation = greeter.start(ear.into_object()).unwrap();
    conversation.say("hi").unwrap();
    assert_eq!(*heard.lock().unwrap(), ["hi"]);
    drop(conversation);
    eventually("the conversation is dropped on the server", || {
        CHATS.load(Ordering::SeqCst) == 0
    });

    // Every kind the provider serves, and the host's provider carries the
    // document the catalog's descriptions write.
    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(counter.count(), 1);
    let provider = &registry.all()[0];
    let names: Vec<String> = provider
        .kind_descriptions()
        .unwrap()
        .iter()
        .map(|k| k.name().to_string())
        .collect();
    for kind in ["greeter", "listener", "conversation", "counter", "wide"] {
        assert!(names.iter().any(|n| n == kind), "{kind} in {names:?}");
    }
}

#[test]
fn a_description_the_server_is_given_decides_its_kind() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let text = std::fs::read_to_string(path).unwrap();
    let edited = text.replace("\"Greets by name.\"", "\"Greets by name, as given.\"");
    assert_ne!(edited, text, "the greeter's doc is where it was");
    let mut described = Kinds::new();
    described.add_json(&edited).unwrap();

    let server = server(described);
    let registry = host(&server);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(greeting(&greeter.greet("bo").unwrap()), "hello, bo");
    let kinds = registry.all()[0].kind_descriptions().unwrap();
    let served = kinds.iter().find(|k| k.name() == "greeter").unwrap();
    assert_eq!(served.doc(), Some("Greets by name, as given."));
}
