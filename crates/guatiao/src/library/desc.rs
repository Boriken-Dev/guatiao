// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a library says about itself, and what a host says about itself.
//!
//! Three `repr(C)` structs and one view. A C library fills them in as
//! static data; a Rust library fills them in behind the `guatiao_library!`
//! macro. Neither needs this crate's mutation functions to do it.
//!
//! # Everything here is BORROWED
//!
//! Not one field is an owned container. A descriptor is data the library
//! owns for as long as it is loaded, nobody else grows it, and nothing
//! frees it — so there is no allocator in any of these and no question
//! about who releases what. The values a provider *produces* are owned
//! and carry their allocator; the description of the provider is not.
//!
//! # `struct_size` first, and every field this build knows required
//!
//! The arrangement [`crate::value::Allocator`] uses: the leading `u32` is
//! the size of the struct **as the side that wrote it compiled it**, and
//! `floor()` is this build's whole struct. A smaller `struct_size` is
//! refused; a larger one is a struct with fields this build does not
//! know, read up to its own.
//!
//! **A field is appended, never changed.** `struct_size` cannot version a
//! signature: there is no size at which a reader stops short of a changed
//! argument list, so a two-argument callback invoked with three arguments
//! would be silent memory corruption. A change no appended field can
//! express bumps [`ABI_VERSION`].
//!
//! # A descriptor's fields are its producer's contract
//!
//! Loading a library, or registering one a host links, is the decision to
//! trust what it wrote. A reader then checks what can be checked where
//! the descriptor enters, and refuses it with a sentence
//! (`LoadError::Malformed`, `KindMismatch`) rather than reading through a
//! fault:
//!
//! - `struct_size` against each floor, `abi_version`, each element's size
//!   against its array's stride;
//! - every non-null pointer aligned for what it points at, outside the
//!   low 64 KiB, and canonical for the architecture;
//! - every length × stride addressable, and capped: 4096 providers, 256
//!   kind names and 256 per-kind tables per provider, strides of at most
//!   4096 bytes, text of at most 64 KiB, tables of at most 64 KiB;
//! - text is UTF-8; a per-kind table's slots after its header are each
//!   null or a plausible function pointer, and its size, hash and
//!   required slots are checked when a kind asks for it;
//! - for a library mapped from a file, a data pointer starting inside its
//!   image ends inside it (one outside it, on the library's heap, is
//!   accepted), and every function pointer — `available`, `create`,
//!   `destroy`, `unload`, each per-kind table's slots — lies inside the
//!   image's executable sections.
//!
//! These catch a wild or stale pointer and a corrupt length. They do not
//! prove that a table is one for the kind it names or that a slot
//! expects its `ctx`; that is the producer's contract.

#![forbid(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::c_void;
use core::mem::size_of;

use super::kind::ProviderError;
use crate::value::alloc::Allocator;
use crate::value::status::Status;
use crate::value::types::{List, Map, MaybeNull, Str, Value};

/// The envelope's own version. A descriptor naming another is refused.
///
/// Bumped only for a change appending a field cannot express.
pub const ABI_VERSION: u32 = 1;

