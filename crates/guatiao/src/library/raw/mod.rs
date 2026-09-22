// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The loader's `unsafe`, in one directory.
//!
//! Everything that reads a descriptor through a raw pointer, everything
//! that opens a library, and the macro that writes a library's entry
//! point, in one place — so the module around it stays provably safe and
//! an auditor's scope is this directory: `entry.rs` is what a library
//! writes, `host.rs` the block a host hands over, `image.rs` reading what a
//! library's descriptors say.
//!
//! # The mapping is the registry's, and stays until the host says
//!
//! `open` hands the handle back and the registry holds it, so a
//! library stays mapped until the host retires or unloads it. A tree a
//! library built through its own allocator, and every vtable, `ctx` and
//! descriptor pointer taken from it, address that mapping — which is why
//! retiring keeps it and only `unload` closes it, on the host's word.
//!
//! # Loading is an irreversible probe
//!
//! There is no way to ask whether a file exports a symbol without mapping
//! it, and mapping it runs its static initialisers, which may do anything
//! including abort the process. A loader therefore never opens a file the
//! caller did not name or a directory the caller did not curate.

#![allow(non_camel_case_types)]
// The descriptor readers are used by the loader, which is behind a
// feature. A library AUTHOR compiles this file for the macro and the entry
// type alone, so without that feature they are genuinely unused — and
// four `#[cfg]`s on items that are correct either way would be four more
// things to keep in step than saying it once.
#![cfg_attr(not(feature = "load"), allow(dead_code))]

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::desc::{HostInfo, HostServices, KindTable, LibraryInfo, ProviderInfo};
use crate::value::alloc::Alloc;
use crate::value::status::Status;
use crate::value::types::{Map, MaybeNull, Str, Value};

mod entry;
mod host;
mod image;

pub use self::entry::*;
pub use self::host::*;
pub use self::image::*;

/// What the tests of every file here build descriptors with.
#[cfg(test)]
mod test_support {
    use super::*;

    /// The byte every descriptor's tail is filled with.
    ///
    /// Non-zero on purpose. A tail of zeroes reads back as a null pointer
    /// and a zero size, which is exactly what a correct guard answers — so
    /// a reader with NO guard would pass every assertion below. `0xAA`
    /// makes the two outcomes different.
    pub(super) const TAIL: u8 = 0xAA;

    /// A descriptor of `T`, `declared` bytes of it real and the rest
    /// filled with [`TAIL`].
    ///
    /// Returns the backing store as well: the pointer is into it, so it
    /// has to outlive the read.
    pub(super) fn short_of<T>(value: &T, declared: usize) -> (Vec<u64>, *const T) {
        assert!(declared <= size_of::<T>());
        // `u64` for its alignment, which is at least what any of these
        // descriptors needs: every field is a pointer, a usize or a u32.
        let words = size_of::<T>().div_ceil(size_of::<u64>());
        let mut buf = vec![0u64; words];
        let base = buf.as_mut_ptr().cast::<u8>();
        // SAFETY: `words * 8` bytes are owned by `buf`.
        unsafe {
            std::ptr::write_bytes(base, TAIL, words * size_of::<u64>());
            std::ptr::copy_nonoverlapping((value as *const T).cast::<u8>(), base, declared);
        }
        let ptr = buf.as_ptr().cast::<T>();
        (buf, ptr)
    }

    pub(super) fn a_host(size: usize) -> HostInfo<'static> {
        HostInfo {
            struct_size: size as u32,
            abi_version: crate::library::ABI_VERSION,
            host_id: Str::new("test-host"),
            host_version: Str::new("1.0"),
            alloc: std::ptr::null(),
            meta: MaybeNull::null(),
            services: std::ptr::null(),
        }
    }
}
