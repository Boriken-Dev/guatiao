// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library written with no glue at all, loaded from disk and called as
//! the trait it implements.
//!
//! `examples/derived_greeter` is one type, two `impl`s and one macro line.
//! `examples/greeter_kind` is the two traits. This test is the host: it
//! maps the library, asks for every `dyn Greeter` on offer, and calls one.

#![cfg(all(feature = "provider", feature = "load"))]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use greeter_kind::{Counter, Greeter, GreeterVtable, Listener};
use guatiao::library::{Kind, KindMismatch, Offer, Registry, Remote};
use guatiao::{Status, Value};

/// Where cargo put a dev-dependency cdylib: beside this binary, or one up.
fn library_path(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let file = format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&file);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("{file} is not beside {}", deps.display());
}

fn loaded() -> Registry {
    let mut registry = Registry::new("derived-tests", env!("CARGO_PKG_VERSION"));
    for name in ["hello_library", "derived_greeter"] {
        registry
            .load_file(&library_path(name))
            .expect("loads")
            .loaded()
            .expect("is a library for this host");
    }
    registry
}

#[test]
fn a_derived_library_is_offered_as_the_trait_and_the_hand_written_one_is_not() {
    let registry = loaded();

    // 1. Only the derived provider is an offer; hello_library's
    // hand-written table sits in `vtable`, not `tables`, so the typed
    // path never sees it — neither as an offer nor as a mismatch.
    let offers: Vec<Offer<dyn Greeter>> = registry.offers::<dyn Greeter>().collect();
    let ids: Vec<&str> = offers.iter().map(Offer::id).collect();
    assert_eq!(ids, ["derived_greeter_hello", "derived_greeter_shouter"]);
    assert_eq!(registry.mismatches::<dyn Greeter>().count(), 0);
    assert!(
        registry
            .provider("hello_library_greeter")
            .unwrap()
            .table_for("greeter")
            .is_some(),
        "and the untyped path still serves it"
    );

    let offer = &offers[0];
    assert_eq!(offer.library(), Some("derived_greeter"));
    assert_eq!(
        offer.version(),
        env!("CARGO_PKG_VERSION"),
        "inherited from the library"
    );
    assert_eq!(offer.available(), Ok(()));
    let answer = offer.greet("ana").expect("the derived greeter answers");
    assert_eq!(
        answer.get("greeting").and_then(Value::as_str),
        Some("hello, ana")
    );

    // 2. The same provider, under its second kind.
    let counter = registry
        .offer::<dyn Counter>("derived_greeter_hello")
        .expect("filed under its id")
        .expect("a valid table");
    let first = counter.count();
    assert_eq!(counter.count(), first + 1, "one instance, counting");

    // 3. A panic inside the implementation is a status; the process lives.
    let e = offer.greet("panic").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL);

    // 4. A ProviderError crosses with its message.
    let e = offer.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet");

    // 5. A default-bodied method runs the host's default when the table
    // ends before its slot: the same table, copied with a smaller size.
    let remote = offer.remote();
    let truncated: &'static [u8] = Box::leak(
        // SAFETY: the table is `GreeterVtable`-sized bytes the library
        // keeps for the process; copying them is a read.
        unsafe { std::slice::from_raw_parts(remote.table().cast::<u8>(), remote.size()) }
            .to_vec()
            .into_boxed_slice(),
    );
    // SAFETY: a byte-for-byte copy of a validated table for this kind,
    // presented as ending after `greet`; `ctx` is the library's instance.
    let short = unsafe {
        Remote::<dyn Greeter>::from_raw(
            truncated.as_ptr().cast::<c_void>(),
            GreeterVtable::floor(),
            remote.ctx(),
        )
    }
    .expect("the floor is enough");
    assert_eq!(
        short.shout("ana"),
        "HELLO, ANA",
        "the proxy's default body ran, through its own `greet`"
    );

    // 6. A forged floor hash is a mismatch.
    let mut forged = truncated.to_vec();
    forged[4] ^= 1;
    let forged: &'static [u8] = Box::leak(forged.into_boxed_slice());
    // SAFETY: as above; the header is what is being tested.
    let why = unsafe {
        Remote::<dyn Greeter>::from_raw(
            forged.as_ptr().cast::<c_void>(),
            remote.size(),
            remote.ctx(),
        )
    }
    .unwrap_err();
    assert!(matches!(why, KindMismatch::HashMismatch { .. }), "{why:?}");
    assert_eq!(<dyn Greeter as Kind>::NAME, "greeter");

    // 7. A provider built from a configuration, across the same dlopen:
    // the host reads its schema, builds a value that fits, and gets an
    // instance whose `self` is that configuration.
    let shouter = &offers[1];
    assert!(shouter.builds_instances());
    assert!(
        !offer.builds_instances(),
        "the hand-made one is its one instance"
    );
    let schema = shouter.config_schema().expect("declares what it takes");
    assert!(
        schema
            .get("properties")
            .and_then(|p| p.get("prefix"))
            .is_some(),
        "the schema is Shouter's own"
    );
    let mut config = Value::map();
    config.set("prefix", "hey").unwrap();
    let loud = shouter
        .instantiate(&config)
        .expect("a fitting configuration builds");
    assert_eq!(
        loud.greet("ana")
            .unwrap()
            .get("greeting")
            .and_then(Value::as_str),
        Some("hey ANA")
    );
    let mut other = Value::map();
    other.set("prefix", "yo").unwrap();
    let quiet = shouter.instantiate(&other).unwrap();
    assert_eq!(
        quiet
            .greet("bo")
            .unwrap()
            .get("greeting")
            .and_then(Value::as_str),
        Some("yo BO"),
        "two instances, two configurations, one provider"
    );
    drop(loud);
    let e = shouter.instantiate(&Value::map()).unwrap_err();
    assert_eq!(
        e.status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "the schema's refusal: {e}"
    );
    let mut wrong = Value::map();
    wrong.set("prefix", 7).unwrap();
    let e = shouter.instantiate(&wrong).unwrap_err();
    assert_eq!(
        e.status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "the decode's refusal, since the type is its own configuration: {e}"
    );
    let e = offer.instantiate(&config).unwrap_err();
    assert_eq!(
        e.status,
        Status::GUATIAO_ERR_NULL,
        "no instances from a provider that is its one"
    );
}

