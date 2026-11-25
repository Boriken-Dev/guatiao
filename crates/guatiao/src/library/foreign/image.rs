// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An ELF file laid out in memory the way the kernel lays out a program and
//! its interpreter: one reservation aligned to the largest segment
//! alignment, each loaded segment copied in, the rest zero, then each page
//! given its segments' protection. Copying rather than mapping the file
//! keeps one path for a file and for bytes built into the crate.

use std::io;

use super::elf::{Elf, PF_R, PF_W, PF_X};
use super::sys;

/// Where an image landed. The memory is never given back: the foreign C
/// library runs from it for the life of the process.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mapped {
    /// What is added to an address in the file's numbering to find it in
    /// memory.
    pub(crate) bias: usize,
}

fn prot(flags: u32) -> core::ffi::c_int {
    let mut prot = sys::PROT_NONE;
    if flags & PF_R != 0 {
        prot |= sys::PROT_READ;
    }
    if flags & PF_W != 0 {
        prot |= sys::PROT_WRITE;
    }
    if flags & PF_X != 0 {
        prot |= sys::PROT_EXEC;
    }
    prot
}

fn too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "an image larger than the address space",
    )
}

/// Lays `elf` out in fresh memory.
pub(crate) fn map(elf: &Elf<'_>, page: usize) -> io::Result<Mapped> {
    let page64 = page as u64;
    let low = elf.loads().map(|p| p.vaddr).min().unwrap_or(0) & !(page64 - 1);
    let high = elf
        .loads()
        .map(|p| p.vaddr + p.memsz)
        .max()
        .unwrap_or(0)
        .checked_next_multiple_of(page64)
        .ok_or_else(too_large)?;
    let span = usize::try_from(high - low).map_err(|_| too_large())?;
    let align = elf
        .loads()
        .map(|p| p.align)
        .filter(|a| a.is_power_of_two())
        .max()
        .unwrap_or(1)
        .max(page64);
    let align = usize::try_from(align).map_err(|_| too_large())?;

    // Reserve span + align, keep the aligned span, give the ends back.
    let reserve = span.checked_add(align).ok_or_else(too_large)?;
    let raw = sys::map_anonymous(reserve, sys::PROT_NONE)?;
    let base = (raw as usize).next_multiple_of(align);
    let head = base - raw as usize;
    let tail = reserve - head - span;
    // SAFETY: both ends are parts of the reservation just made, which
    // nothing uses yet.
    unsafe {
        if head != 0 {
            sys::unmap(raw, head);
        }
        if tail != 0 {
            sys::unmap((base + span) as *mut u8, tail);
        }
    }
    let base_ptr = base as *mut u8;
    let low = usize::try_from(low).map_err(|_| too_large())?;
    let bias = base.wrapping_sub(low);

    // SAFETY: the span is the reservation this function owns.
    unsafe { sys::protect(base_ptr, span, sys::PROT_READ | sys::PROT_WRITE)? };
    for p in elf.loads() {
        let start = usize::try_from(p.offset).map_err(|_| too_large())?;
        let len = usize::try_from(p.filesz).map_err(|_| too_large())?;
        let from = &elf.bytes[start..start + len];
        let at = usize::try_from(p.vaddr).map_err(|_| too_large())? - low;
        // SAFETY: `Elf::parse` checked the segment lies in the file, and
        // `[at, at + memsz)` lies in the span by how `low` and `high` were
        // taken; the memory is fresh, so nothing overlaps.
        unsafe { core::ptr::copy_nonoverlapping(from.as_ptr(), base_ptr.add(at), len) };
    }

    // A page shared by two segments gets both their protections, as the
    // kernel's mapping of the same file would give it.
    let pages = span / page;
    let mut wanted = vec![sys::PROT_NONE; pages];
    for p in elf.loads() {
        let first = (p.vaddr as usize & !(page - 1)) - low;
        let last = (p.vaddr + p.memsz).next_multiple_of(page64) as usize - low;
        for w in &mut wanted[first / page..last / page] {
            *w |= prot(p.flags);
        }
    }
    let mut run = 0;
    while run < pages {
        let mut end = run + 1;
        while end < pages && wanted[end] == wanted[run] {
            end += 1;
        }
        // SAFETY: pages of the span this function owns.
        unsafe { sys::protect(base_ptr.add(run * page), (end - run) * page, wanted[run])? };
        run = end;
    }
    Ok(Mapped { bias })
}
