// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a reader checks about a descriptor's pointers and lengths before
//! it reads through them: the faults a trusted producer can still make
//! (a wild or stale pointer, a corrupt length, a code pointer into data)
//! become a refusal with a sentence. What these cannot see — that a
//! table is one for the kind it names, that a slot expects its `ctx` — is
//! the producer's contract.

#![forbid(unsafe_code)]

use core::ops::Range;

/// The most providers one library descriptor may list.
pub(crate) const MAX_PROVIDERS: usize = 4096;
/// The most kind names one provider may list.
pub(crate) const MAX_KINDS: usize = 256;
/// The most per-kind tables one provider may carry.
pub(crate) const MAX_TABLES: usize = 256;
/// The widest stride a provider or table array may declare, in bytes.
pub(crate) const MAX_STRIDE: usize = 4096;
/// The longest text a descriptor may name (an id, a version, a kind), in
/// bytes.
pub(crate) const MAX_TEXT: usize = 64 * 1024;
/// The largest function table a descriptor may declare, in bytes.
pub(crate) const MAX_TABLE: usize = 64 * 1024;

/// Below this no pointer addresses anything: the low 64 KiB, which Windows
/// never maps and Linux's default `mmap_min_addr` keeps unmapped, so a
/// small offset from null faults rather than reading something.
const LOW_UNMAPPED: usize = 0x1_0000;

/// Why a descriptor was refused: which part of it, and what was wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Refusal {
    /// The part, as a reader would name it: "the provider array".
    pub(crate) what: &'static str,
    /// What was wrong with it, as the rest of the sentence.
    pub(crate) why: &'static str,
}

impl Refusal {
    pub(crate) const fn new(what: &'static str, why: &'static str) -> Refusal {
        Refusal { what, why }
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} {}", self.what, self.why)
    }
}

const MISALIGNED: &str = "is not aligned for what it points at";
const IMPLAUSIBLE: &str =
    "is not an address: it lies in the low 64 KiB or outside the address space";
const PAST_IMAGE: &str = "runs past the end of the library's image";
const NOT_CODE: &str = "lies outside the library's executable sections";
const TOO_LONG: &str = "is longer than a reader accepts";
const NULL_WITH_LENGTH: &str = "is null and claims a length";
const WIDE_STRIDE: &str =
    "declares a stride wider than a reader accepts, or one that breaks its alignment";

/// Whether `addr` could address anything a process maps: past the low
/// 64 KiB, and canonical where the architecture says what that is. On
/// AArch64 the top byte is a tag the hardware ignores, so it is not read.
fn plausible(addr: usize) -> bool {
    addr >= LOW_UNMAPPED && canonical(addr)
}

#[cfg(target_arch = "x86_64")]
fn canonical(addr: usize) -> bool {
    // Bits 56..=63 copy bit 56 or below: canonical under 57-bit paging,
    // which admits every 48-bit address too.
    let top = (addr as i64) >> 56;
    top == 0 || top == -1
}

#[cfg(target_arch = "aarch64")]
fn canonical(addr: usize) -> bool {
    // Bits 52..=55 are all clear or all set, under 52-bit addressing.
    let high = (addr as u64 & 0x00ff_ffff_ffff_ffff) >> 52;
    high == 0 || high == 0xf
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn canonical(_addr: usize) -> bool {
    true
}

/// One mapped image, at the addresses the loader put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Image {
    span: Range<usize>,
    code: Vec<Range<usize>>,
}

