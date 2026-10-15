// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a library asks its host, over descriptors the host LINKS: no
//! `dlopen`, so Miri runs it, under Stacked and Tree Borrows both.
//!
//! An older library's descriptor is shorter than this build's
//! `ProviderInfo`. The host's answers are read under the size each
//! descriptor declares, so a field past it reads as absent and nothing
//! forms a reference to bytes the library never wrote.

#![cfg(feature = "load")]

use guatiao::library::{
    EntryPoint, HostInfo, KindTables, Kinds, LibraryInfo, ProviderInfo, Providers, Registry,
};
use guatiao::value::types::{MaybeNull, Str};

/// One provider descriptor, `declared` bytes of it and nothing after: what
/// a library compiled before the newer fields hands over.
fn short_provider(id: &'static str, declared: usize) -> *const ProviderInfo<'static> {
    let full = ProviderInfo {
        struct_size: declared as u32,
        vtable_size: 0,
        kinds: Kinds::empty(),
        id: Str::new(id),
        display_name: Str::new("Old"),
        config: core::ptr::null(),
        vtable: core::ptr::null(),
        ctx: core::ptr::null_mut(),
        meta: MaybeNull::null(),
        version: Str::new("9.9.9"),
        available: None,
        tables: KindTables::empty(),
        create: None,
        destroy: None,
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
fn older_library() -> *const LibraryInfo<'static> {
    let floor = ProviderInfo::floor();
    Box::leak(Box::new(LibraryInfo {
        struct_size: size_of::<LibraryInfo>() as u32,
        abi_version: guatiao::library::ABI_VERSION,
        id: Str::new("old"),
        version: Str::new("0.1"),
        providers: Providers {
            ptr: short_provider("old_provider", floor),
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
unsafe extern "C" fn older_entry(_host: *const HostInfo<'static>) -> *const LibraryInfo<'static> {
    older_library()
}

fn registry_with_an_older_library() -> Registry {
    let mut registry = Registry::new("lookups", "1.0");
    // SAFETY: `older_entry` answers a leaked descriptor, well-formed for
    // the size each part of it declares.
    let entry = unsafe { EntryPoint::new(older_entry) };
    registry
        .register_entry("old", entry)
        .expect("an older library registers")
        .loaded()
        .expect("it accepts this host");
    registry
}

/// `get` reads an older descriptor under its own size: the fields it
/// declares come back, the ones it predates read as absent.
#[test]
fn a_host_reads_an_older_descriptor_under_its_declared_size() {
    let mut registry = registry_with_an_older_library();
    let host = registry.host();
    let view = host
        .get("old_provider")
        .expect("the host offers services")
        .expect("the provider is found");
    assert_eq!(view.id(), "old_provider");
    assert_eq!(view.display_name(), "Old");
    assert_eq!(
        view.version(),
        None,
        "`version` sits past the declared size, so it is absent"
    );
    assert!(view.meta().is_none() && view.create().is_none());
    assert_eq!(view.available(), Ok(()));
}

/// `list` answers the same descriptors, in the host's order, read the
/// same way; an unknown key is `None`; a dropped registry is `GONE`.
#[test]
fn a_host_lists_what_it_gets() {
    let mut registry = registry_with_an_older_library();
    let host = registry.host();
    let listed = host.list("").expect("the host offers services");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), "old_provider");
    let got = host.get("old_provider").unwrap().unwrap();
    assert_eq!(got.raw(), listed[0].raw(), "one descriptor, read twice");
    assert!(matches!(host.get("nobody"), Ok(None)));

    drop(registry);
    assert_eq!(
        host.list("").err(),
        Some(guatiao::Status::GUATIAO_ERR_GONE),
        "the block outlives the registry, and says so"
    );
}
