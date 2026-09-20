// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Growth, shared by all four owned containers.
//!
//! They differ only in element type, so the alloc/copy/free sequence is
//! written once and each reaches it through [`Container`]. Four copies
//! would be four chances to get the `cap == 0` branch wrong, and that
//! branch is the one that corrupts a heap.
//!
//! **`cap > 0` implies `ptr` came from `alloc` and must be freed through
//! it. `cap == 0` implies `ptr` is not ours** -- a literal, a borrow, or
//! an empty container's dangling placeholder -- and is never freed.
//! `cap > 0` requires a non-null `alloc`; `cap == 0` permits either, and
//! a C literal carrying null **adopts** the allocator of the first
//! growing call.
//!
//! [`reserve`] copies out of a `cap == 0` buffer and never touches it
//! again; removing, clearing and replacing write through it. So a literal
//! a consumer intends to mutate must live in writable storage. `cap >=
//! len` is therefore not an input invariant: `len = 5, cap = 0` is the
//! normal shape of a statically declared value, and every computation of
//! spare capacity here handles it before subtracting.

use std::mem::{align_of, size_of};
use std::ptr;

use super::alloc::{Alloc, AllocError, Allocator};
use super::types::{Buffer, Entry, List, Map, Text, Value};

/// One of the four owned containers, seen as its four fields.
///
/// # Safety
///
/// An implementer's `parts` and `set_parts` must address the same storage,
/// and `Elem` must be the element type the `ptr` field really points at:
/// the growth code computes layouts from `Elem` and hands them to `free`.
pub(crate) unsafe trait Container {
    /// What the buffer holds.
    type Elem;

    /// `(ptr, len, cap, alloc)`, as stored.
    fn parts(&self) -> (*mut Self::Elem, usize, usize, *const Allocator);

    /// Writes all four fields at once, so a growth cannot leave two of
    /// them describing the old block and two the new one.
    fn set_parts(&mut self, ptr: *mut Self::Elem, len: usize, cap: usize, alloc: *const Allocator);
}

macro_rules! container {
    ($t:ty, $e:ty) => {
        // SAFETY: the four accessors below read and write the container's
        // own four fields, and `$e` is the element type of its `ptr`.
        unsafe impl Container for $t {
            type Elem = $e;

            fn parts(&self) -> (*mut $e, usize, usize, *const Allocator) {
                (self.ptr, self.len, self.cap, self.alloc)
            }

            fn set_parts(
                &mut self,
                ptr: *mut $e,
                len: usize,
                cap: usize,
                alloc: *const Allocator,
            ) {
                self.ptr = ptr;
                self.len = len;
                self.cap = cap;
                self.alloc = alloc;
            }
        }
    };
}

container!(Text, u8);
container!(Buffer, u8);
container!(List, Value);
container!(Map, Entry);

/// Whether two byte ranges share a byte. Empty ranges touch nothing.
///
/// An append's source may address the buffer it is appending to -- a C
/// caller can hand one a view of the value itself -- and growth frees that
/// buffer. Staging the bytes first makes both cases one case.
pub(crate) fn overlaps(a: *const u8, a_len: usize, b: *const u8, b_len: usize) -> bool {
    if a_len == 0 || b_len == 0 {
        return false;
    }
    let (a, b) = (a as usize, b as usize);
    a < b.saturating_add(b_len) && b < a.saturating_add(a_len)
}

/// The pointer an empty container holds: dangling but **aligned**, never
/// null. `slice::from_raw_parts` requires that even for a zero-length
/// slice, and enforces it with a non-unwinding panic no `catch_unwind`
/// can intercept.
pub(crate) fn dangling<T>() -> *mut T {
    ptr::dangling_mut::<T>()
}

/// The smallest capacity worth allocating, matching std's own floor:
/// one element at a time makes repeated append quadratic in allocator
/// calls.
const fn min_cap<T>() -> usize {
    if size_of::<T>() == 1 { 8 } else { 4 }
}

/// The allocator a container will grow through: its own, or the one
/// passed in when it has none. The single place the two cases meet.
fn allocator_for(stored: *const Allocator, adopt: Option<Alloc>) -> Result<Alloc, AllocError> {
    if stored.is_null() {
        adopt.ok_or(AllocError::Null)
    } else {
        // SAFETY: a non-null `alloc` field is the address of a
        // `Allocator` the producer promised would outlive the tree.
        // `from_raw` reads only what `struct_size` covers.
        unsafe { Alloc::from_raw(stored) }
    }
}