impl Image {
    /// The image a file's layout describes, placed where its entry symbol
    /// landed: `entry_at` is the symbol's address in this process, `entry`
    /// and every range the file's own. `None` when a range does not fit
    /// the address space or the entry lands outside every code range, so
    /// a layout that does not describe this mapping is never used.
    pub(crate) fn place(
        entry: u64,
        span: (u64, u64),
        code: &[(u64, u64)],
        entry_at: usize,
    ) -> Option<Image> {
        let bias = (entry_at as u64).wrapping_sub(entry);
        // A range keeps its length; only its start moves.
        let place = |(start, end): (u64, u64)| -> Option<Range<usize>> {
            let len = usize::try_from(end.checked_sub(start)?).ok()?;
            let at = usize::try_from(start.wrapping_add(bias)).ok()?;
            Some(at..at.checked_add(len)?)
        };
        let image = Image {
            span: place(span)?,
            code: code.iter().map(|&r| place(r)).collect::<Option<_>>()?,
        };
        let inside = image.code.iter().any(|r| r.contains(&entry_at))
            && image
                .code
                .iter()
                .all(|r| r.start >= image.span.start && r.end <= image.span.end);
        inside.then_some(image)
    }

    /// Whether `addr` lies in one of the executable ranges.
    #[cfg(test)]
    pub(crate) fn contains_code(&self, addr: usize) -> bool {
        self.code.iter().any(|r| r.contains(&addr))
    }

    /// The executable ranges.
    #[cfg(test)]
    pub(crate) fn code_ranges(&self) -> &[Range<usize>] {
        &self.code
    }

    /// The whole span.
    #[cfg(test)]
    pub(crate) fn span_range(&self) -> Range<usize> {
        self.span.clone()
    }
}

/// Where a descriptor's pointers may point. Every descriptor gets the
/// checks that need no image; one a mapped library answered also gets
/// its image's bounds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds<'a> {
    image: Option<&'a Image>,
}

impl<'a> Bounds<'a> {
    /// Bounds with no image: alignment, the low 64 KiB, the address
    /// space, and the caps.
    pub(crate) const ANY: Bounds<'static> = Bounds { image: None };

    /// Bounds for a descriptor `image` answered, or [`Bounds::ANY`].
    pub(crate) fn of(image: Option<&'a Image>) -> Bounds<'a> {
        Bounds { image }
    }

    /// `bytes` at `addr`, aligned to `align`: plausible, and inside the
    /// image throughout when it starts there. A pointer outside the image
    /// is accepted, since a library may build its descriptor on its heap.
    pub(crate) fn data(&self, addr: usize, bytes: usize, align: usize) -> Result<(), &'static str> {
        if !addr.is_multiple_of(align) {
            return Err(MISALIGNED);
        }
        if !plausible(addr) {
            return Err(IMPLAUSIBLE);
        }
        let end = addr.checked_add(bytes).ok_or(IMPLAUSIBLE)?;
        if bytes > 0 && !plausible(end - 1) {
            return Err(IMPLAUSIBLE);
        }
        if let Some(image) = self.image {
            let starts_in = image.span.contains(&addr);
            let ends_in = end <= image.span.end;
            let straddles = addr < image.span.start && end > image.span.start;
            if (starts_in && !ends_in) || straddles {
                return Err(PAST_IMAGE);
            }
        }
        Ok(())
    }

    /// A function pointer: plausible, and inside the image's executable
    /// sections for a mapped library.
    pub(crate) fn code(&self, addr: usize) -> Result<(), &'static str> {
        if !plausible(addr) {
            return Err(IMPLAUSIBLE);
        }
        match self.image {
            Some(image) if !image.code.iter().any(|r| r.contains(&addr)) => Err(NOT_CODE),
            _ => Ok(()),
        }
    }

    /// An array of `len` elements `stride` bytes apart at `addr`, each
    /// aligned to `align`, of at most `cap` elements. Empty is accepted
    /// whatever `addr` holds.
    pub(crate) fn array(
        &self,
        addr: usize,
        len: usize,
        stride: usize,
        align: usize,
        cap: usize,
    ) -> Result<(), &'static str> {
        if len == 0 {
            return Ok(());
        }
        if addr == 0 {
            return Err(NULL_WITH_LENGTH);
        }
        if len > cap {
            return Err(TOO_LONG);
        }
        if stride > MAX_STRIDE || !stride.is_multiple_of(align) {
            return Err(WIDE_STRIDE);
        }
        let bytes = len
            .checked_mul(stride)
            .filter(|&b| b <= isize::MAX as usize)
            .ok_or(TOO_LONG)?;
        self.data(addr, bytes, align)
    }

    /// Text of `len` bytes at `addr`, of at most [`MAX_TEXT`].
    pub(crate) fn text(&self, addr: usize, len: usize) -> Result<(), &'static str> {
        if len == 0 {
            return Ok(());
        }
        if addr == 0 {
            return Err(NULL_WITH_LENGTH);
        }
        if len > MAX_TEXT {
            return Err(TOO_LONG);
        }
        self.data(addr, len, 1)
    }
}

