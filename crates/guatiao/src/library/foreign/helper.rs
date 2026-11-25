// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The program `ld.so` starts, written in memory: an ELF image holding no
//! code at all, only what makes `ld.so` load the C library for it and fill
//! in the addresses of the functions the loader needs. `ld.so` then jumps
//! to the entry point the loader names in the auxiliary vector, which is
//! this crate's own code ([`super::bare`]).
//!
//! The image is one writable segment: headers, the interpreter's path,
//! string and symbol tables, the symbol versions it needs, one `GLOB_DAT`
//! relocation per function and the slots they fill, and a 16-byte
//! thread-local block holding a random mark, by which the host tells the
//! foreign C library's threads from its own.
//!
//! Every symbol is asked for at the oldest version glibc has on the
//! architecture (GLIBC_2.2.5 on x86_64, where `dladdr1` is GLIBC_2.3.3;
//! GLIBC_2.17 on aarch64), so the image runs on any glibc since.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use super::elf::MACHINE;
use super::worker::Import;

#[cfg(target_arch = "x86_64")]
const R_GLOB_DAT: u64 = 6;
#[cfg(target_arch = "aarch64")]
const R_GLOB_DAT: u64 = 1025;

/// The interpreter a glibc program on this architecture names.
#[cfg(target_arch = "x86_64")]
pub(crate) const DEFAULT_LD_SO: &str = "/lib64/ld-linux-x86-64.so.2";
#[cfg(target_arch = "aarch64")]
pub(crate) const DEFAULT_LD_SO: &str = "/lib/ld-linux-aarch64.so.1";

/// The version each import is asked for.
fn version_of(import: Import) -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "GLIBC_2.17"
    } else if import == Import::Dladdr1 {
        "GLIBC_2.3.3"
    } else {
        "GLIBC_2.2.5"
    }
}

/// The libraries the image names by default: before glibc 2.34 `dlopen`
/// and `pthread_create` live in libdl and libpthread; since, those are
/// empty and everything is in libc.
pub(crate) const DEFAULT_NEEDED: [&str; 3] = ["libdl.so.2", "libpthread.so.0", "libc.so.6"];

/// The library whose versions the image asks for.
const VERSIONS_FROM: &str = "libc.so.6";

/// One version a program needs from a library it links: `GLIBC_2.17` of
/// `libc.so.6`, for example.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionNeed {
    /// The library's soname.
    pub file: String,
    /// The version.
    pub version: String,
}

/// A written image and where its parts lie, as offsets from its start
/// (the image is linked at 0, so these are its addresses too).
#[derive(Debug, Clone)]
pub(crate) struct Written {
    pub(crate) bytes: Vec<u8>,
    /// Each import's slot, in [`Import::ALL`]'s order.
    pub(crate) slots: Vec<u64>,
}

const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_DEBUG: u64 = 21;
const DT_FLAGS: u64 = 30;
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DT_VERSYM: u64 = 0x6fff_fff0;
const DT_VERNEED: u64 = 0x6fff_fffe;
const DT_VERNEEDNUM: u64 = 0x6fff_ffff;
const DF_BIND_NOW: u64 = 8;
const DF_1_NOW: u64 = 1;
const DF_1_PIE: u64 = 0x0800_0000;