/// The byte size of `cap` elements, refusing anything a slice could not
/// describe.
fn array_size<T>(cap: usize) -> Result<usize, AllocError> {
    let size = cap
        .checked_mul(size_of::<T>())
        .ok_or(AllocError::TooLarge)?;
    if size > isize::MAX as usize {
        return Err(AllocError::TooLarge);
    }
    Ok(size)
}

/// Makes room for at least `extra` more elements past `len`. **On
/// failure nothing changed**: the fields still describe the old block,
/// and it is still live.
///
/// # Safety
///
/// The container's fields are a consistent description of its storage: the
/// first `len` elements are initialised, and `cap > 0` means `ptr` came
/// from `alloc` with a layout of `cap` elements.
pub(crate) unsafe fn reserve<C: Container>(
    c: &mut C,
    extra: usize,
    adopt: Option<Alloc>,
) -> Result<(), AllocError> {
    let (ptr, len, cap, stored) = c.parts();

    // `cap - len` is wrong here and would underflow on a literal, which is
    // the normal shape of a statically declared value rather than an edge
    // case. Compare the requirement against the capacity instead.
    let needed = len.checked_add(extra).ok_or(AllocError::TooLarge)?;
    if cap >= needed && cap > 0 {
        return Ok(());
    }
    if extra == 0 && cap == 0 && len == 0 {
        // Nothing is being asked for and there is nothing to preserve.
        // Leaving this alone keeps "an empty container never allocates"
        // true, which is what keeps a zero-size request away from a
        // `malloc` that would return a real block for it.
        return Ok(());
    }

    // `cap > 0` with a null `alloc` is malformed -- the shape a C brace
    // initialiser produces by leaving a field out -- and the block would
    // be freed through the allocator it ADOPTED, not the one that made
    // it.
    debug_assert!(
        cap == 0 || !stored.is_null(),
        "a container with cap > 0 must carry the allocator that made it"
    );
    let alloc = allocator_for(stored, adopt)?;

    // Exponential, with a floor. Growth must never produce `cap == 0` or
    // the block becomes unfreeable under the never-free rule.
    let doubled = cap.saturating_mul(2);
    let new_cap = needed.max(doubled).max(min_cap::<C::Elem>());
    let new_size = array_size::<C::Elem>(new_cap)?;
    let align = align_of::<C::Elem>();

    let new_ptr = alloc.alloc(new_size, align)?.cast::<C::Elem>();

    // Copy `len`, never `cap`: only the first `len` elements are
    // initialised, and reading past them would be reading uninitialised
    // memory at a type that forbids it.
    if len > 0 {
        // SAFETY: the first `len` elements are initialised and
        // readable, `new_ptr` has room for them, the blocks cannot
        // overlap because one was just allocated, and both are
        // aligned.
        unsafe { ptr::copy_nonoverlapping(ptr, new_ptr, len) };
    }

    // Only now is the old block released, and only if it was ours. A
    // `cap == 0` pointer belongs to somebody else — freeing one is an
    // immediate, unrecoverable heap corruption rather than an error.
    if cap > 0 {
        let old_size = array_size::<C::Elem>(cap)?;
        // SAFETY: `cap > 0` means this block came from this container's
        // allocator with exactly this layout, and nothing reads it again.
        unsafe { alloc.free(ptr.cast::<u8>(), old_size, align) };
    }

    c.set_parts(new_ptr, len, new_cap, alloc.as_raw());
    Ok(())
}

