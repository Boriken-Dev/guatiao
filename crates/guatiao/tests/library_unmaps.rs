// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What can only be measured in a process that maps a library **once**.
//!
//! Both claims here are about a library's first mapping: whether closing
//! it really unmaps, and which allocator its descriptor was built in.
//!
//! **Only Windows is asserted to unmap.** Whether closing frees the
//! address space is the loader's call rather than this crate's: measured
//! in CI, Windows does and macOS does not.
//! Each is decided by state a library initialises on its first entry
//! call, so a second loader anywhere in the same process answers from the
//! first one's run and the measurement says nothing.
//!
//! Hence a test binary of its own, one test per library:
//! `an_unloaded_library_is_really_unmapped` is the only loader of
//! `hello_library` here, and
//! `a_derived_librarys_descriptor_lives_in_the_hosts_arena` the only
//! loader of `derived_greeter`.

#![cfg(feature = "load")]

use std::ffi::c_void;
use std::path::PathBuf;

use guatiao::library::Registry;
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};

/// Where cargo put an example library, found from this binary's own path.
fn library_path(basename: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}{basename}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("{name} is not beside {}", deps.display());
}

/// How many times the mapping at `path` has been entered, read through a
/// handle of this test's own that is closed before it returns.
///
/// A handle held across the unload would keep the mapping alive and make
/// the measurement always say "still mapped".
fn entry_calls(path: &std::path::Path) -> u64 {
    // SAFETY: the file is a library this test just had the loader map,
    // so mapping it again runs no initialiser that has not already run;
    // the symbol is this example's own and takes no arguments.
    unsafe {
        let library = libloading::Library::new(path).expect("it is already mapped");
        let count = library
            .get::<unsafe extern "C" fn() -> u64>(b"hello_library_entry_calls\0")
            .expect("the example exports its entry counter");
        let n = count();
        drop(library);
        n
    }
}

/// Unloading really unmaps: the library's statics start again.
///
/// **Platform truth rather than assumption.** Windows and macOS drop the
/// image, so a fresh load reads 1. glibc may keep a mapping
/// (`RTLD_NODELETE`, unique symbols), and a kept one reads 2. The number
/// each platform gives is printed, and the assertion is the one this
/// platform was measured at.
#[test]
fn an_unloaded_library_is_really_unmapped() {
    let path = library_path("hello_library");
    let mut registry = Registry::new("unmap-tests", "1.0");
    registry
        .load_file(&path)
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");
    assert_eq!(entry_calls(&path), 1, "one entry call, this one");

    // SAFETY: nothing was taken from the library: no tree it allocated,
    // no table, no instance, and the counter handle above is closed.
    unsafe { registry.unload("hello_library") }.expect("nothing is outstanding");

    registry
        .load_file(&path)
        .expect("it loads again")
        .loaded()
        .expect("it accepts this host");
    let after = entry_calls(&path);
    println!("entry calls after an unload and a fresh load: {after}");

    // Whether a close really unmaps is the platform loader's call, and
    // both answers are correct behaviour for `unload`: the library is out
    // of the registry either way. Measured in CI 2026-09-20: Windows 1,
    // the image went; macOS 2, its `dlclose` kept it.
    if cfg!(windows) {
        assert_eq!(
            after, 1,
            "the image went away, so its statics started again"
        );
    } else {
        assert!(
            after == 1 || after == 2,
            "a loader either dropped the image (1) or kept it (2), not {after}"
        );
    }
}

/// A host that offers an allocator gets a derived library's descriptor
/// built in it, so the schema outlives that library's mapping.
///
/// `providers!` builds the descriptor once, through
/// `host.alloc()` when the host has one and its own otherwise.
#[test]
fn a_derived_librarys_descriptor_lives_in_the_hosts_arena() {
    let path = library_path("derived_greeter");
    let mut registry = Registry::with_alloc("arena-tests", "1.0", Some(host_alloc()));
    registry
        .load_file(&path)
        .expect("the derived library loads")
        .loaded()
        .expect("it accepts this host");

    let provider = registry
        .provider("derived_greeter_shouter")
        .expect("the configured provider is loaded");
    // The registry keeps its own copy of the schema; the question here is
    // where the LIBRARY built the one in its descriptor.
    let raw = provider.view().raw;
    // SAFETY: `raw` is the descriptor this provider was read from, in a
    // mapping this registry still holds; `config` is non-null because
    // this provider declares a configuration.
    let alloc = unsafe { (*(*raw).config).alloc() }.expect("a map records its allocator");
    assert_eq!(
        alloc.as_raw(),
        &HOST_ALLOC.0 as *const Allocator,
        "the library built its descriptor in the host's arena"
    );
    assert!(
        COUNTED.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "and it went through this test's own allocator"
    );
}

// --- a host allocator this test can recognise ---------------------------

static COUNTED: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

unsafe extern "C" fn counted_alloc(_ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    COUNTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request to the allocator this one
    // wraps.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counted_free(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: the same block with the layout it was allocated with.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

/// An `Allocator` holds a `*mut c_void`, so it is not `Sync` and cannot
/// be a `static` without saying why.
struct VTable(Allocator);
// SAFETY: a constant that is never written, whose `ctx` is null and whose
// two functions are thread-safe.
unsafe impl Sync for VTable {}

static HOST_ALLOC: VTable = VTable(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: std::ptr::null_mut(),
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

fn host_alloc() -> Alloc {
    // SAFETY: a fully initialised constant that lives for the process and
    // declares its own size.
    unsafe { Alloc::from_raw(&HOST_ALLOC.0) }.expect("the constant vtable is complete")
}
