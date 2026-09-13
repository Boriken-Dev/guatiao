// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The allocator that travels with an owned tree.
//!
//! Every owned container stores a pointer to one of these, so growth and
//! free never take an allocator argument: a container knows how it was
//! allocated. That is `Vec<T, A>` transposed, and it is what lets a library
//! hand a map to a host that then appends to it and frees it without
//! either side naming the other's heap.
//!
//! # Versioning, and why a reference is the wrong tool
//!
//! [`Allocator`] leads with `struct_size` so fields can be appended.
//! Reading a caller's struct therefore cannot start by making a
//! `&Allocator`: a reference asserts that the **whole pointee** is
//! dereferenceable, which is exactly what a caller compiled before the
//! last field was appended does not have. That is undefined behaviour
//! before any check could run, and it is not theoretical — reading 40
//! bytes off a 32-byte object placed against a guard page is an access
//! violation.
//!
//! So [`Alloc::from_raw`] reads `struct_size` through a raw field
//! projection, decides what is covered, and reads each field the same way.
//! It keeps the function pointers rather than the struct pointer, so no
//! later code can re-ask a question that was already answered.
//!
//! # What an allocator implementer must guarantee
//!
//! Stated here because `struct_size` cannot express any of it, and the
//! shipped header repeats it next to the function pointers:
//!
//! - `alloc` returns memory aligned to at least `align`, or null. An
//!   allocator that cannot honour the alignment is *supposed* to return
//!   null; one that returns a misaligned pointer instead is broken, and
//!   this crate treats the two identically.
//! - `free` receives the same `(size, align)` the block was allocated
//!   with. Pairing `_aligned_malloc` with `free` corrupts the heap.
//! - Neither may unwind, throw a C++ exception, or `longjmp`. A foreign
//!   unwind entering Rust across a `"C"` boundary is undefined behaviour
//!   that no code on this side can defend against.
//! - The `Allocator` itself must outlive every tree allocated through
//!   it, since containers keep a pointer to it.

#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::ptr;

/// Allocates `size` bytes aligned to `align`, or returns null.
///
/// `size` is never 0: an empty container is bookkeeping, not an
/// allocation, and a zero-size request to a `malloc`-backed allocator
/// returns a real block that would then leak.
pub type AllocFn = unsafe extern "C" fn(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void;

/// Releases a block previously returned by the matching `alloc`, with the
/// **same** `size` and `align` it was allocated with.
pub type FreeFn =
    unsafe extern "C" fn(ctx: *mut c_void, ptr: *mut c_void, size: usize, align: usize);

/// Tears down the allocator itself, once nothing allocated through it is
/// still live. Optional: most allocators have nothing to release.
pub type ReleaseFn = unsafe extern "C" fn(ctx: *mut c_void);

/// An allocator, as a vtable a foreign caller can fill in.
///
/// Every field after `struct_size` is read only when `struct_size` says it
/// is there, so fields may be **appended** without breaking a caller
/// compiled against an older copy.
#[repr(C)]
#[derive(Debug)]
pub struct Allocator {
    /// `sizeof(guatiao_alloc)` as the caller compiled it. Always first,
    /// always set. Zero is refused rather than read charitably.
    pub struct_size: u32,
    /// Passed back to every callback below. Opaque to this crate.
    pub ctx: *mut c_void,
    /// Required. A null here is refused at the boundary rather than
    /// called.
    ///
    /// Spelled out rather than written as the type alias above, and that
    /// is a header-generation constraint rather than a style: through an
    /// alias, a header generator renders this field as an opaque struct
    /// used by value, which is an incomplete type and compiles nowhere.
    pub alloc:
        Option<unsafe extern "C" fn(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void>,
    /// Required. A null here is refused at the boundary rather than
    /// called. Spelled out for the reason given on `alloc`.
    pub free:
        Option<unsafe extern "C" fn(ctx: *mut c_void, ptr: *mut c_void, size: usize, align: usize)>,
    /// Optional, and the newest field. A caller compiled before it existed
    /// declares a smaller `struct_size` and it reads as absent — never as
    /// whatever bytes happened to follow that caller's allocation.
    pub release: Option<unsafe extern "C" fn(ctx: *mut c_void)>,
}

impl Allocator {
    /// Everything up to and including `free`: what a caller must cover for
    /// this struct to be usable at all.
    ///
    /// **This floor must not move when a field is appended.** Comparing a
    /// caller's `struct_size` against `size_of::<Allocator>()` instead
    /// would refuse every caller built before the newest field existed,
    /// which inverts the whole convention. Each appended field gets its
    /// own guard instead; see [`Alloc::from_raw`].
    pub const fn floor() -> usize {
        offset_of!(Allocator, free) + size_of::<Option<FreeFn>>()
    }

    /// One past the end of `release`, the newest field.
    pub const fn release_end() -> usize {
        offset_of!(Allocator, release) + size_of::<Option<ReleaseFn>>()
    }
}

/// Why a caller's allocator could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AllocError {
    /// The pointer to the allocator was null.
    Null,
    /// `struct_size` did not even cover `free`, so there is no usable
    /// vtable here — not an old caller, a wrong one.
    TooSmall {
        /// What the caller declared.
        declared: usize,
        /// What it must be at least.
        need: usize,
    },
    /// The `alloc` slot was null. It is not optional.
    NoAllocFn,
    /// The `free` slot was null. It is not optional.
    NoFreeFn,
    /// The allocator returned null, or returned memory that did not meet
    /// the alignment it was asked for. The two are deliberately one case:
    /// an allocator that cannot honour an alignment is supposed to return
    /// null, and one that returns a misaligned pointer is broken in the
    /// same way.
    Failed,
    /// The requested size overflowed what a block may be. See
    /// [`Alloc::alloc`].
    TooLarge,
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::Null => f.write_str("the allocator pointer was null"),
            AllocError::TooSmall { declared, need } => write!(
                f,
                "the allocator declared struct_size {declared}, which does not cover its \
                 required callbacks ({need} bytes)"
            ),
            AllocError::NoAllocFn => f.write_str("the allocator's `alloc` callback was null"),
            AllocError::NoFreeFn => f.write_str("the allocator's `free` callback was null"),
            AllocError::Failed => f.write_str("the allocator returned null or misaligned memory"),
            AllocError::TooLarge => {
                f.write_str("the requested allocation exceeds the maximum block size")
            }
        }
    }
}