/// Releases a container's buffer and leaves it empty. Does **not** touch
/// the elements: only the caller knows whether they own anything.
///
/// # Safety
///
/// The container's fields describe its storage, and no element is read
/// after this returns.
pub(crate) unsafe fn release_buffer<C: Container>(c: &mut C) {
    let (ptr, _len, cap, stored) = c.parts();
    // Each of the three conditions below can refuse a block this container
    // owns, and refusing means the block is never freed. All three are
    // invariant violations rather than states this crate can produce: a
    // container with `cap > 0` carries a usable allocator and a layout an
    // allocation already succeeded at.
    debug_assert!(
        cap == 0 || !stored.is_null(),
        "a container with cap > 0 must carry the allocator that made it"
    );
    // SAFETY: `from_raw` reads only what `struct_size` covers, and a
    // non-null `alloc` is the address of an allocator the producer
    // promised would outlive the tree.
    debug_assert!(
        cap == 0 || stored.is_null() || unsafe { Alloc::from_raw(stored) }.is_ok(),
        "a container with cap > 0 must carry a usable allocator, or its block is lost"
    );
    debug_assert!(
        cap == 0 || array_size::<C::Elem>(cap).is_ok(),
        "a container's own capacity must describe a layout, or its block is lost"
    );
    if cap > 0
        && !stored.is_null()
        // SAFETY: `from_raw` reads only what `struct_size` covers, and a
        // container with `cap > 0` was allocated through this allocator.
        && let Ok(alloc) = unsafe { Alloc::from_raw(stored) }
        && let Ok(size) = array_size::<C::Elem>(cap)
    {
        // SAFETY: this block came from that allocator with exactly this
        // layout, and nothing reads it again.
        unsafe { alloc.free(ptr.cast::<u8>(), size, align_of::<C::Elem>()) };
    }
    c.set_parts(dangling::<C::Elem>(), 0, 0, stored);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::alloc::rust_alloc;
    use std::cell::Cell;
    use std::ffi::c_void;

    fn empty_buffer(alloc: *const Allocator) -> Buffer {
        Buffer {
            ptr: dangling::<u8>(),
            len: 0,
            cap: 0,
            alloc,
        }
    }

    #[test]
    fn an_empty_container_never_reaches_the_allocator() {
        let a = rust_alloc();
        let mut b = empty_buffer(&a);
        unsafe { reserve(&mut b, 0, None) }.unwrap();
        assert_eq!(b.cap, 0, "asking for nothing must not allocate");
        assert!(!b.ptr.is_null(), "the placeholder is dangling, never null");
    }

    /// A literal is `len > 0, cap == 0`. Growing it must copy out and
    /// leave the original untouched, and must never hand that pointer to
    /// an allocator.
    #[test]
    fn growing_a_literal_copies_out_and_leaves_it_alone() {
        static LITERAL: &[u8; 5] = b"hello";
        let a = rust_alloc();
        let mut b = Buffer {
            ptr: LITERAL.as_ptr() as *mut u8,
            len: 5,
            cap: 0,
            alloc: ptr::null(),
        };

        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();
        unsafe { reserve(&mut b, 3, Some(alloc)) }.unwrap();

        assert!(b.cap >= 8);
        assert_ne!(b.ptr as *const u8, LITERAL.as_ptr(), "a fresh buffer");
        assert_eq!(unsafe { std::slice::from_raw_parts(b.ptr, 5) }, LITERAL);
        assert_eq!(LITERAL, b"hello", "the literal itself is untouched");
        assert!(!b.alloc.is_null(), "the allocator was adopted");

        unsafe { release_buffer(&mut b) };
    }

    #[test]
    fn a_literal_with_no_allocator_and_nothing_to_adopt_is_refused() {
        static LITERAL: &[u8; 5] = b"hello";
        let mut b = Buffer {
            ptr: LITERAL.as_ptr() as *mut u8,
            len: 5,
            cap: 0,
            alloc: ptr::null(),
        };
        assert_eq!(
            unsafe { reserve(&mut b, 1, None) },
            Err(AllocError::Null),
            "growth needs an allocator from somewhere"
        );
        assert_eq!(b.cap, 0, "a refused growth changes nothing");
    }

    #[test]
    fn growth_is_exponential_and_preserves_the_elements() {
        let a = rust_alloc();
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();
        let mut b = empty_buffer(&a);

        for i in 0..100u8 {
            unsafe { reserve(&mut b, 1, Some(alloc)) }.unwrap();
            unsafe { b.ptr.add(b.len).write(i) };
            b.len += 1;
        }
        assert_eq!(b.len, 100);
        assert!(b.cap >= 100);
        let seen = unsafe { std::slice::from_raw_parts(b.ptr, b.len) };
        assert!(seen.iter().copied().eq(0..100));

        unsafe { release_buffer(&mut b) };
        assert_eq!(b.cap, 0);
        assert_eq!(b.len, 0);
    }

    #[test]
    fn a_failing_allocator_leaves_the_container_exactly_as_it_was() {
        unsafe extern "C" fn nope(_c: *mut c_void, _s: usize, _a: usize) -> *mut c_void {
            ptr::null_mut()
        }
        unsafe extern "C" fn noop(_c: *mut c_void, _p: *mut c_void, _s: usize, _a: usize) {}

        let mut a = rust_alloc();
        a.alloc = Some(nope);
        a.free = Some(noop);

        let mut b = empty_buffer(&a);
        let before = (b.ptr, b.len, b.cap);
        assert_eq!(
            unsafe { reserve(&mut b, 16, None) },
            Err(AllocError::Failed)
        );
        assert_eq!((b.ptr, b.len, b.cap), before, "nothing changed on failure");
    }

    #[test]
    fn an_absurd_request_is_refused_rather_than_overflowing() {
        let a = rust_alloc();
        let mut l = List {
            ptr: dangling::<Value>(),
            len: 0,
            cap: 0,
            alloc: &a,
        };
        assert_eq!(
            unsafe { reserve(&mut l, usize::MAX, None) },
            Err(AllocError::TooLarge)
        );
        assert_eq!(l.cap, 0);
    }

    /// Every free must present the layout its allocation used, or the
    /// allocator is being lied to about the block it is reclaiming.
    /// What `free` was told, recorded through the allocator's own `ctx`.
    ///
    /// `ctx` exists for exactly this: state an allocator needs, carried by
    /// the allocator rather than by the process. A `thread_local!` here
    /// would be per-linkage state inside a crate whose premise is that it
    /// has none, and the `no_statics` test fails the build over one.
    #[derive(Default)]
    struct Seen {
        allocs: Cell<usize>,
        frees: Cell<usize>,
        outstanding: Cell<isize>,
        last_free_layout: Cell<(usize, usize)>,
    }

    unsafe extern "C" fn counting_alloc(ctx: *mut c_void, s: usize, al: usize) -> *mut c_void {
        // SAFETY: `ctx` is the `&Seen` the test installed, and it outlives
        // every call made through this vtable.
        let seen = unsafe { &*(ctx as *const Seen) };
        seen.allocs.set(seen.allocs.get() + 1);
        seen.outstanding.set(seen.outstanding.get() + 1);
        let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
        // SAFETY: forwarding an unchanged request to the real allocator.
        unsafe { real(ptr::null_mut(), s, al) }
    }

    unsafe extern "C" fn counting_free(ctx: *mut c_void, p: *mut c_void, s: usize, al: usize) {
        // SAFETY: as above.
        let seen = unsafe { &*(ctx as *const Seen) };
        seen.frees.set(seen.frees.get() + 1);
        seen.outstanding.set(seen.outstanding.get() - 1);
        seen.last_free_layout.set((s, al));
        let real = rust_alloc().free.expect("the rust allocator has a free");
        // SAFETY: forwarding the same block and the same layout to the
        // allocator that actually made it.
        unsafe { real(ptr::null_mut(), p, s, al) }
    }

    fn counting(seen: &Seen) -> Allocator {
        Allocator {
            struct_size: size_of::<Allocator>() as u32,
            ctx: (seen as *const Seen).cast_mut().cast(),
            alloc: Some(counting_alloc),
            free: Some(counting_free),
            release: None,
        }
    }

    #[test]
    fn free_presents_the_layout_the_allocation_used() {
        let seen = Seen::default();
        let a = counting(&seen);
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();

        let mut l = List {
            ptr: dangling::<Value>(),
            len: 0,
            cap: 0,
            alloc: &a,
        };
        unsafe { reserve(&mut l, 1, Some(alloc)) }.unwrap();
        let cap = l.cap;
        unsafe { release_buffer(&mut l) };

        assert_eq!(
            seen.last_free_layout.get(),
            (cap * size_of::<Value>(), align_of::<Value>()),
            "free saw the capacity's layout, not the length's"
        );
        assert_eq!(seen.outstanding.get(), 0, "nothing outstanding");
    }

    /// Growing repeatedly frees each old block exactly once, so a long
    /// append does not leak a block per doubling.
    #[test]
    fn repeated_growth_leaves_nothing_outstanding() {
        let seen = Seen::default();
        let a = counting(&seen);
        let alloc = unsafe { Alloc::from_raw(&a) }.unwrap();

        let mut b = empty_buffer(&a);
        for i in 0..200u8 {
            unsafe { reserve(&mut b, 1, Some(alloc)) }.unwrap();
            unsafe { b.ptr.add(b.len).write(i) };
            b.len += 1;
        }
        assert!(seen.allocs.get() > 1, "it really did grow more than once");
        assert_eq!(
            seen.outstanding.get(),
            1,
            "one live block while the buffer is live"
        );

        unsafe { release_buffer(&mut b) };
        assert_eq!(seen.outstanding.get(), 0);
        assert_eq!(seen.allocs.get(), seen.frees.get());
    }
}