/// A host-side listener: an object kind implemented here, handed to the
/// library, called back from it, and destroyed by it.
struct Tape {
    heard: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
}

impl Listener for Tape {
    fn heard(&mut self, what: &str) {
        self.heard.lock().unwrap().push(what.to_string());
    }
}

impl Drop for Tape {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn objects_cross_both_ways_and_are_destroyed_by_whoever_holds_them_last() {
    let registry = loaded();
    let offer = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .expect("filed under its id")
        .expect("a valid table");

    // 1. The host hands an object IN and gets one OUT, across the dlopen.
    let heard = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let listener = Tape {
        heard: Arc::clone(&heard),
        dropped: Arc::clone(&dropped),
    }
    .into_object();
    let mut chat = offer
        .start(listener)
        .expect("the derived greeter holds conversations");
    assert_eq!(chat.turns(), 0);

    // 2. `&mut self` calls drive the object; the library calls the host's
    // listener back from inside them.
    chat.say("hi").unwrap();
    chat.say("there").unwrap();
    assert_eq!(chat.turns(), 2);
    assert_eq!(*heard.lock().unwrap(), ["hi", "there"]);

    // 3. A ProviderError crosses out of an object method like any other.
    let e = chat.say("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nothing to say");

    // 4. An out-buffer: the object writes into the host's own memory and
    // says how much; a short buffer is filled and not overrun.
    let mut buffer = [0u8; 64];
    let n = chat.read(&mut buffer);
    assert_eq!(&buffer[..n as usize], b"hi\nthere");
    let mut short = [0u8; 3];
    assert_eq!(chat.read(&mut short), 3);
    assert_eq!(&short, b"hi\n");

    // 5. Dropping the handle runs the library's `destroy`, which drops the
    // conversation, which drops the listener the library owned -- whose
    // `destroy` is the host's. Both crossed once, in opposite directions.
    assert!(
        !dropped.load(Ordering::SeqCst),
        "held while the conversation lives"
    );
    drop(chat);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the host's listener was destroyed by the library"
    );

    // 6. A greeter built before `start` existed: the table ends before
    // the slot, so the PROXY runs the default body here, and the listener
    // it was handed is dropped on this side. Nothing crossed.
    let remote = offer.remote();
    let truncated: &'static [u8] = Box::leak(
        // SAFETY: the table is `GreeterVtable`-sized bytes the library
        // keeps for the process; copying them is a read.
        unsafe { std::slice::from_raw_parts(remote.table().cast::<u8>(), remote.size()) }
            .to_vec()
            .into_boxed_slice(),
    );
    // SAFETY: a byte-for-byte copy of a validated table, presented as
    // ending after `greet`; `ctx` is the library's instance.
    let old = unsafe {
        Remote::<dyn Greeter>::from_raw(
            truncated.as_ptr().cast::<c_void>(),
            GreeterVtable::floor(),
            remote.ctx(),
        )
    }
    .unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let listener = Tape {
        heard: Arc::new(Mutex::new(Vec::new())),
        dropped: Arc::clone(&dropped),
    }
    .into_object();
    let e = old.start(listener).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_NULL, "{e}");
    assert_eq!(e.message(), "this greeter holds no conversations");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the default body dropped the listener on this side"
    );

    // 7. A provider with no default instance refuses the call before it
    // runs anything -- and still destroys the object it was handed,
    // because ownership crossed with the call. The one leak an early
    // return could cause, and the reason objects are taken first.
    let shouter = registry
        .offer::<dyn Greeter>("derived_greeter_shouter")
        .unwrap()
        .unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let listener = Tape {
        heard: Arc::new(Mutex::new(Vec::new())),
        dropped: Arc::clone(&dropped),
    }
    .into_object();
    let e = shouter.start(listener).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_NULL, "{e}");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the shim destroyed the listener before refusing"
    );
}
