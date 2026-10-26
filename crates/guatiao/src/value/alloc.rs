// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The allocator that travels with an owned tree.
//!
//! Every owned container stores a pointer to one, so growth and free take
//! no allocator argument: `Vec<T, A>` transposed. A library hands a map to
//! a host, which appends to it and frees it, without either side naming
//! the other's heap.
//!
//! [`Allocator`] leads with `struct_size` so fields can be appended, which
//! is why reading one never starts with a `&Allocator`: a reference
//! asserts the WHOLE pointee is dereferenceable, and a caller compiled
//! before the last field was appended does not have it. Reading 40 bytes
//! off a 32-byte object against a guard page is an access violation, so
//! [`Alloc::from_raw`] projects raw fields and keeps the function
//! pointers rather than the struct pointer.
//!
//! An implementer must guarantee what `struct_size` cannot express, and
//! the shipped header repeats it beside the function pointers: `alloc`
//! returns memory aligned to at least `align` or null; `free` receives
//! the same `(size, align)` the block was allocated with; neither may
//! unwind, throw or `longjmp`; the `Allocator` outlives every tree
//! allocated through it.

#![allow(non_camel_case_types)]

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

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
/// `struct_size` covers at least every field this build knows; a larger
/// one is a caller built with fields appended after them, read up to this
/// build's.
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
    /// Optional: null for an allocator with nothing to tear down.
    pub release: Option<unsafe extern "C" fn(ctx: *mut c_void)>,
}

impl Allocator {
    /// The smallest `struct_size` this build reads: every field it knows.
    pub const fn floor() -> usize {
        size_of::<Allocator>()
    }
}

/// Why a caller's allocator could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AllocError {
    /// The pointer to the allocator was null.
    Null,
    /// `struct_size` is below this build's [`Allocator`]: not a vtable
    /// this build can read.
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

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AllocError::Null => f.write_str("the allocator pointer was null"),
            AllocError::TooSmall { declared, need } => write!(
                f,
                "the allocator declared struct_size {declared}, below the {need} bytes of \
                 this build's guatiao_alloc"
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

impl core::error::Error for AllocError {}

/// A checked view of a caller's allocator: the function pointers rather
/// than the struct pointer, so the size question is answered once.
/// `release` is `None` for an allocator that declares it null.
///
/// **An allocator outlives everything built through it, by contract and
/// not by borrow**: every owned container records its address, so one
/// that goes away first leaves those trees pointing at nothing. There is
/// no lifetime parameter because the borrow checker could only police an
/// allocator on the stack, the one case nobody has.
#[derive(Clone, Copy)]
pub struct Alloc {
    ctx: *mut c_void,
    alloc: AllocFn,
    free: FreeFn,
    release: Option<ReleaseFn>,
    raw: *const Allocator,
}

impl core::fmt::Debug for Alloc {
    /// Deliberately does not print the function pointers: a raw address is
    /// not something a reader can act on. What is diagnostic is which
    /// optional slot the allocator fills.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
    /// 1. `raw` is null, or points at `raw->struct_size` readable,
    ///    aligned bytes. Nothing past this build's fields is read.
    /// 2. **The allocator outlives every value built through it.** A
    ///    vtable inside a loaded library therefore means that library
    ///    stays mapped while the host holds a tree it made -- which is
    ///    what `Registry::unload`'s contract asks the host to state.
    pub unsafe fn from_raw(raw: *const Allocator) -> Result<Alloc, AllocError> {
        if raw.is_null() {
            return Err(AllocError::Null);
        }

        // The leading u32 only, through the raw pointer. Forming `&*raw`
        // first would assert that the whole struct is dereferenceable,
        // which a caller declaring a smaller one does not have.
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
        let release = unsafe { ptr::addr_of!((*raw).release).read() };

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
    /// The vtable it borrows is a **constant**, which is what makes a
    /// `'static` borrow of it sound and why this is the one `static` in
    /// the crate. It is not process-global state: that rule is about
    /// state two linkages could **disagree** about, and nothing
    /// immutable can.
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

    /// Allocates `size` bytes aligned to `align`, rejecting a misaligned
    /// return as it rejects null. `size` is non-zero and `align` a power
    /// of two, both guaranteed by `raw`'s callers.
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

    /// Releases a block with the size and alignment it was allocated
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
    let Ok(layout) = core::alloc::Layout::from_size_align(size, align) else {
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
    unsafe { alloc::alloc::alloc(layout).cast::<c_void>() }
}

unsafe extern "C" fn rust_free_fn(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    if p.is_null() || size == 0 {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(size, align) else {
        return;
    };
    // SAFETY: the contract on `FreeFn` is that `p` came from the
    // matching `alloc` with this exact size and alignment.
    unsafe { alloc::alloc::dealloc(p.cast::<u8>(), layout) }
}

/// An allocator over Rust's global allocator, as a raw vtable for a
/// caller filling in a struct to hand across a boundary.
/// [`Alloc::rust`] is the ready-made one. It must outlive every tree
/// built through it.
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

    /// A caller one word or one byte short of this build's struct is
    /// refused naming both sizes, before `release` is read: a tail of
    /// guard words there would otherwise be called.
    #[test]
    fn a_caller_short_of_this_builds_struct_is_refused() {
        for declared in [
            Allocator::floor() - size_of::<usize>(),
            Allocator::floor() - 1,
        ] {
            let mut a = rust_alloc();
            a.struct_size = declared as u32;
            let why = unsafe { Alloc::from_raw(&a) }.unwrap_err();
            assert_eq!(
                why,
                AllocError::TooSmall {
                    declared,
                    need: Allocator::floor()
                }
            );
            let words = why.to_string();
            assert!(
                words.contains(&declared.to_string())
                    && words.contains(&Allocator::floor().to_string()),
                "{words}"
            );
        }
    }

    /// `release` is read: an allocator that fills it hands it over.
    #[test]
    fn a_release_slot_is_read() {
        unsafe extern "C" fn tear_down(_c: *mut c_void) {}
        let mut a = rust_alloc();
        a.release = Some(tear_down);
        let got = unsafe { Alloc::from_raw(&a) }.unwrap();
        assert!(got.has_release());
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
            core::ptr::dangling_mut::<c_void>()
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
