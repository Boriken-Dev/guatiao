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

use greeter_kind::{Counter, Greeter, GreeterVtable};
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
    assert_eq!(ids, ["derived_greeter_hello"]);
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
}