/// What the host tells a library about itself, on the way in.
///
/// The allocator is here because a library that wants to build a tree the
/// host will keep can build it in the host's own arena, and then the host
/// frees it with nothing to remember. A library that would rather use its
/// own passes its own; every owned container records which it was.
///
/// **Valid for the life of the process.** A host hands a library a pointer
/// to a block it never frees, so a library may keep the pointer (as a
/// [`Host`](super::raw::Host)) and read it from any later call. What the
/// block answers about the registry changes as the host loads and ranks;
/// once the host's registry is gone, every lookup through it answers
/// `GUATIAO_ERR_GONE` and touches nothing freed.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostInfo<'a> {
    /// `sizeof(guatiao_host_info)` as the host compiled it. Always first.
    pub struct_size: u32,
    /// The envelope version the host speaks. See [`ABI_VERSION`].
    pub abi_version: u32,
    /// Who the host is, for a library that registers different providers
    /// for different hosts.
    pub host_id: Str<'a>,
    /// The host's own version string, uninterpreted.
    pub host_version: Str<'a>,
    /// The host's allocator, or null. Valid for the life of the process,
    /// like the block it sits in, and it must be: every tree built through
    /// it records this address.
    pub alloc: *const Allocator,
    /// Anything else this host wants to say, as an ordinary value, or
    /// null. Conventionally a map.
    ///
    /// The escape hatch every envelope needs and no envelope can specify:
    /// a build id, a capability flag, a vendor's own key. A reader that
    /// does not know a key skips it, which is the rule the value model
    /// already has for a tag it does not know.
    ///
    /// **A pointer, because this struct is `Copy`** and every reader
    /// projects its fields with a bitwise read. A [`Map`] held INLINE
    /// would be copied by each of those reads, and a `Map` frees what it
    /// owns on drop — so every copy would be a second owner of one
    /// buffer. A pointer has no drop glue whatever it addresses.
    ///
    /// **Non-null means a well-formed map.** Nothing here checks that,
    /// and a library that writes a non-null pointer to anything else has
    /// caused undefined behaviour at the read. It is the same class of
    /// promise as [`vtable`](ProviderInfo::vtable), whose shape only the
    /// `kind` knows, and as the allocator's outliving everything built
    /// through it.
    ///
    /// Unlike [`config`](ProviderInfo::config), null and an empty map mean
    /// the same thing here — "nothing to add" has no second reading
    /// worth keeping apart.
    pub meta: MaybeNull<Map>,
    /// What the host will answer a library that asks, or null for a host
    /// that answers nothing. See [`HostServices`].
    pub services: *const HostServices,
}

impl HostInfo<'_> {
    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<HostInfo>()
    }
}

/// What a host answers a library that asks it something: the lookups its
/// registry can do, and its allocator, as slots a library calls.
///
/// **Every answer is a descriptor the host built** from what the offering
/// library declared: its own copies of the text, values and tables, the
/// library's code and `ctx`. `get` and `list` answer one that stays
/// valid until that library is retired. `get_pinned` and `list_pinned`
/// answer one **pinned**: it stays valid, and the host will not unload
/// the library it names, until the caller passes it to `unpin`; `pin`
/// takes another hold on one already pinned. A library that keeps an
/// answer, or calls through one, past a retire it cannot see pins it.
///
/// A host that has been dropped answers `GUATIAO_ERR_GONE` from every
/// lookup and touches nothing it freed; a pinned answer stays valid.
///
/// Slots are spelled out inline rather than through a type alias: cbindgen
/// renders an aliased function-pointer field as an opaque struct used by
/// value, which is an incomplete type that compiles nowhere.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostServices {
    /// `sizeof(guatiao_host_services)` as the host compiled it. Always
    /// first.
    pub struct_size: u32,
    /// Passed back to every slot below. Opaque to a library.
    pub ctx: *mut c_void,
    /// One provider by the key the host files it under, or
    /// `GUATIAO_ERR_NOT_FOUND`. Writes the descriptor's address through
    /// `out`.
    pub get: Option<
        unsafe extern "C" fn(ctx: *mut c_void, key: Str, out: *mut *const ProviderInfo) -> Status,
    >,
    /// Every provider serving `kind` — every one, when `kind` is empty —
    /// in the host's own order (`priority DESC, key ASC`), **including
    /// providers that are not available**: the caller asks each and
    /// chooses. Fills up to `cap` entries at `out` and writes the total
    /// through `total`; call with `cap = 0` to size a buffer.
    pub list: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            kind: Str,
            out: *mut *const ProviderInfo,
            cap: usize,
            total: *mut usize,
        ) -> Status,
    >,
    /// The same allocator as `HostInfo::alloc`, for code that kept only
    /// this table. Null for a host that offers none.
    pub alloc: Option<unsafe extern "C" fn(ctx: *mut c_void) -> *const Allocator>,
    /// `get`, answering a descriptor pinned for the caller: release it
    /// with `unpin`.
    pub get_pinned: Option<
        unsafe extern "C" fn(ctx: *mut c_void, key: Str, out: *mut *const ProviderInfo) -> Status,
    >,
    /// `list`, each of the first `cap` descriptors it writes pinned for
    /// the caller: release each with `unpin`. The total counts every
    /// provider, as `list`'s does.
    pub list_pinned: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            kind: Str,
            out: *mut *const ProviderInfo,
            cap: usize,
            total: *mut usize,
        ) -> Status,
    >,
    /// Another hold on a descriptor the caller already holds pinned.
    pub pin: Option<unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo)>,
    /// Gives back one hold on a pinned descriptor; the caller does not
    /// read it again unless it holds another.
    pub unpin: Option<unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo)>,
}

