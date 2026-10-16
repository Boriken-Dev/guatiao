// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a handle holds on its library: the registry's own copies of the
//! library's tables, kept for as long as anything uses them.
//!
//! A [`Lease`] is one per registered library. Every handle that can reach
//! the library's code (`Remote`, `Offer`, `Instance`, `Object`,
//! `ProviderView`) holds a [`Hold`] on it, and the registry counts them:
//! `unload` refuses while one is held, and `collect` closes a retired
//! library once none is. Dropping a lease frees memory and never unmaps.

use alloc::sync::Arc;

use super::*;

/// One run of table bytes: sixteen-aligned, so a copy is aligned for any
/// table a kind can declare.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Chunk([u8; 16]);

/// One library's tables as this side copied them, and the parts of a
/// library the host built.
pub(crate) struct Lease {
    /// Each table, copied byte for byte under the size its descriptor
    /// declared. Only read through the pointers handed out.
    #[allow(dead_code)]
    tables: Vec<Vec<Chunk>>,
    /// A library the host built, owned until the last handle goes.
    #[allow(dead_code)]
    parts: Option<Box<crate::library::kind::LibraryParts>>,
}

impl core::fmt::Debug for Lease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lease")
            .field("tables", &self.tables.len())
            .field("owned", &self.parts.is_some())
            .finish()
    }
}

/// What one handle holds on the library it reaches.
///
/// `None` for a handle with no library behind it: a table from
/// [`Remote::from_raw`](crate::library::Remote::from_raw), a lent object,
/// an object the host built.
#[derive(Debug, Clone, Default)]
pub(crate) enum Hold {
    #[default]
    None,
    // Held for what dropping it lets go of.
    Lease(#[allow(dead_code)] Arc<Lease>),
}

impl Hold {
    /// Whether this holds `lease`.
    #[allow(dead_code)]
    pub(crate) fn holds(&self, lease: &Arc<Lease>) -> bool {
        matches!(self, Hold::Lease(held) if Arc::ptr_eq(held, lease))
    }
}

/// `size` bytes at `table`, copied into storage the lease owns.
///
/// # Safety
///
/// `table` is null, or addresses `size` readable bytes.
unsafe fn copy_table(table: *const c_void, size: usize) -> Option<Vec<Chunk>> {
    if table.is_null() {
        return None;
    }
    // A `Vec`, never a `Box`: moving a `Box` retags it as unique, which
    // invalidates the table pointer taken from it before the move.
    let mut copy = vec![Chunk([0; 16]); size.div_ceil(size_of::<Chunk>())];
    // SAFETY: the caller's contract for the source; the copy holds at
    // least `size` bytes and is its own allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(table.cast::<u8>(), copy.as_mut_ptr().cast::<u8>(), size)
    };
    Some(copy)
}

impl Lease {
    /// A lease over copies of every table `views` name, with each view's
    /// table pointers moved onto the copies and holding the lease.
    ///
    /// Safe because a view is built only by reading a descriptor, whose
    /// every table pointer passed the read's checks for the size it
    /// declares; what those checks cannot see is the producer's contract.
    pub(crate) fn adopt(
        views: &mut [ProviderView],
        parts: Option<Box<crate::library::kind::LibraryParts>>,
    ) -> Arc<Lease> {
        let mut tables = Vec::new();
        let mut keep = |table: &mut *const c_void, size: usize| {
            // SAFETY: a view's table addresses the bytes its descriptor
            // declared, as above.
            if let Some(copy) = unsafe { copy_table(*table, size) } {
                *table = copy.as_ptr().cast::<c_void>();
                tables.push(copy);
            }
        };
        for view in views.iter_mut() {
            keep(&mut view.vtable, view.vtable_size);
            for (_, table, size) in &mut view.tables {
                keep(table, *size);
            }
        }
        let lease = Arc::new(Lease { tables, parts });
        for view in views.iter_mut() {
            view.hold = Hold::Lease(Arc::clone(&lease));
        }
        lease
    }

    /// One view adopted on its own: what a descriptor read with no
    /// registry behind it holds.
    pub(crate) fn alone(mut view: ProviderView) -> ProviderView {
        Lease::adopt(core::slice::from_mut(&mut view), None);
        view
    }
}
