// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library written in C, loaded and called as a Rust trait.
//!
//! `tests/c_consumer/greeter_in_c.c` fills the table `greeter_kind.h`
//! renders for `#[guatiao::kind] trait Greeter`, offers it through the
//! envelope, and links nothing. This test compiles it into a shared
//! library, loads it through a registry, and calls it through the proxy
//! the kind attribute generated -- the same one a Rust library's table is
//! called through. A kind that cannot be written in C is not a C ABI;
//! this is the check.

#![cfg(all(feature = "provider", feature = "load"))]

use guatiao::value::convert::TryAsRef;
use std::path::{Path, PathBuf};
use std::process::Command;

use core::ffi::c_void;
use core::ptr::NonNull;

use greeter_kind::{Greeter, GreeterVtable, Listener};
use guatiao::library::{Kind, ProviderError, Registry};
use guatiao::{Alloc, Map, Status, Str};

/// Counts this thread's heap allocations, so a call can be shown to make
/// none: the other tests run on other threads.
struct PerThread;

std::thread_local! {
    static ALLOCATIONS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl core::alloc::GlobalAlloc for PerThread {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: forwarded.
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // SAFETY: forwarded.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static HEAP: PerThread = PerThread;

/// `f`, and how many allocations it made on this thread.
fn allocations<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCATIONS.with(|n| n.get());
    let r = f();
    (r, ALLOCATIONS.with(|n| n.get()) - before)
}

/// The C compilers worth trying, in order (as `c_consumer.rs`).
fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if Command::new(&path).arg("--version").output().is_ok() {
            return Some(path);
        }
    }
    None
}

/// Builds the C greeter into a shared library beside the test artefacts,
/// or says why it could not.
fn build_c_greeter(cc: &Path) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c_consumer/greeter_in_c.c");
    let guatiao_include = manifest.join("include");
    let kind_include = manifest.join("../../examples/greeter_kind/include");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "{}c_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compile = Command::new(cc)
        .args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        // Position-independent code, or a linker whose default is not
        // PIC refuses the shared object outright: `read-only segment has
        // dynamic relocations`. Not passed on Windows, where a DLL needs
        // no such flag and clang warns the argument is unused.
        .args(if cfg!(windows) {
            &[][..]
        } else {
            &["-fPIC"][..]
        })
        .arg("-I")
        .arg(&guatiao_include)
        .arg("-I")
        .arg(&kind_include)
        .arg("-o")
        .arg(&out)
        .arg(&source)
        .output()
        .expect("the compiler answered --version, so it should run");
    assert!(
        compile.status.success(),
        "the C greeter did not compile against the rendered kind header.\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr),
    );
    out
}