impl HostServices {
    /// The smallest `struct_size` this build reads: every slot it knows.
    pub const fn floor() -> usize {
        size_of::<HostServices>()
    }
}

/// A borrowed sequence of provider descriptors.
///
/// A view, like every other `{ptr, len}` in this crate: the library owns
/// the array.
///
/// # Why it carries a stride
///
/// **`struct_size` places the fields within one element, not the
/// elements.** A library built with fields appended after this build's
/// lays its elements further apart, so it states the stride and a reader
/// walks by bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Providers<'a> {
    /// First descriptor. May be null when `len` is 0.
    pub ptr: *const ProviderInfo<'a>,
    /// How many.
    pub len: usize,
    /// `sizeof(guatiao_provider_info)` as the LIBRARY compiled it, which
    /// is the array's stride in bytes.
    ///
    /// Always the element size, even for one element: a reader refuses a
    /// stride below [`ProviderInfo::floor`], and refuses an element whose
    /// own `struct_size` exceeds it, which would overlap its neighbour.
    pub stride: usize,
}

/// A borrowed sequence of kind names.
///
/// # Why this one carries no stride
///
/// [`Providers`] states its stride because a `ProviderInfo` can grow a
/// field. A [`Str`] cannot: it declares no `struct_size`, so its element
/// size is the same number on both sides of the boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Kinds<'a> {
    /// First name. May be null when `len` is 0.
    pub ptr: *const Str<'a>,
    /// How many.
    pub len: usize,
}

impl<'a> Kinds<'a> {
    /// A borrowed array of names.
    pub const fn new(kinds: &'a [Str<'a>]) -> Kinds<'a> {
        Kinds {
            ptr: kinds.as_ptr(),
            len: kinds.len(),
        }
    }

    /// No kinds: a provider that serves no vtable, reached by name alone.
    pub const fn empty() -> Kinds<'a> {
        Kinds {
            ptr: core::ptr::null(),
            len: 0,
        }
    }
}

impl<'a> Providers<'a> {
    /// A borrowed array, with the stride this build lays it out at.
    ///
    /// The way to write one: a hand-set stride is a number to get wrong
    /// exactly once.
    pub const fn new(providers: &'a [ProviderInfo<'a>]) -> Providers<'a> {
        Providers {
            ptr: providers.as_ptr(),
            len: providers.len(),
            stride: size_of::<ProviderInfo>(),
        }
    }

    /// No providers, which is a library that loaded and had nothing for
    /// this host.
    pub const fn empty() -> Providers<'a> {
        Providers {
            ptr: core::ptr::null(),
            len: 0,
            stride: size_of::<ProviderInfo>(),
        }
    }
}

