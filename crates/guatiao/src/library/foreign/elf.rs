// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The ELF headers the loader needs, read as data: the program headers it
//! maps, the interpreter a program names, and the C library versions it
//! needs. Every offset is checked against the file before it is read.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use super::error::ImageError;
use super::helper::VersionNeed;

pub(crate) const ET_DYN: u16 = 3;
pub(crate) const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
pub(crate) const PF_X: u32 = 1;
pub(crate) const PF_W: u32 = 2;
pub(crate) const PF_R: u32 = 4;
const PT_DYNAMIC: u32 = 2;
const DT_STRTAB: u64 = 5;
const DT_VERNEED: u64 = 0x6fff_fffe;
const DT_VERNEEDNUM: u64 = 0x6fff_ffff;

#[cfg(target_arch = "x86_64")]
pub(crate) const MACHINE: u16 = 62;
#[cfg(target_arch = "aarch64")]
pub(crate) const MACHINE: u16 = 183;

/// One program header.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Program {
    pub(crate) kind: u32,
    pub(crate) flags: u32,
    pub(crate) offset: u64,
    pub(crate) vaddr: u64,
    pub(crate) filesz: u64,
    pub(crate) memsz: u64,
    pub(crate) align: u64,
}

/// A 64-bit little-endian ELF file for this machine.
#[derive(Debug)]
pub(crate) struct Elf<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) kind: u16,
    pub(crate) entry: u64,
    pub(crate) phoff: u64,
    pub(crate) phentsize: u16,
    pub(crate) phnum: u16,
    pub(crate) programs: Vec<Program>,
}

fn take(bytes: &[u8], at: u64, len: usize) -> Result<&[u8], ImageError> {
    let at = usize::try_from(at).map_err(|_| ImageError::Malformed("an offset past the file"))?;
    at.checked_add(len)
        .and_then(|end| bytes.get(at..end))
        .ok_or(ImageError::Malformed("an offset past the file"))
}