#[cfg(test)]
// An image with one code range is the ordinary case, not a mistyped range.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn the_low_64_kib_and_a_filled_word_are_not_addresses() {
        assert!(!plausible(0));
        assert!(!plausible(0x10));
        assert!(!plausible(LOW_UNMAPPED - 1));
        assert!(plausible(LOW_UNMAPPED));
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        assert!(!plausible(0xAAAA_AAAA_AAAA_AAAAu64 as usize));
        // A real address of this process.
        let here = 0u64;
        assert!(plausible(&here as *const u64 as usize));
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn a_tagged_pointer_is_an_address() {
        let here = 0u64;
        let tagged = (&here as *const u64 as usize) | (0xB4usize << 56);
        assert!(plausible(tagged));
    }

    #[test]
    fn data_is_aligned_and_stays_inside_the_image_it_starts_in() {
        let image = Image {
            span: 0x10_0000..0x20_0000,
            code: [0x10_0000..0x18_0000].to_vec(),
        };
        let mapped = Bounds::of(Some(&image));
        assert_eq!(mapped.data(0x10_0001, 8, 8), Err(MISALIGNED));
        assert_eq!(mapped.data(0x1F_FFF8, 8, 8), Ok(()));
        assert_eq!(mapped.data(0x1F_FFF8, 16, 8), Err(PAST_IMAGE));
        assert_eq!(mapped.data(0x0F_FFF8, 16, 8), Err(PAST_IMAGE));
        assert_eq!(mapped.data(0x30_0000, 1 << 20, 8), Ok(()), "a heap pointer");
        assert_eq!(mapped.code(0x17_0000), Ok(()));
        assert_eq!(mapped.code(0x19_0000), Err(NOT_CODE));
        assert_eq!(Bounds::ANY.code(0x19_0000), Ok(()));
    }

    #[test]
    fn an_array_is_capped_and_its_stride_keeps_its_alignment() {
        let any = Bounds::ANY;
        assert_eq!(any.array(0, 0, 0, 8, 4), Ok(()), "empty is anything");
        assert_eq!(any.array(0, 1, 8, 8, 4), Err(NULL_WITH_LENGTH));
        assert_eq!(any.array(0x10_0000, 5, 8, 8, 4), Err(TOO_LONG));
        assert_eq!(any.array(0x10_0000, 1, 12, 8, 4), Err(WIDE_STRIDE));
        assert_eq!(
            any.array(0x10_0000, 1, MAX_STRIDE + 8, 8, 4),
            Err(WIDE_STRIDE)
        );
        assert_eq!(any.array(0x10_0000, 4, 16, 8, 4), Ok(()));
        assert_eq!(any.text(0x10_0001, MAX_TEXT + 1), Err(TOO_LONG));
    }

    #[test]
    fn an_image_is_placed_by_its_entry_and_refused_when_that_lands_outside_code() {
        let code = [(0x1000u64, 0x5000u64)];
        let image = Image::place(0x2000, (0x0, 0x9000), &code, 0x7ff0_2000).expect("placed");
        assert_eq!(image.span, 0x7ff0_0000..0x7ff0_9000);
        assert_eq!(image.code, [0x7ff0_1000..0x7ff0_5000]);
        assert!(Image::place(0x6000, (0x0, 0x9000), &code, 0x7ff0_6000).is_none());
    }
}