impl std::error::Error for AllocError {}

/// A checked view of a caller's allocator.
///
/// Holds the function pointers rather than the struct pointer, so the
/// coverage question is answered once, here. `release` is `None` for a
/// caller that did not declare it, and nothing downstream can tell that
/// apart from a caller that declared it as null — which is correct, since
/// both mean "no teardown".
///
/// # An allocator outlives everything built through it
///
/// **That is a contract, not a borrow.** Every owned container records the
/// address of the allocator that made it and calls back into it to grow
/// and to free, so an allocator that goes away first leaves every tree
/// built through it holding a pointer to nothing — and the symptom is a
/// free through an unmapped function pointer, arriving at teardown, a long
/// way from the mistake.
///
/// **Not a lifetime parameter**, and that is deliberate rather than
/// missing. [`Alloc::from_raw`] is `unsafe` and hands back whatever
/// lifetime the caller asks for, so the borrow checker could only ever
/// police an allocator built on the stack — the case nobody has. Every
/// case anybody does have (a constant, a `static`, a vtable inside a
/// loaded library) it cannot see at all.
///
/// So the rule is stated instead, here and on [`Alloc::from_raw`]: an
/// allocator lives at least as long as everything built through it. A
/// library that hands out trees must therefore never be unloaded while
/// the host still holds one.
#[derive(Clone, Copy)]
pub struct Alloc {
    ctx: *mut c_void,
    alloc: AllocFn,
    free: FreeFn,
    release: Option<ReleaseFn>,
    raw: *const Allocator,
}

impl std::fmt::Debug for Alloc {
    /// Deliberately does not print the function pointers: a raw address is
    /// not something a reader can act on. What is diagnostic is which
    /// optional slot survived the coverage check.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alloc")
            .field("ctx_is_null", &self.ctx.is_null())
            .field("has_release", &self.release.is_some())
            .finish_non_exhaustive()
    }
}

