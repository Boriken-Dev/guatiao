// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A handle into a library holds a lease on it: the table it calls is
//! the registry's copy, and what the library's descriptor named outlives
//! the registry's own record for as long as the handle does.

#![cfg(all(feature = "provider", feature = "load"))]

use core::ffi::c_void;

use greeter_kind::{Counter, CounterVtable};
use guatiao::library::kind::{LibraryParts, ProviderParts};
use guatiao::library::{KindTable, Registry};

struct Seven;

impl Counter for Seven {
    fn count(&self) -> i64 {
        7
    }
}

static SEVEN: Seven = Seven;
static TABLE: CounterVtable = CounterVtable::of::<Seven>();

fn parts() -> LibraryParts {
    let provider = ProviderParts::builder("leased_seven")
        .kinds(&["counter"])
        .table(KindTable::new(
            "counter",
            &TABLE as *const CounterVtable as *const c_void,
            size_of::<CounterVtable>(),
        ))
        .ctx(&SEVEN as *const Seven as *mut c_void)
        .build();
    LibraryParts::new("leased", "1.0.0", vec![provider])
}

fn registry() -> Registry {
    let mut registry = Registry::new("lease-tests", "1.0");
    registry
        .register_local("leased", |_host| Some(parts()))
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    registry
}

/// The table a handle calls is the registry's copy of the library's,
/// byte for byte, and not the library's own.
#[test]
fn a_handle_calls_the_registrys_copy_of_the_table() {
    let registry = registry();
    let remote = registry
        .offer::<dyn Counter>("leased_seven")
        .expect("filed under its id")
        .expect("a valid table")
        .remote()
        .expect("its own instance");
    let library = (&TABLE as *const CounterVtable).cast::<c_void>();
    assert_ne!(remote.table(), library, "a copy, not the library's table");
    assert_eq!(remote.size(), size_of::<CounterVtable>());
    // SAFETY: both address `size` readable bytes: the library's static,
    // and the copy the handle's lease keeps.
    let (copy, original) = unsafe {
        (
            core::slice::from_raw_parts(remote.table().cast::<u8>(), remote.size()),
            core::slice::from_raw_parts(library.cast::<u8>(), remote.size()),
        )
    };
    assert_eq!(copy, original);
    assert_eq!(remote.as_dyn().count(), 7);

    let (view_table, _) = registry
        .provider("leased_seven")
        .expect("filed under its id")
        .table_for("counter")
        .expect("it serves the kind");
    assert_eq!(
        view_table,
        remote.table(),
        "one copy per table, shared by every handle"
    );
}

/// A host's own descriptor is borrowed for the registration alone: the
/// registry copies what it says, so the descriptor may go right after.
#[test]
fn a_borrowed_descriptor_is_read_once_and_not_kept() {
    let mut registry = Registry::new("lease-tests", "1.0");
    let built = parts();
    registry
        .register_local("borrowed", |_host| Some(&built))
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    drop(built);
    let provider = registry
        .provider("leased_seven")
        .expect("filed under its id");
    assert_eq!(provider.kinds(), ["counter"]);
    let remote = provider
        .as_kind::<dyn Counter>()
        .expect("a valid table")
        .expect("its own instance");
    assert_eq!(remote.as_dyn().count(), 7, "its copy of the table calls");

    // The same for a descriptor read with no registry: the view keeps
    // nothing of it.
    let built = parts();
    let view = built.info().providers.ptr;
    // SAFETY: the parts own one provider descriptor at `ptr`.
    let view = unsafe { &*view }.view().expect("it reads");
    drop(built);
    assert_eq!(view.id(), "leased_seven");
    let (table, _) = view.table_for("counter").expect("it serves the kind");
    assert_ne!(table, (&TABLE as *const CounterVtable).cast::<c_void>());
}

/// `Remote` is `Clone`, not `Copy`: each clone holds the lease, and every
/// one keeps working after the library is retired.
#[test]
fn every_clone_outlives_the_retire() {
    let mut registry = registry();
    let first = registry
        .offer::<dyn Counter>("leased_seven")
        .expect("filed under its id")
        .expect("a valid table")
        .remote()
        .expect("its own instance");
    let second = first.clone();
    registry.retire("leased").expect("it is registered");
    drop(registry);
    assert_eq!(first.as_dyn().count(), 7);
    drop(first);
    assert_eq!(second.as_dyn().count(), 7, "the last clone still calls");
}