/// What a library says about itself, on the way out.
///
/// Its fields, and those of every provider it lists, are its producer's
/// contract: a host checks their sizes, pointers and lengths where the
/// descriptor enters, and refuses one that fails.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LibraryInfo<'a> {
    /// `sizeof(guatiao_library_info)` as the library compiled it.
    pub struct_size: u32,
    /// The envelope version the library speaks.
    pub abi_version: u32,
    /// A stable identifier for the library itself, for diagnostics and
    /// for refusing to load the same one twice.
    pub id: Str<'a>,
    /// Its version, declared **semver**.
    ///
    /// A field a host can name in the key it files providers under
    /// (`%version`), which is what lets two builds of one provider be
    /// loaded at once. This crate compares it as a string and never parses
    /// it — ordering is a host's policy, applied with the semver library
    /// it already has.
    pub version: Str<'a>,
    /// Everything it offers. May be empty, which is a library that
    /// loaded and had nothing for this host.
    pub providers: Providers<'a>,
    /// Anything else this library wants to say, as an ordinary value, or
    /// null. Conventionally a map.
    ///
    /// The escape hatch every envelope needs and no envelope can specify:
    /// a build id, a capability flag, a vendor's own key. A reader that
    /// does not know a key skips it, which is the rule the value model
    /// already has for a tag it does not know.
    ///
    /// **A pointer, because this struct is `Copy`** and every reader
    /// projects its fields with a bitwise read. A [`Map`] held INLINE
    /// would be copied by each of those reads, and a `Map` frees what it
    /// owns on drop — so every copy would be a second owner of one
    /// buffer. A pointer has no drop glue whatever it addresses.
    ///
    /// **Non-null means a well-formed map.** Nothing here checks that,
    /// and a library that writes a non-null pointer to anything else has
    /// caused undefined behaviour at the read. It is the same class of
    /// promise as [`vtable`](ProviderInfo::vtable), whose shape only the
    /// `kind` knows, and as the allocator's outliving everything built
    /// through it.
    ///
    /// Unlike [`config`](ProviderInfo::config), null and an empty map mean
    /// the same thing here — "nothing to add" has no second reading
    /// worth keeping apart.
    pub meta: MaybeNull<Map>,
    /// The library's say in being unmapped, or null for one that may be
    /// unmapped without notice.
    ///
    /// A host calls this before it closes the mapping. `GUATIAO_OK` means
    /// go ahead; any other status refuses, and the host leaves the library
    /// registered and mapped. **This is the one thing the host cannot
    /// see**: a thread still running, a callback still registered
    /// elsewhere, values this library's allocator made that somebody still
    /// holds.
    ///
    /// Takes no context: a library asking about itself already has its
    /// own state.
    pub unload: Option<unsafe extern "C" fn() -> Status>,
}

impl LibraryInfo<'_> {
    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<LibraryInfo>()
    }
}

