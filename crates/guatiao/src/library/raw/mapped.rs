// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a mapped library lies in this process, read from the platform
//! loader's own record of the mapping rather than from the file: the PE
//! headers at a Windows module's base, the program headers
//! `dl_iterate_phdr` reports for an ELF object, the Mach-O header `dladdr`
//! finds on Apple platforms. Where none of those is read, the file's
//! parse places the image instead.

use core::ffi::c_void;
use core::ops::Range;
use std::path::Path;

use super::check::Image;
use crate::library::scan::Layout;

/// The image holding `entry_at`, placed by the platform's record of the
/// mapping, or by `file`'s parse of the library at `path` when there is
/// none. `None` when neither places an image the entry lies in.
pub(crate) fn place(
    path: &Path,
    entry_at: usize,
    file: impl FnOnce(&Path) -> Option<Layout>,
) -> Option<Image> {
    place_with(path, entry_at, image, file)
}

/// [`place`] with the mapped reader named, so a test can take it away.
fn place_with(
    path: &Path,
    entry_at: usize,
    mapped: impl FnOnce(usize) -> Option<Image>,
    file: impl FnOnce(&Path) -> Option<Layout>,
) -> Option<Image> {
    mapped(entry_at)
        .or_else(|| file(path).and_then(|l| Image::place(l.entry, l.span, &l.code, entry_at)))
}

/// The mapped image holding `entry_at`, as the platform's loader records
/// it: its whole span and its executable ranges. `None` where this build
/// reads no such record, or the record does not hold `entry_at` in code.
pub(crate) fn image(entry_at: usize) -> Option<Image> {
    let (span, code) = os::mapping(entry_at)?;
    let code: Vec<(u64, u64)> = code
        .into_iter()
        .map(|r| (r.start as u64, r.end as u64))
        .collect();
    // Absolute addresses, so the entry's own address places them as they
    // are; `place` checks the entry lies in code and code in the span.
    Image::place(
        entry_at as u64,
        (span.start as u64, span.end as u64),
        &code,
        entry_at,
    )
}

/// The span from the lowest range's start to the highest one's end.
#[cfg(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn span_of(ranges: &[Range<usize>]) -> Option<Range<usize>> {
    let start = ranges.iter().map(|r| r.start).min()?;
    let end = ranges.iter().map(|r| r.end).max()?;
    Some(start..end)
}

/// `len` bytes from `start`, or `None` past the address space or empty.
#[cfg(any(
    windows,
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn range(start: usize, len: usize) -> Option<Range<usize>> {
    (len > 0).then_some(())?;
    Some(start..start.checked_add(len)?)
}

/// Reads a `T` at `at`, unaligned.
///
/// # Safety
///
/// `size_of::<T>()` bytes at `at` are mapped and readable.
#[cfg(any(windows, target_vendor = "apple"))]
unsafe fn read<T: Copy>(at: usize) -> T {
    // SAFETY: the caller's contract.
    unsafe { (at as *const T).read_unaligned() }
}

// --- Windows: the PE headers at the module's base -------------------------

