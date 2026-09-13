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
//! # `struct_size` first, and appended fields only
//!
//! Exactly the arrangement [`crate::value::Allocator`] uses, for the same
//! reason and with the same rule: the leading `u32` is the size of the
//! struct **as the side that wrote it compiled it**, a frozen `floor()`
//! says how much must be present to be usable at all, and each field
//! added later carries its own guard.
//!
//! **A slot is appended, never changed.** `struct_size` cannot version a
//! signature: there is no size at which an old caller stops short of a
//! changed argument list, so the field is present, the pointer is called,
//! and a two-argument callback invoked with three arguments is silent
//! memory corruption in every already-compiled consumer. Adding a second
//! slot beside the first, with null meaning "use the old one", is the
//! only change that is safe.

#![forbid(unsafe_code)]
#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::mem::{offset_of, size_of};

use crate::value::alloc::Allocator;
use crate::value::types::{Map, MaybeNull, Str, Value};

/// The envelope's own version, for a library that wants to refuse a host
/// it does not understand.
///
/// Bumped only for a change no `struct_size` guard can express. Appending
/// a field is not such a change.
pub const ABI_VERSION: u32 = 1;

/// What the host tells a library about itself, on the way in.
///
/// The allocator is here because a library that wants to build a tree the
/// host will keep can build it in the host's own arena, and then the host
/// frees it with nothing to remember. A library that would rather use its
/// own passes its own; every owned container records which it was.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostInfo {
    /// `sizeof(guatiao_host_info)` as the host compiled it. Always first.
    pub struct_size: u32,
    /// The envelope version the host speaks. See [`ABI_VERSION`].
    pub abi_version: u32,
    /// Who the host is, for a library that registers different providers
    /// for different hosts.
    pub host_id: Str,
    /// The host's own version string, uninterpreted.
    pub host_version: Str,
    /// The host's allocator, or null. Borrowed for the call and for as
    /// long as anything built through it lives.
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
}

impl HostInfo {
    /// The smallest `struct_size` that can be used at all.
    ///
    /// Frozen at the first field added after v1, and never moved. A floor
    /// that tracked the newest field would refuse every host compiled
    /// before it existed, which is the failure the guard exists to
    /// prevent rather than to cause.
    pub const fn floor() -> usize {
        offset_of!(HostInfo, host_version) + size_of::<Str>()
    }

    /// Where the `alloc` field ends, for the guard that reads it.
    pub const fn alloc_end() -> usize {
        offset_of!(HostInfo, alloc) + size_of::<*const Allocator>()
    }

    /// Where the `meta` field ends, for the guard that reads it.
    pub const fn meta_end() -> usize {
        offset_of!(HostInfo, meta) + size_of::<MaybeNull<Map>>()
    }
}

/// A borrowed sequence of provider descriptors.
///
/// A view, like every other `{ptr, len}` in this crate: the library owns
/// the array.
///
/// # Why it carries a stride
///
/// **`struct_size` cannot version an array's element size.** Every
/// descriptor here declares its own size and a reader guards each
/// appended field against it — but that only places the fields *within*
/// an element. Finding element `i` needs the size the LIBRARY laid the
/// array out at, and a reader that assumed its own would land inside an
/// element built before its newest slot existed, read whatever sits at
/// that offset as a `struct_size`, and guard every field against a number
/// it invented.
///
/// So the library states the stride and a reader walks by bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Providers {
    /// First descriptor. May be null when `len` is 0.
    pub ptr: *const ProviderInfo,
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
/// field. A [`Str`] cannot: it declares no `struct_size`, so it has no
/// mechanism to grow through and its layout is frozen by definition.
/// Where there is no versioning there is no version skew, and the element
/// size is the same number on both sides of the boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Kinds {
    /// First name. May be null when `len` is 0.
    pub ptr: *const Str,
    /// How many.
    pub len: usize,
}

