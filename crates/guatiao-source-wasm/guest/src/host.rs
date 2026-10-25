// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's imports (module `guatiao`), as safe functions. Off wasm32
//! they are stand-ins that keep what they were given, so the glue runs in
//! a native test.

#![allow(unsafe_code)]

#[cfg(target_arch = "wasm32")]
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "guatiao")]
    unsafe extern "C" {
        pub(super) fn answer(ptr: *const u8, len: usize);
        pub(super) fn write_back(index: u32, ptr: *const u8, len: usize) -> u32;
        pub(super) fn object_call(handle: i64, ptr: *const u8, len: usize) -> u32;
        pub(super) fn result(ptr: *mut u8, cap: usize) -> u32;
        pub(super) fn drop(handle: i64);
    }
}

/// Hands the host the current call's answer, which it copies.
#[cfg(target_arch = "wasm32")]
pub(crate) fn answer(bytes: &[u8]) {
    // SAFETY: the host reads `len` bytes at `ptr` of this module's memory,
    // which the slice covers, and keeps no pointer.
    unsafe { imports::answer(bytes.as_ptr(), bytes.len()) }
}

/// Writes argument `index` back over the caller's; a status.
#[cfg(target_arch = "wasm32")]
pub(crate) fn write_back(index: u32, bytes: &[u8]) -> u32 {
    // SAFETY: as for `answer`.
    unsafe { imports::write_back(index, bytes.as_ptr(), bytes.len()) }
}

/// Calls a host object: its status and result (a wire value, nothing, or
/// words).
#[cfg(target_arch = "wasm32")]
pub(crate) fn object_call(handle: i64, request: &[u8]) -> (u32, Vec<u8>) {
    // SAFETY: as for `answer`.
    let status = unsafe { imports::object_call(handle, request.as_ptr(), request.len()) };
    // SAFETY: with a capacity of 0 the host writes nothing and answers the
    // result's length.
    let len = unsafe { imports::result(core::ptr::null_mut(), 0) } as usize;
    let mut result = alloc::vec![0u8; len];
    // SAFETY: the host writes at most `len` bytes at `ptr`, which the
    // vector holds.
    unsafe { imports::result(result.as_mut_ptr(), len) };
    (status, result)
}

/// Lets go of a host object.
#[cfg(target_arch = "wasm32")]
pub(crate) fn drop(handle: i64) {
    // SAFETY: a handle is a number; the host checks it.
    unsafe { imports::drop(handle) }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{answer, drop, object_call, write_back};

/// Off wasm32: what the glue told the host, kept per thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    /// A stand-in for the host's `object_call`.
    pub(crate) type ObjectCall = Box<dyn FnMut(i64, &[u8]) -> (u32, Vec<u8>)>;

    #[derive(Default)]
    pub(crate) struct Told {
        pub(crate) answers: Vec<Vec<u8>>,
        pub(crate) written: Vec<(u32, Vec<u8>)>,
        pub(crate) dropped: Vec<i64>,
        pub(crate) object_call: Option<ObjectCall>,
    }

    std::thread_local! {
        pub(crate) static TOLD: RefCell<Told> = RefCell::new(Told::default());
    }

    pub(crate) fn answer(bytes: &[u8]) {
        TOLD.with(|t| t.borrow_mut().answers.push(bytes.to_vec()));
    }

    pub(crate) fn write_back(index: u32, bytes: &[u8]) -> u32 {
        TOLD.with(|t| t.borrow_mut().written.push((index, bytes.to_vec())));
        0
    }

    pub(crate) fn object_call(handle: i64, request: &[u8]) -> (u32, Vec<u8>) {
        let hook = TOLD.with(|t| t.borrow_mut().object_call.take());
        let Some(mut hook) = hook else {
            return (13, b"no host".to_vec());
        };
        let out = hook(handle, request);
        TOLD.with(|t| t.borrow_mut().object_call = Some(hook));
        out
    }

    pub(crate) fn drop(handle: i64) {
        TOLD.with(|t| t.borrow_mut().dropped.push(handle));
    }
}