#[cfg(windows)]
mod os {
    use super::*;

    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
    const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
    }

    pub(super) fn mapping(entry_at: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        let mut module: *mut c_void = core::ptr::null_mut();
        // SAFETY: with FROM_ADDRESS the name is read as an address inside a
        // module, never dereferenced as text; UNCHANGED_REFCOUNT takes no
        // reference, so nothing is owed back.
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                entry_at as *const u16,
                &mut module,
            )
        };
        if found == 0 || module.is_null() {
            return None;
        }
        // SAFETY: a module handle is its base, where the loader mapped its
        // headers, readable while it stays mapped, which the caller's open
        // handle keeps.
        unsafe { pe(module as usize) }
    }

    /// The span and executable sections of the PE image mapped at `base`,
    /// every offset read checked against the headers' own size.
    ///
    /// # Safety
    ///
    /// A PE image is mapped at `base`.
    unsafe fn pe(base: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        // The DOS header's first page is mapped for every image; nothing is
        // read past it until `SizeOfHeaders` says how far the headers go.
        const FIRST_PAGE: usize = 0x1000;
        let at = |offset: usize| base.checked_add(offset);
        // SAFETY: inside the first page of a mapped image.
        if unsafe { read::<u16>(base) } != 0x5A4D {
            return None;
        }
        // SAFETY: as above.
        let pe = unsafe { read::<u32>(at(0x3C)?) } as usize;
        // The signature, the file header and the optional header's fields
        // read below lie in the first page.
        if pe.checked_add(4 + 20 + 64)? > FIRST_PAGE {
            return None;
        }
        // SAFETY: inside the first page, checked above.
        let (signature, sections, optional_size, magic, image_size, headers_size) = unsafe {
            (
                read::<u32>(at(pe)?),
                read::<u16>(at(pe + 4 + 2)?) as usize,
                read::<u16>(at(pe + 4 + 16)?) as usize,
                read::<u16>(at(pe + 24)?),
                read::<u32>(at(pe + 24 + 56)?) as usize,
                read::<u32>(at(pe + 24 + 60)?) as usize,
            )
        };
        if signature != 0x0000_4550 || !matches!(magic, 0x10B | 0x20B) {
            return None;
        }
        let table = pe.checked_add(24)?.checked_add(optional_size)?;
        if sections > 96
            || headers_size > image_size
            || table.checked_add(sections.checked_mul(40)?)? > headers_size
        {
            return None;
        }
        let mut code = Vec::new();
        for i in 0..sections {
            let header = table + i * 40;
            // SAFETY: the section table lies inside `SizeOfHeaders`,
            // checked above, all of which the loader maps.
            let (virtual_size, address, raw_size, flags) = unsafe {
                (
                    read::<u32>(at(header + 8)?) as usize,
                    read::<u32>(at(header + 12)?) as usize,
                    read::<u32>(at(header + 16)?) as usize,
                    read::<u32>(at(header + 36)?),
                )
            };
            if flags & IMAGE_SCN_MEM_EXECUTE == 0 {
                continue;
            }
            let len = if virtual_size == 0 {
                raw_size
            } else {
                virtual_size
            };
            if let Some(r) = range(at(address)?, len) {
                code.push(r);
            }
        }
        Some((range(base, image_size)?, code))
    }
}

// --- ELF platforms: the loader's program headers ---------------------------

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
mod os {
    use super::*;
    use core::ffi::{c_char, c_int};

    const PT_LOAD: u32 = 1;
    const PF_X: u32 = 1;

    /// `Elf64_Phdr`, laid out whole though only some fields are read.
    ///
    /// cbindgen:ignore
    #[cfg(target_pointer_width = "64")]
    #[repr(C)]
    #[allow(dead_code)]
    #[derive(Clone, Copy)]
    struct Phdr {
        kind: u32,
        flags: u32,
        offset: u64,
        vaddr: u64,
        paddr: u64,
        filesz: u64,
        memsz: u64,
        align: u64,
    }

    /// `Elf32_Phdr`: the flags come after the sizes.
    ///
    /// cbindgen:ignore
    #[cfg(target_pointer_width = "32")]
    #[repr(C)]
    #[allow(dead_code)]
    #[derive(Clone, Copy)]
    struct Phdr {
        kind: u32,
        offset: u32,
        vaddr: u32,
        paddr: u32,
        filesz: u32,
        memsz: u32,
        flags: u32,
        align: u32,
    }

    /// The leading fields of `struct dl_phdr_info`, which every version
    /// of it starts with.
    ///
    /// cbindgen:ignore
    #[repr(C)]
    #[allow(dead_code)]
    struct PhdrInfo {
        addr: usize,
        name: *const c_char,
        phdr: *const Phdr,
        phnum: u16,
    }

    unsafe extern "C" {
        fn dl_iterate_phdr(
            callback: unsafe extern "C" fn(*mut PhdrInfo, usize, *mut c_void) -> c_int,
            data: *mut c_void,
        ) -> c_int;
    }

    struct Search {
        entry_at: usize,
        found: Option<(Range<usize>, Vec<Range<usize>>)>,
    }

    /// One loaded object: its loadable segments, and whether one holds the
    /// entry. Answers 1 to stop at the object that does.
    unsafe extern "C" fn visit(info: *mut PhdrInfo, size: usize, data: *mut c_void) -> c_int {
        if info.is_null() || data.is_null() || size < core::mem::size_of::<PhdrInfo>() {
            return 0;
        }
        // SAFETY: the loader hands a record at least `size` bytes long,
        // and `data` is the `Search` `mapping` passed in.
        let (info, search) = unsafe { (&*info, &mut *data.cast::<Search>()) };
        if info.phdr.is_null() {
            return 0;
        }
        // SAFETY: the loader's program headers for this object, `phnum`
        // of them, mapped while it is loaded, which the iteration holds.
        let headers = unsafe { core::slice::from_raw_parts(info.phdr, usize::from(info.phnum)) };
        let mut loads = Vec::new();
        let mut code = Vec::new();
        for h in headers.iter().filter(|h| h.kind == PT_LOAD) {
            let Some(r) = usize::try_from(h.vaddr)
                .ok()
                .and_then(|v| info.addr.checked_add(v))
                .zip(usize::try_from(h.memsz).ok())
                .and_then(|(start, len)| range(start, len))
            else {
                continue;
            };
            if h.flags & PF_X != 0 {
                code.push(r.clone());
            }
            loads.push(r);
        }
        if !loads.iter().any(|r| r.contains(&search.entry_at)) {
            return 0;
        }
        search.found = span_of(&loads).map(|span| (span, code));
        1
    }