#[test]
fn a_greeter_written_in_c_is_called_as_the_trait() {
    let Some(cc) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM install path, \
             cc and gcc). This is the test that proves a kind is a C ABI, so it is \
             worth having on at least one machine."
        );
        return;
    };
    let library = build_c_greeter(&cc);

    let mut registry = Registry::new("c-library-tests", env!("CARGO_PKG_VERSION"));
    let loaded = registry
        .load_file(&library)
        .expect("a shared library the compiler just wrote")
        .loaded()
        .expect("the C entry accepted this host")
        .id
        .clone();
    assert_eq!(loaded, "c_greeter_library");

    // 1. The C table is an offer of the Rust trait: its hash is the one
    // the header carried, its required slot is filled.
    let offer = registry
        .offer::<dyn Greeter>("c_greeter")
        .expect("filed under its id")
        .expect("a table the header described validates");
    assert_eq!(offer.display_name(), "Greeter, in C");
    assert_eq!(offer.version(), "1.0");
    assert_eq!(
        offer.remote().expect("its ctx is its instance").size(),
        core::mem::size_of::<GreeterVtable>()
    );
    let greeter = offer.default_instance().expect("its ctx is its instance");

    // 2. A call crosses into C and back: the answer is a map C built
    // through the allocator the proxy carried in, this side's own, so the
    // host holds the very block C built: nothing was copied.
    // SAFETY: the library the registry mapped above; opening it again only
    // takes another reference to the same image.
    let image = unsafe { libloading::Library::new(&library) }.expect("the same file");
    // SAFETY: the signature `greeter_in_c.c` declares.
    let last_built: libloading::Symbol<unsafe extern "C" fn() -> *const c_void> =
        unsafe { image.get(b"c_greeter_last_built\0") }.expect("exported");
    let (answer, allocated) = allocations(|| greeter.greet("ana"));
    let answer = answer.expect("the C greeter answers");
    assert_eq!(
        allocated, 3,
        "C's three blocks (entries, key, text) and nothing on arrival: no copy, no walk stack"
    );
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello from C, ana")
    );
    let (entries, len, cap, alloc) = answer.into_raw_parts();
    // SAFETY: a getter that takes nothing.
    assert_eq!(entries.cast::<c_void>().cast_const(), unsafe {
        last_built()
    });
    assert_eq!(
        alloc,
        Alloc::rust().as_raw(),
        "built in the host's allocator"
    );
    // SAFETY: the four fields `into_raw_parts` just gave up.
    drop(unsafe { Map::from_raw_parts(entries, len, cap, alloc) });

    // 2b. A caller that carries no allocator -- an older host, a zeroed
    // slot -- gets a map built through C's own, and frees it through the
    // allocator it records.
    {
        let remote = offer.remote().expect("its ctx is its instance");
        // SAFETY: the registry's copy of the table this offer validated.
        let greet = unsafe { (*remote.table().cast::<GreeterVtable>()).greet }.expect("filled");
        // SAFETY: an empty map over no storage.
        let mut out =
            unsafe { Map::from_raw_parts(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        let mut err = ProviderError::none();
        // SAFETY: the slot's signature, a live name, writable out slots.
        let status = unsafe { greet(remote.ctx(), Str::new("bo"), &mut out, &mut err) };
        assert_eq!(status, Status::GUATIAO_OK);
        assert_ne!(
            out.alloc().expect("recorded").as_raw(),
            Alloc::rust().as_raw(),
            "built in C's own allocator"
        );
        assert_eq!(
            out.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
            Some("hello from C, bo")
        );
    }

    // 3. A ProviderError crosses out of C with its message.
    let e = greeter.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet, says C");

    // 4. The slots C left null are appended ones: the proxy runs the
    // trait's default bodies -- `shout` through C's own `greet`, `start`
    // refusing as a greeter that holds no conversations.
    assert_eq!(greeter.shout("bo"), "HELLO FROM C, BO");
    let e = greeter
        .start(NoListener.into_object())
        .expect_err("the default body refuses");
    assert_eq!(e.status, Status::GUATIAO_ERR_NULL);

    // 5. The hash C wrote is the literal the attribute wrote, on both
    // sides of the boundary.
    assert_eq!(<dyn Greeter as Kind>::FLOOR_HASH, GreeterVtable::FLOOR_HASH);

    // 6. A listener LENT to C for one call: C calls its `heard` slot
    // through the `const guatiao_object *` it was given, which is this
    // stack's, and keeps nothing. Nothing is allocated to lend it.
    let mut ear = Ear::default();
    let (answer, allocated) = allocations(|| greeter.greet_into("cy", &mut ear));
    answer.expect("the C greeter answers");
    assert_eq!(allocated, 0, "lending to C allocates nothing");
    assert_eq!(ear.heard, 1);
    assert_eq!(&ear.last[..ear.len], b"hello from C, cy");

    // 7. And C lending ITS object to a Rust greeter: the table is the one
    // `of::<Plain>()` built, the listener C's own, on C's stack. The Rust
    // shim calls it through C's table and never calls its `destroy`.
    {
        // SAFETY: the signature `greeter_in_c.c` declares.
        let lend: libloading::Symbol<
            unsafe extern "C" fn(*const GreeterVtable, *mut c_void, Str<'_>, *mut Tape) -> Status,
        > = unsafe { image.get(b"c_greeter_lend_tape\0") }.expect("exported");
        let mut tape = Tape::default();
        let (status, allocated) = allocations(|| {
            // SAFETY: a Rust table over a ZST `ctx`, and a tape C writes.
            unsafe {
                lend(
                    &PLAIN,
                    NonNull::<Plain>::dangling().as_ptr().cast(),
                    Str::new("dee"),
                    &mut tape,
                )
            }
        });
        assert_eq!(status, Status::GUATIAO_OK);
        assert_eq!(allocated, 0, "lending from C allocates nothing here");
        assert_eq!(
            (tape.heard, tape.destroyed),
            (1, 0),
            "heard once, never destroyed"
        );
        assert_eq!(&tape.last[..tape.len], b"dee");
    }
    drop(image);

    // 8. A C library with no `unload` slot promises nothing, so the host
    // is refused until it insists.
    drop(offer);
    // SAFETY: nothing taken from the library is held any more.
    let why = registry
        .unload("c_greeter_library")
        .expect_err("it has no slot");
    assert!(
        matches!(why, guatiao::library::UnloadError::NotSupported { .. }),
        "{why:?}"
    );
    assert_eq!(registry.loaded().len(), 1, "a refusal changes nothing");
    // SAFETY: nothing taken from the library is held, and it handed out
    // nothing that is alive.
    unsafe { registry.unload_unchecked("c_greeter_library") }.expect("the host insisted");
    assert!(registry.loaded().is_empty());
}

/// A listener nobody will hear from: what `start`'s default body drops.
struct NoListener;

impl Listener for NoListener {
    fn heard(&mut self, _what: &str) {}
}

/// A listener that keeps what it heard in place, allocating nothing.
#[derive(Default)]
struct Ear {
    heard: usize,
    last: [u8; 32],
    len: usize,
}

impl Listener for Ear {
    fn heard(&mut self, what: &str) {
        self.heard += 1;
        self.len = what.len().min(self.last.len());
        self.last[..self.len].copy_from_slice(&what.as_bytes()[..self.len]);
    }
}

/// What `c_greeter_lend_tape` writes, laid out as `greeter_in_c.c`'s
/// `tape`.
#[repr(C)]
#[derive(Default)]
struct Tape {
    heard: i64,
    destroyed: i64,
    len: usize,
    last: [u8; 32],
}

/// A Rust greeter that tells its listener the name it was given.
struct Plain;

impl Greeter for Plain {
    fn greet(&self, _: &str) -> Result<Map, ProviderError> {
        Ok(Map::new())
    }
    fn greet_into(&self, name: &str, listener: &mut dyn Listener) -> Result<(), ProviderError> {
        listener.heard(name);
        Ok(())
    }
}

static PLAIN: GreeterVtable = GreeterVtable::of::<Plain>();