impl Kinds {
    /// A borrowed array of names.
    pub const fn new(kinds: &'static [Str]) -> Kinds {
        Kinds {
            ptr: kinds.as_ptr(),
            len: kinds.len(),
        }
    }

    /// No kinds: a provider that serves no vtable, reached by name alone.
    pub const fn empty() -> Kinds {
        Kinds {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}

impl Providers {
    /// A borrowed array, with the stride this build lays it out at.
    ///
    /// The way to write one: a hand-set stride is a number to get wrong
    /// exactly once.
    pub const fn new(providers: &'static [ProviderInfo]) -> Providers {
        Providers {
            ptr: providers.as_ptr(),
            len: providers.len(),
            stride: size_of::<ProviderInfo>(),
        }
    }

    /// No providers, which is a library that loaded and had nothing for
    /// this host.
    pub const fn empty() -> Providers {
        Providers {
            ptr: std::ptr::null(),
            len: 0,
            stride: size_of::<ProviderInfo>(),
        }
    }
}

/// What a library says about itself, on the way out.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LibraryInfo {
    /// `sizeof(guatiao_library_info)` as the library compiled it.
    pub struct_size: u32,
    /// The envelope version the library speaks.
    pub abi_version: u32,
    /// A stable identifier for the library itself, for diagnostics and
    /// for refusing to load the same one twice.
    pub id: Str,
    /// Its version, declared **semver**.
    ///
    /// A field a host can name in the key it files providers under
    /// (`%version`), which is what lets two builds of one provider be
    /// loaded at once. This crate compares it as a string and never parses
    /// it — ordering is a host's policy, applied with the semver library
    /// it already has.
    pub version: Str,
    /// Everything it offers. May be empty, which is a library that
    /// loaded and had nothing for this host.
    pub providers: Providers,
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
}

impl LibraryInfo {
    /// The smallest usable `struct_size`.
    pub const fn floor() -> usize {
        offset_of!(LibraryInfo, providers) + size_of::<Providers>()
    }

    /// Where the `meta` field ends, for the guard that reads it.
    pub const fn meta_end() -> usize {
        offset_of!(LibraryInfo, meta) + size_of::<MaybeNull<Map>>()
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
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProviderInfo {
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
    pub kinds: Kinds,
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
    pub id: Str,
    /// A name to show a person. May be empty, and a host that shows
    /// nothing to anybody ignores it.
    pub display_name: Str,
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
    pub version: Str,
}

impl ProviderInfo {
    /// The smallest usable `struct_size`.
    pub const fn floor() -> usize {
        offset_of!(ProviderInfo, ctx) + size_of::<*mut c_void>()
    }

    /// Where the `meta` field ends, for the guard that reads it.
    pub const fn meta_end() -> usize {
        offset_of!(ProviderInfo, meta) + size_of::<MaybeNull<Map>>()
    }

    /// Where the `version` field ends, for the guard that reads it.
    pub const fn version_end() -> usize {
        offset_of!(ProviderInfo, version) + size_of::<Str>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A floor must sit at or below the current size, or appending a
    /// field locks out every caller compiled before it.
    ///
    /// The same assertion `Allocator` carries, for the same reason and in
    /// the same shape.
    #[test]
    fn every_floor_sits_within_its_struct() {
        assert!(HostInfo::floor() <= size_of::<HostInfo>());
        assert!(HostInfo::alloc_end() <= size_of::<HostInfo>());
        assert!(LibraryInfo::floor() <= size_of::<LibraryInfo>());
        assert!(ProviderInfo::floor() <= size_of::<ProviderInfo>());
    }

    /// The leading field is at offset zero in every one of them, because
    /// a reader that cannot find `struct_size` cannot find anything.
    #[test]
    fn struct_size_is_first() {
        assert_eq!(offset_of!(HostInfo, struct_size), 0);
        assert_eq!(offset_of!(LibraryInfo, struct_size), 0);
        assert_eq!(offset_of!(ProviderInfo, struct_size), 0);
    }
}