    pub(super) fn mapping(entry_at: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        let mut search = Search {
            entry_at,
            found: None,
        };
        // SAFETY: `visit` reads each record as the loader defines it and
        // `search` outlives the iteration, which calls back synchronously.
        unsafe { dl_iterate_phdr(visit, (&mut search as *mut Search).cast()) };
        search.found
    }
}

// --- Apple platforms: the Mach-O header `dladdr` finds ---------------------

#[cfg(target_vendor = "apple")]
mod os {
    use super::*;
    use core::ffi::{c_char, c_int};

    const MH_MAGIC_64: u32 = 0xFEED_FACF;
    const LC_SEGMENT_64: u32 = 0x19;
    const VM_PROT_EXECUTE: u32 = 0x4;
    /// The most load commands' bytes read: far past any real image.
    const MAX_COMMANDS: usize = 1 << 20;

    /// `Dl_info`, which `dladdr` fills; only the base is read.
    ///
    /// cbindgen:ignore
    #[repr(C)]
    #[allow(dead_code)]
    struct DlInfo {
        fname: *const c_char,
        fbase: *mut c_void,
        sname: *const c_char,
        saddr: *mut c_void,
    }

    unsafe extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    }

    pub(super) fn mapping(entry_at: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        let mut info = DlInfo {
            fname: core::ptr::null(),
            fbase: core::ptr::null_mut(),
            sname: core::ptr::null(),
            saddr: core::ptr::null_mut(),
        };
        // SAFETY: `dladdr` reads the address as a key and writes `info`.
        if unsafe { dladdr(entry_at as *const c_void, &mut info) } == 0 || info.fbase.is_null() {
            return None;
        }
        // SAFETY: `dli_fbase` is the image's Mach-O header, mapped while it
        // is loaded, which the caller's open handle keeps.
        unsafe { macho(info.fbase as usize) }
    }

    /// The span and executable segments of the 64-bit Mach-O image whose
    /// header is at `base`.
    ///
    /// # Safety
    ///
    /// A Mach-O image's header and load commands are mapped at `base`.
    unsafe fn macho(base: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        let at = |offset: usize| base.checked_add(offset);
        // SAFETY: the header's fixed 32 bytes.
        let (magic, ncmds, sizeofcmds) = unsafe {
            (
                read::<u32>(base),
                read::<u32>(at(16)?) as usize,
                read::<u32>(at(20)?) as usize,
            )
        };
        if magic != MH_MAGIC_64 || sizeofcmds > MAX_COMMANDS {
            return None;
        }
        // Segments at their linked addresses, then slid by where the
        // segment holding the header (file offset 0) landed.
        let mut segments = Vec::new();
        let mut slide = None;
        let mut offset = 32usize;
        let end = offset.checked_add(sizeofcmds)?;
        for _ in 0..ncmds {
            if offset.checked_add(8)? > end {
                return None;
            }
            // SAFETY: inside the load commands, checked against their size.
            let (cmd, cmdsize) = unsafe {
                (
                    read::<u32>(at(offset)?),
                    read::<u32>(at(offset + 4)?) as usize,
                )
            };
            if cmdsize < 8 || offset.checked_add(cmdsize)? > end {
                return None;
            }
            if cmd == LC_SEGMENT_64 && cmdsize >= 72 {
                // SAFETY: a `segment_command_64`'s fixed fields, inside
                // the command whose size was checked.
                let (vmaddr, vmsize, fileoff, filesize, initprot) = unsafe {
                    (
                        read::<u64>(at(offset + 24)?),
                        read::<u64>(at(offset + 32)?),
                        read::<u64>(at(offset + 40)?),
                        read::<u64>(at(offset + 48)?),
                        read::<u32>(at(offset + 60)?),
                    )
                };
                if fileoff == 0 && filesize > 0 {
                    slide = Some((base as u64).wrapping_sub(vmaddr));
                }
                if vmsize > 0 {
                    segments.push((vmaddr, vmsize, initprot & VM_PROT_EXECUTE != 0));
                }
            }
            offset += cmdsize;
        }
        let slide = slide?;
        let mut loads = Vec::new();
        let mut code = Vec::new();
        for (vmaddr, vmsize, executable) in segments {
            let start = usize::try_from(vmaddr.wrapping_add(slide)).ok()?;
            let Some(r) = range(start, usize::try_from(vmsize).ok()?) else {
                continue;
            };
            if executable {
                code.push(r.clone());
            }
            loads.push(r);
        }
        Some((span_of(&loads)?, code))
    }
}