impl Alloc {
    /// Checks a caller's allocator and captures its callbacks.
    ///
    /// # Safety
    ///
    /// Two things, and the second is the one the compiler stopped
    /// checking when the lifetime parameter went:
    ///
    /// 1. `raw` is null, or points at `raw->struct_size` readable, aligned
    ///    bytes. Nothing past `struct_size` is read.
    /// 2. **The allocator outlives every value built through it.** Each
    ///    owned container records this address and calls back into it to
    ///    grow and to free, so an allocator that goes away first leaves
    ///    every such tree pointing at nothing. A vtable inside a loaded
    ///    library therefore means that library is never unloaded while the
    ///    host holds a tree it made — which is why a loader forgets its
    ///    handle rather than closing it.
    pub unsafe fn from_raw(raw: *const Allocator) -> Result<Alloc, AllocError> {
        if raw.is_null() {
            return Err(AllocError::Null);
        }

        // The leading u32 only, through the raw pointer. Forming `&*raw`
        // first would assert that the whole struct is dereferenceable,
        // which is precisely what a caller with a smaller one does not
        // have.
        let declared = unsafe { ptr::addr_of!((*raw).struct_size).read() } as usize;

        if declared < Allocator::floor() {
            return Err(AllocError::TooSmall {
                declared,
                need: Allocator::floor(),
            });
        }

        // Every read below is a field projection through the raw pointer,
        // for the same reason. A `declared` LARGER than this build's
        // struct needs no clamp: only fields this build has are ever read.
        let ctx = unsafe { ptr::addr_of!((*raw).ctx).read() };
        let Some(alloc) = (unsafe { ptr::addr_of!((*raw).alloc).read() }) else {
            return Err(AllocError::NoAllocFn);
        };
        let Some(free) = (unsafe { ptr::addr_of!((*raw).free).read() }) else {
            return Err(AllocError::NoFreeFn);
        };

        // The appended-field guard. `>=` against offset plus size means a
        // declared size covering only PART of the slot reads as absent
        // rather than handing back half a real pointer and half of
        // whatever followed the caller's allocation — which would be
        // non-null, and therefore called.
        let release = if declared >= Allocator::release_end() {
            unsafe { ptr::addr_of!((*raw).release).read() }
        } else {
            None
        };

        Ok(Alloc {
            ctx,
            alloc,
            free,
            release,
            raw,
        })
    }

    /// Rust's global allocator, ready to use and needing no `unsafe`.
    ///
    /// The vtable it borrows is a **constant**: written at compile time,
    /// never mutated, carrying a null context. That is what makes handing
    /// out a `'static` borrow of it sound, and it is why this is the one
    /// `static` in the crate.
    ///
    /// # This does not reintroduce process-global state
    ///
    /// The rule it might look like it breaks is about state two linkages
    /// could **disagree** about -- an interning table, a registry, a
    /// counter -- where a value produced by one artifact is invisible to
    /// another. Nothing here can disagree: it is immutable, it has no
    /// interior mutability, and the compiler initialises it rather than
    /// any code at run time.
    ///
    /// A host and a library that each link their own copy of this crate get
    /// their own constant, naming their own Rust allocator, which is
    /// exactly right: **every owned container records the allocator that
    /// made it**, so a tree built in the library is freed through the
    /// library's allocator even after it crosses into the host.
    pub fn rust() -> Alloc {
        /// A `Allocator` holds a `*mut c_void` context, so it is not
        /// `Sync` and cannot be a `static` without saying why.
        struct Vtable(Allocator);

        // SAFETY: the value is a compile-time constant that is never
        // written, its `ctx` is null so nothing is shared through it, and
        // the two functions it names are Rust's global allocator, which is
        // itself thread-safe. A shared reference to it hands out nothing
        // that can race.
        unsafe impl Sync for Vtable {}

        static RUST: Vtable = Vtable(rust_alloc());

        // SAFETY: `RUST.0` is fully initialised, lives for `'static`, and
        // declares its own size, so every field this reads is present.
        unsafe { Alloc::from_raw(&RUST.0) }
            .expect("the constant vtable this crate writes is complete")
    }

    /// The allocator's own address, which is what an owned container
    /// stores.
    pub fn as_raw(&self) -> *const Allocator {
        self.raw
    }

    /// Whether this allocator declared a teardown callback.
    pub fn has_release(&self) -> bool {
        self.release.is_some()
    }