/// One thing a library offers.
///
/// The envelope defines **no vtable of its own**. Whoever defines a
/// `kind` defines what its vtable looks like, and this carries the
/// pointer and the size the library compiled it at — so a host that knows
/// the kind can check the size the same way it checks an allocator's, and
/// a host that does not know the kind can list the provider without ever
/// looking at the pointer.
///
/// Its fields are its producer's contract: a host checks their sizes,
/// pointers and lengths where the descriptor enters, and refuses one that
/// fails.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProviderInfo<'a> {
    /// `sizeof(guatiao_provider_info)` as the library compiled it.
    pub struct_size: u32,
    /// Size of the struct `vtable` points at, as the library compiled it.
    /// Zero when there is no vtable.
    pub vtable_size: u32,
    /// What sorts of thing this is: `"greeter"`, `"codec"`, whatever the
    /// host and the library have agreed.
    ///
    /// **A list, because one provider commonly serves several.** A crate
    /// that both discovers hosts and opens sessions to them is one
    /// implementation with one identity, and registering it twice to say
    /// so would make it two providers a host has to know are the same.
    ///
    /// Each is a **capability**, never a name: it says which vtable this
    /// provider speaks, so a host can ask for everything that speaks one.
    /// What identifies the provider is `id`. Empty is legal and means a
    /// provider that serves no vtable — pure data, reached by name.
    pub kinds: Kinds<'a>,
    /// This provider's own identifier, **unique across every provider a
    /// host loads**.
    ///
    /// Names an implementation rather than a protocol: `"pve_qemu"`,
    /// `"mdns"`. Conventionally `{library id}_{name}`, where the name is
    /// whatever the implementation is called in its own source — which is
    /// what makes an id unique without any central register.
    ///
    /// **A re-export keeps the ORIGINAL id.** Two libraries offering one
    /// provider is an ordinary thing; that they agree on its id is what
    /// lets a host notice it already has it and skip the second, rather
    /// than loading one implementation twice under two names.
    ///
    /// What a host files it under is that host's own key template, `%id`
    /// by default.
    pub id: Str<'a>,
    /// A name to show a person. May be empty, and a host that shows
    /// nothing to anybody ignores it.
    pub display_name: Str<'a>,
    /// The schema for this provider's configuration, as an ordinary
    /// value, or null when it takes none.
    ///
    /// Null rather than an empty map, so "declares no configuration" and
    /// "declares an empty one" stay different statements.
    pub config: *const Value,
    /// The function table, whose shape is the `kind`'s business. Null
    /// when the kind is pure data.
    pub vtable: *const c_void,
    /// Passed back to every call through the vtable, untouched.
    pub ctx: *mut c_void,
    /// Anything else this provider wants to say, as an ordinary value, or
    /// null. Conventionally a map.
    ///
    /// The escape hatch every envelope needs and no envelope can specify:
    /// a build id, a capability flag, a vendor's own key. A reader that
    /// does not know a key skips it, which is the rule the value model
    /// already has for a tag it does not know.
    ///
    /// **A pointer, because this struct is `Copy`** and every reader
    /// projects its fields with a bitwise read. A [`Map`] held INLINE
    /// would be copied by each of those reads, and a `Map` frees what it
    /// owns on drop — so every copy would be a second owner of one
    /// buffer. A pointer has no drop glue whatever it addresses.
    ///
    /// **Non-null means a well-formed map.** Nothing here checks that,
    /// and a library that writes a non-null pointer to anything else has
    /// caused undefined behaviour at the read. It is the same class of
    /// promise as [`vtable`](ProviderInfo::vtable), whose shape only the
    /// `kind` knows, and as the allocator's outliving everything built
    /// through it.
    ///
    /// Unlike [`config`](ProviderInfo::config), null and an empty map mean
    /// the same thing here — "nothing to add" has no second reading
    /// worth keeping apart.
    pub meta: MaybeNull<Map>,
    /// This provider's own version, or **empty to inherit the library's**.
    ///
    /// Empty is the common case and the right default: a provider shipped
    /// in its own library versions with it, and saying so twice is two
    /// numbers to keep in step. A provider states its own when it does not
    /// move with its library — a re-exported one, or one whose contract
    /// froze while the library around it went on.
    ///
    /// Declared **semver**. This crate compares it as a string and never
    /// parses it: ordering is a host's policy, applied with the semver
    /// library it already has.
    pub version: Str<'a>,
    /// Can this provider actually run **here, right now** — and if not,
    /// why not?
    ///
    /// Null means yes. A library that does not implement this is
    /// available, which is the right default and the common case: most
    /// providers are code that either loaded or did not.
    ///
    /// Otherwise it returns `true` for available and leaves `reason`
    /// untouched, or `false` and writes a borrowed [`Str`] through
    /// `reason` saying why. **A boolean return rather than an empty
    /// string**, so "available" and "unavailable, with nothing to say
    /// about it" stay different answers.
    ///
    /// # What this envelope does and does not define
    ///
    /// It defines the **slot**: the signature, when the reason is written,
    /// and how long it lives. It does **not** define what "available"
    /// means, when a host should ask, or what a host should do with a
    /// refusal — those are between a host and a library, like everything
    /// else a `kind` agrees.
    ///
    /// What this crate promises is only that it **never caches the
    /// answer**. A library may load an optional dependency, lose a device,
    /// or fail its own integrity check while a process runs, and a host
    /// that asked once at load time would be holding an answer from before
    /// any of that.
    ///
    /// # The reason must outlive every reader
    ///
    /// Point it at a literal in the library's image, or at something
    /// leaked once. **Never at a buffer shared by callers**: two threads
    /// asking the same provider at the same time would each read whatever
    /// the other had just written, and what comes back is a fragment with
    /// nothing reporting it. Immortal, per-call or per-thread — but not
    /// one slot everybody writes.
    ///
    /// Spelled out inline rather than through a type alias: cbindgen
    /// renders an aliased function-pointer field as an opaque struct used
    /// by value, which is an incomplete type that compiles nowhere.
    pub available: Option<unsafe extern "C" fn(ctx: *mut c_void, reason: *mut Str) -> bool>,
    /// One function table per kind, for a provider serving several kinds
    /// with a table each. See [`KindTables`]. May be empty: `vtable`
    /// above then serves every kind in `kinds`, which is how a provider
    /// with one table for everything, or none, declares itself.
    ///
    /// A reader asks [`ProviderView::table_for`](super::raw::ProviderView::table_for):
    /// a table here for the kind first, then `vtable` when `kinds` names
    /// the kind.
    pub tables: KindTables<'a>,
    /// Builds an **instance** from a configuration, or null for a provider
    /// that is its one instance.
    ///
    /// `ctx` is this descriptor's own `ctx`. `config` is a value fitting
    /// [`config`](ProviderInfo::config). On `GUATIAO_OK` the instance is
    /// written through `out`, and **that pointer is the `ctx` every
    /// kind-table slot takes** for calls on this instance; on any other
    /// status `err` may carry the provider's own words. A provider may
    /// still offer a default instance through `ctx` beside the ones it
    /// builds, or leave `ctx` null: instances only.
    pub create: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            config: *const Value,
            out: *mut *mut c_void,
            err: *mut ProviderError,
        ) -> Status,
    >,
    /// Releases an instance `create` built. Null when `create` is null.
    pub destroy: Option<unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void)>,
    /// The descriptions of the kinds this provider serves, and of every
    /// object kind their slots take or answer, each once: one document
    /// `{"kinds":[...]}` as JSON text, the encoding `docs/encodings.md`
    /// states ("A kind, described"). **Empty when the provider carries
    /// none.**
    ///
    /// What lets a host bind a kind it has no file for. `#[derive(Provider)]`
    /// writes it; a provider written by hand may leave it empty. The text
    /// outlives every reader, as `id` does.
    pub kind_descriptions: Str<'a>,
}