// --- anywhere else: no record read, so the file's parse places it ----------

#[cfg(not(any(
    windows,
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
mod os {
    use super::*;

    pub(super) fn mapping(_entry_at: usize) -> Option<(Range<usize>, Vec<Range<usize>>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::raw::test_support::hello_library;
    use crate::library::raw::{DECLARES_SYMBOL, ENTRY_SYMBOL, EntryFn};

    /// A real library of this repository, mapped for the process: its path,
    /// its entry symbol's address and its declaration's.
    fn mapped_fixture() -> (std::path::PathBuf, usize, usize) {
        let path = hello_library();
        // SAFETY: a fixture of this repository, mapped and never unmapped.
        let library = unsafe { libloading::Library::new(&path) }.expect("it maps");
        // SAFETY: the entry symbol has this crate's entry type; the data
        // symbol is a byte array, read here only as an address.
        let (entry_at, data_at) = unsafe {
            let entry = library
                .get::<EntryFn>(ENTRY_SYMBOL)
                .expect("an entry symbol");
            let declares = library
                .get::<*const u8>(DECLARES_SYMBOL)
                .expect("a declaration");
            (*entry as usize, *declares as usize)
        };
        core::mem::forget(library);
        (path, entry_at, data_at)
    }

    /// The loader's record places the image with no file read, and agrees
    /// with the file's parse: the entry in code, every executable section
    /// the file names inside an executable range of the mapping, and the
    /// declaration inside the span.
    #[cfg_attr(miri, ignore = "Miri cannot map a library")]
    #[cfg(any(windows, target_vendor = "apple", target_os = "linux"))]
    #[test]
    fn the_loaders_record_places_a_mapped_library_without_reading_its_file() {
        let (path, entry_at, data_at) = mapped_fixture();
        let mut read_file = 0;
        let mapped = place(&path, entry_at, |p| {
            read_file += 1;
            crate::library::scan::layout(p)
        })
        .expect("the platform's record places it");
        assert_eq!(read_file, 0, "no file is read once the library is mapped");

        let layout = crate::library::scan::layout(&path).expect("its file has a layout");
        let parsed =
            Image::place(layout.entry, layout.span, &layout.code, entry_at).expect("placed");
        assert!(mapped.contains_code(entry_at));
        for r in parsed.code_ranges() {
            assert!(
                mapped
                    .code_ranges()
                    .iter()
                    .any(|m| m.start <= r.start && r.end <= m.end),
                "the file's executable section {r:x?} lies in the mapping's code {:x?}",
                mapped.code_ranges()
            );
        }
        assert!(mapped.span_range().contains(&data_at));
        assert!(parsed.span_range().start >= mapped.span_range().start);
        // The image's last bytes are writable data or link metadata on
        // every format: inside the span, outside code.
        let last = mapped.span_range().end - 1;
        assert!(!mapped.contains_code(last), "{last:#x} is not code");
    }

    /// Where the platform's record gives nothing, the file's parse places
    /// the image, as it did before the record was read.
    #[cfg_attr(miri, ignore = "Miri cannot map a library")]
    #[test]
    fn the_files_parse_places_it_where_the_platform_gives_nothing() {
        let (path, entry_at, _) = mapped_fixture();
        let mut read_file = 0;
        let image = place_with(
            &path,
            entry_at,
            |_| None,
            |p| {
                read_file += 1;
                crate::library::scan::layout(p)
            },
        )
        .expect("the file places it");
        assert_eq!(read_file, 1);
        assert!(image.contains_code(entry_at));
    }

    /// An address in no library is placed by neither: the heap.
    #[cfg_attr(miri, ignore = "Miri has no loader to ask")]
    #[test]
    fn an_address_in_no_library_has_no_image() {
        let heap = Box::new(0u64);
        assert!(image(&*heap as *const u64 as usize).is_none());
    }
}