/// The System V hash of a version name, as `.gnu.version_r` records it.
fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in name {
        h = (h << 4).wrapping_add(u32::from(c));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

struct Strings(Vec<u8>);

impl Strings {
    fn add(&mut self, s: &str) -> u32 {
        let at = self.0.len() as u32;
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        at
    }
}

fn pad(bytes: &mut Vec<u8>, align: usize) {
    while !bytes.len().is_multiple_of(align) {
        bytes.push(0);
    }
}

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Writes the image for `interpreter`, naming `needed`, with `mark` as its
/// thread-local block.
pub(crate) fn write(interpreter: &[u8], needed: &[String], mark: [u64; 2]) -> Written {
    const EHDR: usize = 64;
    const PHENT: usize = 56;
    const PHNUM: usize = 6;

    let mut strings = Strings(vec![0]);
    let needed_at: Vec<u32> = needed.iter().map(|n| strings.add(n)).collect();
    let from_at = strings.add(VERSIONS_FROM);
    let mut versions: Vec<(&str, u32)> = Vec::new();
    for import in Import::ALL {
        let v = version_of(import);
        if !versions.iter().any(|&(name, _)| name == v) {
            versions.push((v, strings.add(v)));
        }
    }
    let names: Vec<u32> = Import::ALL.iter().map(|i| strings.add(i.name())).collect();

    // Everything after the headers, each part aligned for what it holds;
    // the offsets are filled in as the parts are laid out.
    let mut body: Vec<u8> = Vec::new();
    let base = EHDR + PHNUM * PHENT;
    let at = |body: &Vec<u8>| (base + body.len()) as u64;

    let interp_at = at(&body);
    body.extend_from_slice(interpreter);
    body.push(0);
    let interp_len = body.len() as u64;

    pad(&mut body, 8);
    let strtab_at = at(&body);
    body.extend_from_slice(&strings.0);

    pad(&mut body, 8);
    let symtab_at = at(&body);
    body.extend_from_slice(&[0u8; 24]);
    for &name in &names {
        put32(&mut body, name);
        body.push(0x12); // STB_GLOBAL, STT_FUNC
        body.push(0);
        put16(&mut body, 0); // undefined
        put64(&mut body, 0);
        put64(&mut body, 0);
    }

    pad(&mut body, 2);
    let versym_at = at(&body);
    put16(&mut body, 0);
    for import in Import::ALL {
        let v = version_of(import);
        let index = versions
            .iter()
            .position(|&(name, _)| name == v)
            .unwrap_or(0);
        put16(&mut body, 2 + index as u16);
    }

    pad(&mut body, 8);
    let verneed_at = at(&body);
    put16(&mut body, 1);
    put16(&mut body, versions.len() as u16);
    put32(&mut body, from_at);
    put32(&mut body, 16);
    put32(&mut body, 0);
    for (i, &(name, name_at)) in versions.iter().enumerate() {
        put32(&mut body, elf_hash(name.as_bytes()));
        put16(&mut body, 0);
        put16(&mut body, 2 + i as u16);
        put32(&mut body, name_at);
        put32(&mut body, if i + 1 < versions.len() { 16 } else { 0 });
    }

    // The slots come after the relocations that name them; their offsets
    // are known once the dynamic section's size is.
    let mut dynamic: Vec<(u64, u64)> = needed_at
        .iter()
        .map(|&n| (DT_NEEDED, u64::from(n)))
        .collect();
    dynamic.extend([
        (DT_STRTAB, strtab_at),
        (DT_STRSZ, strings.0.len() as u64),
        (DT_SYMTAB, symtab_at),
        (DT_SYMENT, 24),
        (DT_RELA, 0),
        (DT_RELASZ, (Import::ALL.len() * 24) as u64),
        (DT_RELAENT, 24),
        (DT_VERSYM, versym_at),
        (DT_VERNEED, verneed_at),
        (DT_VERNEEDNUM, 1),
        (DT_FLAGS, DF_BIND_NOW),
        (DT_FLAGS_1, DF_1_NOW | DF_1_PIE),
        (DT_DEBUG, 0),
        (DT_NULL, 0),
    ]);
    pad(&mut body, 8);
    let rela_at = at(&body);
    let rela_len = Import::ALL.len() * 24;
    let dynamic_at = rela_at + rela_len as u64;
    let dynamic_len = dynamic.len() * 16;
    let slots_at = dynamic_at + dynamic_len as u64;
    let slots: Vec<u64> = (0..Import::ALL.len() as u64)
        .map(|i| slots_at + 8 * i)
        .collect();
    for (i, slot) in slots.iter().enumerate() {
        put64(&mut body, *slot);
        put64(&mut body, ((i as u64 + 1) << 32) | R_GLOB_DAT);
        put64(&mut body, 0);
    }
    for (tag, value) in dynamic {
        put64(&mut body, tag);
        put64(&mut body, if tag == DT_RELA { rela_at } else { value });
    }
    for _ in &slots {
        put64(&mut body, 0);
    }
    pad(&mut body, 16);
    let tls_at = at(&body);
    put64(&mut body, mark[0]);
    put64(&mut body, mark[1]);
    let total = (base + body.len()) as u64;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"\x7fELF");
    out.extend_from_slice(&[2, 1, 1, 0]);
    out.extend_from_slice(&[0; 8]);
    put16(&mut out, ET_DYN);
    put16(&mut out, MACHINE);
    put32(&mut out, 1);
    put64(&mut out, 0); // entry: the auxiliary vector names it
    put64(&mut out, EHDR as u64);
    put64(&mut out, 0);
    put32(&mut out, 0);
    put16(&mut out, EHDR as u16);
    put16(&mut out, PHENT as u16);
    put16(&mut out, PHNUM as u16);
    put16(&mut out, 64);
    put16(&mut out, 0);
    put16(&mut out, 0);
    let header = |out: &mut Vec<u8>, kind: u32, flags: u32, at: u64, len: u64, align: u64| {
        put32(out, kind);
        put32(out, flags);
        put64(out, at);
        put64(out, at);
        put64(out, at);
        put64(out, len);
        put64(out, len);
        put64(out, align);
    };
    let phdrs = (PHNUM * PHENT) as u64;
    header(&mut out, PT_PHDR, PF_R, EHDR as u64, phdrs, 8);
    header(&mut out, PT_INTERP, PF_R, interp_at, interp_len, 1);
    header(&mut out, PT_LOAD, PF_R | PF_W, 0, total, 4096);
    header(
        &mut out,
        PT_DYNAMIC,
        PF_R | PF_W,
        dynamic_at,
        dynamic_len as u64,
        8,
    );
    header(&mut out, PT_TLS, PF_R, tls_at, 16, 16);
    header(&mut out, PT_GNU_STACK, PF_R | PF_W, 0, 0, 16);
    out.extend_from_slice(&body);
    Written { bytes: out, slots }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::foreign::elf::Elf;

    #[test]
    fn the_image_reads_back_as_a_program_naming_its_interpreter_and_versions() {
        let needed: Vec<String> = DEFAULT_NEEDED.iter().map(|s| (*s).into()).collect();
        let written = write(DEFAULT_LD_SO.as_bytes(), &needed, [1, 2]);
        let elf = Elf::parse(&written.bytes).expect("parses");
        assert_eq!(elf.kind, ET_DYN);
        assert_eq!(elf.interpreter().unwrap(), Some(DEFAULT_LD_SO.as_bytes()));
        let needs = elf.version_needs().expect("reads");
        assert!(needs.iter().all(|n| n.file == "libc.so.6"));
        let versions: Vec<&str> = needs.iter().map(|n| n.version.as_str()).collect();
        if cfg!(target_arch = "x86_64") {
            assert_eq!(versions, ["GLIBC_2.2.5", "GLIBC_2.3.3"]);
        } else {
            assert_eq!(versions, ["GLIBC_2.17"]);
        }
        assert_eq!(written.slots.len(), Import::ALL.len());
        assert!(
            written
                .slots
                .iter()
                .all(|&s| s % 8 == 0 && (s as usize) < written.bytes.len())
        );
    }

    #[test]
    fn the_version_hash_is_the_system_v_one() {
        assert_eq!(elf_hash(b"GLIBC_2.2.5"), 0x09691a75);
        assert_eq!(elf_hash(b"GLIBC_2.17"), 0x06969197);
    }
}
