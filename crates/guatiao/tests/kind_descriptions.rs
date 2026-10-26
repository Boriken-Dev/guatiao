// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a provider carries in `kind_descriptions`, read back by a host in
//! Rust and in C: a document as it was written, none for empty text or a
//! descriptor older than the field, and a refusal naming where for text
//! that is not a description. Each case is a library this binary links,
//! written by hand as a C library would write it.

#![cfg(feature = "load")]

use std::sync::OnceLock;

use guatiao::exports::kind::guatiao_kind_document_read;
use guatiao::exports::library::{
    guatiao_registry_free, guatiao_registry_kind_descriptions, guatiao_registry_new,
    guatiao_registry_register_entry,
};
use guatiao::library::kind::KindDescription;
use guatiao::library::{HostInfo, Kinds, LibraryInfo, ProviderInfo, Providers, Registry};
use guatiao::value::status::Status;
use guatiao::value::types::{MaybeNull, Str};
use guatiao::{Alloc, List, Map, TryAsRef, Value};

/// The greeter kinds, as the kind crate commits them.
const COMMITTED: &str =
    include_str!("../../../examples/greeter_kind/include/greeter_kind.kinds.json");

/// Each case: the provider's id and what it carries.
fn case(k: usize) -> (&'static str, &'static str) {
    [
        ("carries_none", ""),
        ("carries_text", r#"{"format":1,"kinds":["#),
        ("carries_greeters", COMMITTED),
    ][k]
}

const CASES: usize = 3;

struct Names([Str<'static>; 1]);
// SAFETY: a constant never written, whose pointer addresses a literal.
unsafe impl Sync for Names {}
static KINDS: Names = Names([Str::new("greeter")]);

struct Described {
    #[allow(dead_code)]
    providers: Vec<ProviderInfo<'static>>,
    desc: LibraryInfo<'static>,
}
// SAFETY: built once, never written again, every pointer addressing what
// this struct owns or a literal.
unsafe impl Sync for Described {}
// SAFETY: as above.
unsafe impl Send for Described {}

static DESCRIBED: [OnceLock<Described>; CASES] = [const { OnceLock::new() }; CASES];

/// The entry point of the library holding case `K`'s one provider.
///
/// # Safety
///
/// Called by a registry with its host's descriptor.
unsafe extern "C" fn entry<const K: usize>(
    _host: *const HostInfo<'static>,
) -> *const LibraryInfo<'static> {
    let described = DESCRIBED[K].get_or_init(|| {
        let (id, text) = case(K);
        let providers = vec![ProviderInfo {
            struct_size: size_of::<ProviderInfo>() as u32,
            vtable_size: 0,
            kinds: Kinds::new(&KINDS.0),
            id: Str::new(id),
            display_name: Str::new(""),
            config: core::ptr::null(),
            vtable: core::ptr::null(),
            ctx: core::ptr::null_mut(),
            meta: MaybeNull::null(),
            version: Str::new(""),
            available: None,
            tables: guatiao::library::KindTables::empty(),
            create: None,
            destroy: None,
            kind_descriptions: Str::new(text),
        }];
        let desc = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: guatiao::library::ABI_VERSION,
            id: Str::new(id),
            version: Str::new("1.0.0"),
            providers: Providers {
                ptr: providers.as_ptr(),
                len: providers.len(),
                stride: size_of::<ProviderInfo>(),
            },
            meta: MaybeNull::null(),
            unload: None,
        };
        Described { providers, desc }
    });
    &described.desc
}

type Entry = unsafe extern "C" fn(*const HostInfo<'static>) -> *const LibraryInfo<'static>;
const ENTRIES: [Entry; CASES] = [entry::<0>, entry::<1>, entry::<2>];

#[test]
fn a_host_reads_what_each_provider_carries() {
    let mut registry = Registry::new("descriptions", "1.0.0");
    for (k, entry) in ENTRIES.into_iter().enumerate() {
        registry
            .register_entry(case(k).0, entry)
            .expect("registers")
            .loaded()
            .expect("a library for this host");
    }
    let of = |id: &str| {
        registry
            .provider(id)
            .expect("registered")
            .kind_descriptions()
    };

    assert_eq!(of("carries_none"), Ok(Vec::new()));
    let refused = of("carries_text").unwrap_err();
    assert_eq!(refused.at(), "byte 21", "{refused}");
    let read = of("carries_greeters").unwrap();
    assert_eq!(read, KindDescription::document_json(COMMITTED).unwrap());
    assert!(read.iter().any(|k| k.name() == "greeter"));
}

/// A C caller with no JSON reader reads a kind crate's document through
/// the library, and is told where text that is not one goes wrong.
#[test]
fn a_c_caller_reads_a_document_from_its_text() {
    let alloc = Alloc::rust().as_raw();
    let read = |text: &str| {
        let (mut out, mut why) = (Value::absent(), Value::absent());
        // SAFETY: the text is readable, the allocator ours, the slots
        // writable.
        let status =
            unsafe { guatiao_kind_document_read(Str::new(text), alloc, &mut out, &mut why) };
        (status, out, why)
    };
    let (status, document, why) = read(COMMITTED);
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(why, Value::absent());
    assert_eq!(
        KindDescription::document(&document).unwrap(),
        KindDescription::document_json(COMMITTED).unwrap()
    );
    for (text, says) in [
        (r#"{"format":1,"kinds":["#, "`byte 21` is not a value"),
        (r#"{"format":2,"kinds":[]}"#, "`format` is not 1"),
        (
            r#"{"format":1,"kinds":[{"name":"k","floor_hash":1,"object":false,"slots":[{"name":"x","required":true,"fallible":false,"args":[{"name":"a","type":"u256"}]}]}]}"#,
            "`kinds[0].slots[0].args[0].type`",
        ),
    ] {
        let (status, out, why) = read(text);
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{text}");
        assert_eq!(out, Value::absent(), "{text}");
        let why: &str = TryAsRef::try_as_ref(&why).expect("why is a string");
        assert!(why.contains(says), "{why}");
    }
    // `why` may be null.
    let mut out = Value::absent();
    // SAFETY: as above, with no slot for why.
    let status = unsafe {
        guatiao_kind_document_read(Str::new("nope"), alloc, &mut out, core::ptr::null_mut())
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
}

#[test]
fn a_c_host_reads_the_document_or_absent_and_each_refusal_has_its_status() {
    let alloc = Alloc::rust().as_raw();
    // SAFETY: the names are readable for the call; the allocator is ours.
    let reg = unsafe { guatiao_registry_new(Str::new("c-descriptions"), Str::new("1.0"), alloc) };
    assert!(!reg.is_null());
    for (k, entry) in ENTRIES.into_iter().enumerate() {
        let mut report = Value::absent();
        // SAFETY: `reg` is live, the name readable, `report` writable.
        let status = unsafe {
            guatiao_registry_register_entry(
                reg,
                Str::new(case(k).0),
                Some(entry),
                alloc,
                &mut report,
            )
        };
        assert_eq!(status, Status::GUATIAO_OK);
    }
    let ask = |id: &str| {
        let mut out = Value::absent();
        // SAFETY: `reg` is live, `id` readable, `out` writable.
        let status =
            unsafe { guatiao_registry_kind_descriptions(reg, Str::new(id), alloc, &mut out) };
        (status, out)
    };

    let (status, out) = ask("carries_none");
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out, Value::absent());
    assert_eq!(ask("carries_text").0, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(ask("nobody").0, Status::GUATIAO_ERR_NOT_FOUND);

    let (status, document) = ask("carries_greeters");
    assert_eq!(status, Status::GUATIAO_OK);
    let map: &Map = TryAsRef::try_as_ref(&document).expect("the document is a map");
    let kinds: &List = TryAsRef::try_as_ref(&map["kinds"]).expect("a list of kinds");
    assert_eq!(
        KindDescription::document(&document).unwrap(),
        KindDescription::document_json(COMMITTED).unwrap()
    );
    assert!(!kinds.is_empty());
    drop(document);

    // SAFETY: `reg` came from `guatiao_registry_new` and is not used again.
    unsafe { guatiao_registry_free(reg) };
}
