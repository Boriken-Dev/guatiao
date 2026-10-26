// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a library asks its host, over descriptors the host LINKS: no
//! `dlopen`, so Miri runs it, under Stacked and Tree Borrows both.
//!
//! The library's descriptor is exactly the bytes it declares, in an
//! allocation no larger: the host's answers read every field this build
//! knows and form no reference past them.

#![cfg(feature = "load")]

use guatiao::library::{
    HostInfo, KindTables, Kinds, LibraryInfo, ProviderInfo, Providers, Registry,
};
use guatiao::value::types::{MaybeNull, Str};

/// One provider descriptor, `declared` bytes of it and nothing after.
fn provider(id: &'static str, declared: usize) -> *const ProviderInfo<'static> {
    let full = ProviderInfo {
        struct_size: declared as u32,
        vtable_size: 0,
        kinds: Kinds::empty(),
        id: Str::new(id),
        display_name: Str::new("Linked"),
        config: core::ptr::null(),
        vtable: core::ptr::null(),
        ctx: core::ptr::null_mut(),
        meta: MaybeNull::null(),
        version: Str::new("9.9.9"),
        available: None,
        tables: KindTables::empty(),
        create: None,
        destroy: None,
        kind_descriptions: Str::new(""),
    };
    // `u64`s for the alignment; exactly the declared bytes are copied, and
    // the allocation is no larger than the words those bytes need.
    let words: &'static mut [u64] = Vec::leak(vec![0u64; declared.div_ceil(8)]);
    // SAFETY: `words` owns at least `declared` bytes, and `full` is a
    // whole descriptor to copy the leading `declared` bytes of.
    unsafe {
        core::ptr::copy_nonoverlapping(
            (&full as *const ProviderInfo).cast::<u8>(),
            words.as_mut_ptr().cast::<u8>(),
            declared,
        );
    }
    words.as_ptr().cast()
}

/// A library whose one provider is a floor-size descriptor at the floor
/// stride.
fn a_library() -> *const LibraryInfo<'static> {
    let floor = ProviderInfo::floor();
    Box::leak(Box::new(LibraryInfo {
        struct_size: size_of::<LibraryInfo>() as u32,
        abi_version: guatiao::library::ABI_VERSION,
        id: Str::new("old"),
        version: Str::new("0.1"),
        providers: Providers {
            ptr: provider("old_provider", floor),
            len: 1,
            stride: floor,
        },
        meta: MaybeNull::null(),
        unload: None,
    }))
}

/// # Safety
///
/// Called by the registry with its host descriptor.
unsafe extern "C" fn entry(_host: *const HostInfo<'static>) -> *const LibraryInfo<'static> {
    a_library()
}

fn registry_with_a_library() -> Registry {
    let mut registry = Registry::new("lookups", "1.0");
    registry
        .register_entry("old", entry)
        .expect("the library registers")
        .loaded()
        .expect("it accepts this host");
    registry
}

/// `get` reads the descriptor's every field.
#[test]
fn a_host_reads_the_descriptor_it_was_handed() {
    let mut registry = registry_with_a_library();
    let host = registry.host();
    let view = host
        .get("old_provider")
        .expect("the host offers services")
        .expect("the provider is found");
    assert_eq!(view.id(), "old_provider");
    assert_eq!(view.display_name(), "Linked");
    assert_eq!(view.version(), Some("9.9.9"));
    assert!(view.meta().is_none() && view.create().is_none());
    assert_eq!(view.available(), Ok(()));
}

/// `list` answers the same descriptors, in the host's order, read the
/// same way; an unknown key is `None`; a dropped registry is `GONE`.
#[test]
fn a_host_lists_what_it_gets() {
    let mut registry = registry_with_a_library();
    let host = registry.host();
    let listed = host.list("").expect("the host offers services");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), "old_provider");
    let got = host.get("old_provider").unwrap().unwrap();
    assert_eq!(got.id(), listed[0].id(), "one provider, read twice");
    assert!(matches!(host.get("nobody"), Ok(None)));

    drop(registry);
    assert_eq!(
        host.list("").err(),
        Some(guatiao::Status::GUATIAO_ERR_GONE),
        "the block outlives the registry, and says so"
    );
}

/// A library the host LINKS, built by hand and handed over: what
/// `register_local` takes, and owns from then on.
fn linked_library(_host: guatiao::library::Host) -> Option<guatiao::library::kind::LibraryParts> {
    use guatiao::library::kind::{LibraryParts, ProviderParts};
    let provider = ProviderParts::builder("linked_provider")
        .name("Linked")
        .version("2.0.0")
        .kinds(&["linked"])
        .build();
    Some(LibraryParts::new("linked", "1.0.0", vec![provider]))
}

/// A linked library's descriptor answers through the host as a loaded
/// one's does, and the host block's own text reads back.
#[test]
fn a_host_answers_for_a_library_it_links() {
    let mut registry = Registry::new("lookups", "1.0");
    registry
        .register_local("linked", linked_library)
        .expect("a linked library registers")
        .loaded()
        .expect("it accepts this host");
    let host = registry.host();
    assert_eq!((host.id(), host.version()), ("lookups", "1.0"));
    let view = host
        .get("linked_provider")
        .expect("the host offers services")
        .expect("the provider is found");
    assert_eq!(view.display_name(), "Linked");
    assert_eq!(view.version(), Some("2.0.0"));
    assert_eq!(view.kinds(), ["linked"]);
    assert_eq!(host.list("linked").unwrap().len(), 1);
    assert!(host.list("nothing").unwrap().is_empty());
}