/// One kind's function table, on a provider that serves several kinds
/// with a table each.
///
/// The table `vtable` points at is a kind's header followed by function
/// pointers, each null or one a reader accepts as code.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KindTable<'a> {
    /// `sizeof(guatiao_kind_table)` as the library compiled it. Always
    /// first.
    pub struct_size: u32,
    /// Size of the struct `vtable` points at, as the library compiled it.
    pub vtable_size: u32,
    /// The kind this table serves.
    pub kind: Str<'a>,
    /// The table, whose shape the kind defines.
    pub vtable: *const c_void,
}

impl<'a> KindTable<'a> {
    /// A table for `kind`, with the size this build lays it out at.
    pub const fn new(kind: &'a str, vtable: *const c_void, vtable_size: usize) -> KindTable<'a> {
        KindTable {
            struct_size: size_of::<KindTable>() as u32,
            vtable_size: vtable_size as u32,
            kind: Str::new(kind),
            vtable,
        }
    }

    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<KindTable>()
    }
}

/// A borrowed array of [`KindTable`], carrying its stride like
/// [`Providers`] and for the same reason: `struct_size` places fields
/// within one element, and only the library knows how far apart the
/// elements are.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KindTables<'a> {
    /// First table. May be null when `len` is 0.
    pub ptr: *const KindTable<'a>,
    /// How many.
    pub len: usize,
    /// `sizeof(guatiao_kind_table)` as the LIBRARY compiled it.
    pub stride: usize,
}

impl<'a> KindTables<'a> {
    /// A borrowed array, with the stride this build lays it out at.
    pub const fn new(tables: &'a [KindTable<'a>]) -> KindTables<'a> {
        KindTables {
            ptr: tables.as_ptr(),
            len: tables.len(),
            stride: size_of::<KindTable>(),
        }
    }

    /// No per-kind tables: `vtable` serves every kind.
    pub const fn empty() -> KindTables<'a> {
        KindTables {
            ptr: core::ptr::null(),
            len: 0,
            stride: size_of::<KindTable>(),
        }
    }
}