    /// Allocates `size` bytes aligned to `align`.
    ///
    /// Rejects a misaligned return the same way it rejects null, so no
    /// reference or slice is ever built from memory that could not satisfy
    /// its own type. `size` must be non-zero and `align` a power of two;
    /// both are guaranteed by the callers in `raw`, which derive
    /// them from an element type rather than from caller input.
    pub fn alloc(&self, size: usize, align: usize) -> Result<*mut u8, AllocError> {
        debug_assert!(size != 0, "an empty container must not reach the allocator");
        debug_assert!(align.is_power_of_two());

        // A block whose size exceeds `isize::MAX` cannot be described by a
        // slice or offset through safely, so it is refused before the
        // allocator is asked.
        if size > isize::MAX as usize {
            return Err(AllocError::TooLarge);
        }

        // SAFETY: `alloc` came from a checked vtable, and the contract on
        // the type says it may be called with any non-zero size and any
        // power-of-two alignment.
        let p = unsafe { (self.alloc)(self.ctx, size, align) };
        if p.is_null() || (p as usize) & (align - 1) != 0 {
            return Err(AllocError::Failed);
        }
        Ok(p.cast::<u8>())
    }

    /// Releases a block, with the size and alignment it was allocated
    /// with.
    ///
    /// # Safety
    ///
    /// `ptr` came from [`Alloc::alloc`] on **this** allocator, with
    /// exactly this `size` and `align`, and is not used again.
    pub unsafe fn free(&self, ptr: *mut u8, size: usize, align: usize) {
        debug_assert!(!ptr.is_null());
        debug_assert!(size != 0);
        // SAFETY: the caller guarantees the block and its layout.
        unsafe { (self.free)(self.ctx, ptr.cast::<c_void>(), size, align) }
    }

    /// Runs the allocator's teardown callback, if it declared one.
    ///
    /// # Safety
    ///
    /// Nothing allocated through this allocator is still live.
    pub unsafe fn release(&self) {
        if let Some(release) = self.release {
            // SAFETY: the caller guarantees nothing is outstanding.
            unsafe { release(self.ctx) }
        }
    }
}

// --- the allocator this crate supplies --------------------------------

unsafe extern "C" fn rust_alloc_fn(_ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(size, align) else {
        return ptr::null_mut();
    };
    if layout.size() == 0 {
        // `std::alloc::alloc` with a zero-size layout is undefined
        // behaviour. Nothing in this crate asks for one, and a foreign
        // caller reaching this vtable directly gets a refusal rather than
        // a demonstration.
        return ptr::null_mut();
    }
    // SAFETY: the layout has non-zero size, checked immediately above.
    unsafe { std::alloc::alloc(layout).cast::<c_void>() }
}

unsafe extern "C" fn rust_free_fn(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    if p.is_null() || size == 0 {
        return;
    }
    let Ok(layout) = std::alloc::Layout::from_size_align(size, align) else {
        return;
    };
    // SAFETY: the contract on `FreeFn` is that `p` came from the
    // matching `alloc` with this exact size and alignment.
    unsafe { std::alloc::dealloc(p.cast::<u8>(), layout) }
}