fn u16_at(bytes: &[u8], at: u64) -> Result<u16, ImageError> {
    let b = take(bytes, at, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: u64) -> Result<u32, ImageError> {
    let b = take(bytes, at, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(bytes: &[u8], at: u64) -> Result<u64, ImageError> {
    let b = take(bytes, at, 8)?;
    let mut word = [0u8; 8];
    word.copy_from_slice(b);
    Ok(u64::from_le_bytes(word))
}

/// The NUL-terminated string at `at`.
fn c_str(bytes: &[u8], at: u64) -> Result<&[u8], ImageError> {
    let at = usize::try_from(at).map_err(|_| ImageError::Malformed("a string past the file"))?;
    let rest = bytes
        .get(at..)
        .ok_or(ImageError::Malformed("a string past the file"))?;
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or(ImageError::Malformed("an unterminated string"))?;
    Ok(&rest[..end])
}

impl<'a> Elf<'a> {
    /// Reads the file header and the program headers, and checks what the
    /// loader relies on: each loaded segment lies inside the file and holds
    /// no more than it occupies in memory.
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Elf<'a>, ImageError> {
        if bytes.get(..4) != Some(b"\x7fELF".as_slice()) {
            return Err(ImageError::NotElf);
        }
        // EI_CLASS 2 (64-bit), EI_DATA 1 (little-endian).
        if bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1) {
            return Err(ImageError::Class);
        }
        let kind = u16_at(bytes, 16)?;
        let machine = u16_at(bytes, 18)?;
        if machine != MACHINE {
            return Err(ImageError::Machine {
                found: machine,
                expected: MACHINE,
            });
        }
        let entry = u64_at(bytes, 24)?;
        let phoff = u64_at(bytes, 32)?;
        let phentsize = u16_at(bytes, 54)?;
        let phnum = u16_at(bytes, 56)?;
        if phentsize < 56 {
            return Err(ImageError::Malformed(
                "program headers smaller than ELF64's",
            ));
        }
        let mut programs = Vec::with_capacity(usize::from(phnum));
        for i in 0..u64::from(phnum) {
            let at = phoff
                .checked_add(i * u64::from(phentsize))
                .ok_or(ImageError::Malformed("an offset past the file"))?;
            let program = Program {
                kind: u32_at(bytes, at)?,
                flags: u32_at(bytes, at + 4)?,
                offset: u64_at(bytes, at + 8)?,
                vaddr: u64_at(bytes, at + 16)?,
                filesz: u64_at(bytes, at + 32)?,
                memsz: u64_at(bytes, at + 40)?,
                align: u64_at(bytes, at + 48)?,
            };
            if program.kind == PT_LOAD {
                let len = usize::try_from(program.filesz)
                    .map_err(|_| ImageError::Malformed("a segment past the file"))?;
                take(bytes, program.offset, len)
                    .map_err(|_| ImageError::Malformed("a segment past the file"))?;
                if program.filesz > program.memsz {
                    return Err(ImageError::Malformed(
                        "a segment larger in the file than in memory",
                    ));
                }
                if program.vaddr.checked_add(program.memsz).is_none() {
                    return Err(ImageError::Malformed("a segment past the address space"));
                }
            }
            programs.push(program);
        }
        if !programs.iter().any(|p| p.kind == PT_LOAD) {
            return Err(ImageError::Malformed("no loaded segment"));
        }
        Ok(Elf {
            bytes,
            kind,
            entry,
            phoff,
            phentsize,
            phnum,
            programs,
        })
    }

    /// The loaded segments.
    pub(crate) fn loads(&self) -> impl Iterator<Item = &Program> {
        self.programs.iter().filter(|p| p.kind == PT_LOAD)
    }

    /// The interpreter the file names, without its NUL.
    pub(crate) fn interpreter(&self) -> Result<Option<&'a [u8]>, ImageError> {
        let Some(p) = self.programs.iter().find(|p| p.kind == PT_INTERP) else {
            return Ok(None);
        };
        let len =
            usize::try_from(p.filesz).map_err(|_| ImageError::Malformed("an interpreter path"))?;
        let path = take(self.bytes, p.offset, len)?;
        let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
        Ok(Some(&path[..end]))
    }

    /// Where the program headers are once the file is mapped, as an address
    /// in the file's own numbering: `PT_PHDR`, or the loaded segment that
    /// holds them.
    pub(crate) fn phdr_vaddr(&self) -> Result<u64, ImageError> {
        if let Some(p) = self.programs.iter().find(|p| p.kind == PT_PHDR) {
            return Ok(p.vaddr);
        }
        self.loads()
            .find(|p| p.offset <= self.phoff && self.phoff < p.offset + p.filesz)
            .map(|p| p.vaddr + (self.phoff - p.offset))
            .ok_or(ImageError::Malformed(
                "program headers outside every loaded segment",
            ))
    }

    /// The file offset of `vaddr`, through the loaded segment holding it.
    fn offset_of(&self, vaddr: u64) -> Result<u64, ImageError> {
        self.loads()
            .find(|p| p.vaddr <= vaddr && vaddr < p.vaddr.saturating_add(p.filesz))
            .map(|p| p.offset + (vaddr - p.vaddr))
            .ok_or(ImageError::Malformed(
                "an address outside every loaded segment",
            ))
    }

    /// The versions the file needs from each library it links, read
    /// through its dynamic section (`DT_VERNEED`); empty when it names
    /// none.
    pub(crate) fn version_needs(&self) -> Result<Vec<VersionNeed>, ImageError> {
        let mut needs = Vec::new();
        let Some(dynamic) = self.programs.iter().find(|p| p.kind == PT_DYNAMIC) else {
            return Ok(needs);
        };
        let (mut verneed, mut count, mut strtab) = (None, 0u64, None);
        let mut at = dynamic.offset;
        let end = dynamic.offset.saturating_add(dynamic.filesz);
        while at + 16 <= end {
            let tag = u64_at(self.bytes, at)?;
            let value = u64_at(self.bytes, at + 8)?;
            match tag {
                0 => break,
                DT_STRTAB => strtab = Some(value),
                DT_VERNEED => verneed = Some(value),
                DT_VERNEEDNUM => count = value,
                _ => {}
            }
            at += 16;
        }
        let (Some(verneed), Some(strtab)) = (verneed, strtab) else {
            return Ok(needs);
        };
        let strings = self.offset_of(strtab)?;
        let name = |offset: u32| -> Result<String, ImageError> {
            let at = strings
                .checked_add(u64::from(offset))
                .ok_or(ImageError::Malformed("a string past the file"))?;
            Ok(String::from_utf8_lossy(c_str(self.bytes, at)?).into_owned())
        };
        let mut entry = self.offset_of(verneed)?;
        for _ in 0..count {
            let aux_count = u16_at(self.bytes, entry + 2)?;
            let file = name(u32_at(self.bytes, entry + 4)?)?;
            let mut aux = entry + u64::from(u32_at(self.bytes, entry + 8)?);
            for _ in 0..aux_count {
                needs.push(VersionNeed {
                    file: file.clone(),
                    version: name(u32_at(self.bytes, aux + 8)?)?,
                });
                let next = u32_at(self.bytes, aux + 12)?;
                if next == 0 {
                    break;
                }
                aux += u64::from(next);
            }
            let next = u32_at(self.bytes, entry + 12)?;
            if next == 0 {
                break;
            }
            entry += u64::from(next);
        }
        Ok(needs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_elf_is_refused() {
        assert!(matches!(
            Elf::parse(b"#!/bin/sh\n"),
            Err(ImageError::NotElf)
        ));
        assert!(matches!(Elf::parse(b""), Err(ImageError::NotElf)));
        let mut header = [0u8; 64];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 1;
        header[5] = 1;
        assert!(matches!(Elf::parse(&header), Err(ImageError::Class)));
    }

    #[test]
    fn a_header_pointing_past_the_file_is_refused() {
        let mut header = [0u8; 64];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 2;
        header[5] = 1;
        header[18..20].copy_from_slice(&MACHINE.to_le_bytes());
        header[32..40].copy_from_slice(&4096u64.to_le_bytes());
        header[54..56].copy_from_slice(&56u16.to_le_bytes());
        header[56..58].copy_from_slice(&1u16.to_le_bytes());
        assert!(matches!(Elf::parse(&header), Err(ImageError::Malformed(_))));
    }
}