impl ProviderInfo<'_> {
    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<ProviderInfo>()
    }
}

/// What a **source** answers: a table a registry keeps for its life, through
/// which it learns which libraries the source can serve now and opens and
/// closes them.
///
/// A source is what serves libraries that are not files a scan can probe:
/// the modules of an interpreter, the entries of a manifest, whatever a
/// bridge to another language holds. The source is added once
/// (`guatiao_registry_add_source`, or a file exporting `guatiao_source`
/// through `guatiao_registry_add_source_file`) and never let go; the
/// libraries it opens are ordinary libraries, retired and closed like any
/// other. A registry calls these slots one at a time per source, from
/// whichever thread calls the registry.
///
/// Its fields are its producer's contract, checked where the table enters
/// as a library descriptor's are.
///
/// A source FILE exports
/// `guatiao_status guatiao_source(const guatiao_host_info *host, const
/// guatiao_value *config, const guatiao_source_info **out,
/// guatiao_provider_error *err)`, which answers a table configured by
/// `config` (the map the host passed, never null) or refuses with a status
/// and words in `err`; the table, its slots and its `ctx` stay valid for
/// the life of the process. It declares `source=<what it serves>` in its
/// `guatiao_declares`, so a scan reports it and never loads it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SourceInfo {
    /// `sizeof(guatiao_source_info)` as the source compiled it. Always
    /// first.
    pub struct_size: u32,
    /// The envelope version the source speaks. See [`ABI_VERSION`].
    pub abi_version: u32,
    /// Passed back to every slot below. Opaque to the registry.
    pub ctx: *mut c_void,
    /// What the source can open now, **without running any provider code**:
    /// writes a list into `out`, one map per candidate,
    /// `{"id": text, "version": text, "kinds": [text], "stamp": text}`.
    /// `stamp` is text the source changes whenever the candidate does (a
    /// modification time, a hash); the registry compares it and never
    /// reads it.
    ///
    /// `out` arrives holding a carrier, an empty list whose `alloc` names
    /// the allocator to build in, as a kind slot's out slot does; write the
    /// list over it without freeing it. A non-OK status is the source
    /// failing to list at all, reported by the refresh that asked.
    pub list: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            host: *const HostInfo<'static>,
            out: *mut List,
        ) -> Status,
    >,
    /// Opens the candidate listed as `id`: writes its library descriptor
    /// through `out`, which stays valid until `close` is called for it.
    /// The descriptor's `id` must be `id`. A non-OK status refuses, with
    /// the source's words in `err`; the refresh reports it and nothing is
    /// loaded.
    pub open: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            host: *const HostInfo<'static>,
            id: Str,
            out: *mut *const LibraryInfo<'static>,
            err: *mut ProviderError,
        ) -> Status,
    >,
    /// Called once for every descriptor `open` answered, after the
    /// registry closed that library: nothing it offered is held any more
    /// and its own `unload` slot, if it has one, agreed. `info` is the
    /// descriptor `open` answered, which names this open when two builds of
    /// one `id` are open at once. Null for a source with nothing to let go.
    pub close:
        Option<unsafe extern "C" fn(ctx: *mut c_void, id: Str, info: *const LibraryInfo<'static>)>,
}

impl SourceInfo {
    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<SourceInfo>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    /// The leading field is at offset zero in every one of them, because
    /// a reader that cannot find `struct_size` cannot find anything.
    #[test]
    fn struct_size_is_first() {
        assert_eq!(offset_of!(HostInfo, struct_size), 0);
        assert_eq!(offset_of!(LibraryInfo, struct_size), 0);
        assert_eq!(offset_of!(ProviderInfo, struct_size), 0);
        assert_eq!(offset_of!(HostServices, struct_size), 0);
        assert_eq!(offset_of!(KindTable, struct_size), 0);
        assert_eq!(offset_of!(SourceInfo, struct_size), 0);
    }
}