/// An allocator over Rust's global allocator.
///
/// The value must outlive every tree built through it, because containers
/// keep a pointer to it. A Rust caller wanting exactly this and nothing
/// more should use [`Alloc::rust`], which hands back a ready one; this is
/// the raw vtable, for a caller filling in a struct to hand across a
/// boundary.
pub const fn rust_alloc() -> Allocator {
    Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: ptr::null_mut(),
        alloc: Some(rust_alloc_fn),
        free: Some(rust_free_fn),
        release: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The floor must sit below the current size, or appending a field
    /// locks out every caller compiled before it.
    #[test]
    fn the_floor_sits_below_the_current_struct() {
        assert!(
            Allocator::floor() < size_of::<Allocator>(),
            "the floor must be BELOW the current size, or appending `release` refuses every \
             caller built before it existed"
        );
        assert!(Allocator::floor() <= offset_of!(Allocator, release));
        assert_eq!(Allocator::release_end(), size_of::<Allocator>());
    }

    /// A caller compiled before `release` existed is accepted, and its
    /// `release` reads as absent rather than as whatever followed it.
    ///
    /// The guard word is non-zero on purpose: a tail of zeroes would make
    /// this pass against a guard that does nothing at all, since a null
    /// function pointer already reads as `None`.
    #[test]
    fn an_older_caller_is_accepted_and_its_release_reads_as_absent() {
        const GUARD: u64 = 0xDEAD_BEEF_DEAD_BEEF;

        #[repr(C)]
        struct Older {
            struct_size: u32,
            ctx: *mut c_void,
            alloc: Option<AllocFn>,
            free: Option<FreeFn>,
        }
        #[repr(C)]
        struct WithGuard {
            older: Older,
            guard: [u64; 2],
        }

        let held = WithGuard {
            older: Older {
                struct_size: size_of::<Older>() as u32,
                ctx: ptr::null_mut(),
                alloc: Some(rust_alloc_fn),
                free: Some(rust_free_fn),
            },
            guard: [GUARD; 2],
        };
        assert_eq!(size_of::<Older>(), Allocator::floor());

        let got = unsafe { Alloc::from_raw(ptr::addr_of!(held.older).cast()) }.unwrap();
        assert!(
            !got.has_release(),
            "an uncovered slot must read as absent, never as the bytes that follow"
        );
        assert!(
            held.guard.iter().all(|&w| w == GUARD),
            "nothing was written"
        );
    }

    /// A declared size that covers only part of the newest slot reads it
    /// as absent, rather than as half a real pointer.
    #[test]
    fn a_size_that_cuts_the_newest_slot_in_half_reads_it_as_absent() {
        let mut a = rust_alloc();
        a.struct_size = (offset_of!(Allocator, release) + 4) as u32;
        let got = unsafe { Alloc::from_raw(&a) }.unwrap();
        assert!(!got.has_release());
    }

    #[test]
    fn a_struct_size_too_small_to_carry_free_is_refused() {
        let mut a = rust_alloc();
        a.struct_size = 8;
        assert_eq!(
            unsafe { Alloc::from_raw(&a) }.unwrap_err(),
            AllocError::TooSmall {
                declared: 8,
                need: Allocator::floor()
            }
        );
    }

    /// Zero is refused rather than read as "the whole struct", which is
    /// the charitable reading that would defeat the check entirely.
    #[test]
    fn a_zero_struct_size_is_refused() {
        let mut a = rust_alloc();
        a.struct_size = 0;
        assert!(matches!(
            unsafe { Alloc::from_raw(&a) }.unwrap_err(),
            AllocError::TooSmall { declared: 0, .. }
        ));
    }

    #[test]
    fn a_null_allocator_and_null_callbacks_are_each_named() {
        assert_eq!(
            unsafe { Alloc::from_raw(ptr::null()) }.unwrap_err(),
            AllocError::Null
        );

        let mut a = rust_alloc();
        a.alloc = None;
        assert_eq!(
            unsafe { Alloc::from_raw(&a) }.unwrap_err(),
            AllocError::NoAllocFn
        );

        let mut a = rust_alloc();
        a.free = None;
        assert_eq!(
            unsafe { Alloc::from_raw(&a) }.unwrap_err(),
            AllocError::NoFreeFn
        );
    }

    /// A caller newer than this build is accepted, and only the fields
    /// this build knows about are read.
    #[test]
    fn a_newer_caller_is_accepted_and_its_unknown_tail_ignored() {
        let mut a = rust_alloc();
        a.struct_size = (size_of::<Allocator>() + 64) as u32;
        let got = unsafe { Alloc::from_raw(&a) }.unwrap();
        assert!(
            !got.has_release(),
            "this allocator declares no teardown, whatever a newer caller's tail might hold"
        );
    }

    #[test]
    fn the_rust_allocator_round_trips_a_block() {
        let a = rust_alloc();
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();
        let p = alloc.alloc(64, 8).unwrap();
        assert!(!p.is_null());
        assert_eq!(p as usize % 8, 0);
        unsafe { alloc.free(p, 64, 8) };
    }

    /// A misaligned return is refused, the same as a null one: nothing
    /// downstream may build a reference out of memory that cannot satisfy
    /// its own type.
    #[test]
    fn a_misaligned_return_is_refused_like_a_null_one() {
        unsafe extern "C" fn skewed(_c: *mut c_void, _s: usize, _a: usize) -> *mut c_void {
            // Address 1: aligned for a byte, misaligned for anything
            // wider, and never dereferenced.
            std::ptr::dangling_mut::<c_void>()
        }
        unsafe extern "C" fn nothing(_c: *mut c_void, _p: *mut c_void, _s: usize, _a: usize) {}
        unsafe extern "C" fn null(_c: *mut c_void, _s: usize, _a: usize) -> *mut c_void {
            ptr::null_mut()
        }

        let mut a = rust_alloc();
        a.alloc = Some(skewed);
        a.free = Some(nothing);
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();
        assert_eq!(alloc.alloc(64, 8), Err(AllocError::Failed));

        a.alloc = Some(null);
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();
        assert_eq!(alloc.alloc(64, 8), Err(AllocError::Failed));
    }
}
