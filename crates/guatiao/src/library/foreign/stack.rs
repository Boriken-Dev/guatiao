// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stack `ld.so` starts on, laid out as the kernel lays out a new
//! program's: from the stack pointer up, `argc`, the argument pointers and
//! a null, the environment pointers and a null, the auxiliary vector ending
//! in `AT_NULL`, then the strings and `AT_RANDOM`'s 16 bytes at the top.

use alloc::vec::Vec;

use super::error::Error;
use super::sys;

/// Memory for the bootstrap thread's stack: `size` bytes above one
/// inaccessible guard page. Never given back: the thread runs on it for
/// the life of the process.
pub(crate) struct Stack {
    low: usize,
    high: usize,
}

impl Stack {
    pub(crate) fn new(size: usize, page: usize) -> Result<Stack, Error> {
        let size = size.next_multiple_of(page);
        let total = size.checked_add(page).ok_or_else(|| Error::Setting {
            name: "bootstrap_stack",
            reason: "larger than the address space".into(),
        })?;
        let at = sys::map_anonymous(total, sys::PROT_NONE).map_err(Error::Memory)?;
        // SAFETY: all but the lowest page of the mapping just made.
        unsafe {
            sys::protect(at.add(page), size, sys::PROT_READ | sys::PROT_WRITE)
                .map_err(Error::Memory)?
        };
        Ok(Stack {
            low: at as usize + page,
            high: at as usize + total,
        })
    }

    /// Writes the start-up block and answers the stack pointer to enter
    /// `ld.so` with, `align`-aligned. Each string carries its NUL;
    /// `auxv` holds every entry but `AT_EXECFN`, `AT_RANDOM` and `AT_NULL`,
    /// which this adds.
    pub(crate) fn build(
        &self,
        align: usize,
        args: &[Vec<u8>],
        env: &[Vec<u8>],
        auxv: &[(u64, u64)],
        random: [u8; 16],
    ) -> Result<usize, Error> {
        let too_small = || Error::Setting {
            name: "bootstrap_stack",
            reason: "too small for the arguments, environment and auxiliary vector".into(),
        };
        // Strings, from the top down; their addresses are known as they go.
        let mut cursor = self.high & !15;
        let mut strings: Vec<(usize, &[u8])> = Vec::new();
        let place = |bytes: &'_ [u8], cursor: &mut usize| -> Result<usize, Error> {
            *cursor = cursor
                .checked_sub(bytes.len())
                .filter(|&c| c > self.low)
                .ok_or_else(too_small)?;
            Ok(*cursor)
        };
        let random_at = place(&random, &mut cursor)?;
        strings.push((random_at, &random));
        let mut arg_ptrs = Vec::with_capacity(args.len());
        for a in args {
            let at = place(a, &mut cursor)?;
            strings.push((at, a));
            arg_ptrs.push(at as u64);
        }
        let mut env_ptrs = Vec::with_capacity(env.len());
        for e in env {
            let at = place(e, &mut cursor)?;
            strings.push((at, e));
            env_ptrs.push(at as u64);
        }

        let mut words: Vec<u64> = Vec::new();
        words.push(args.len() as u64);
        words.extend(&arg_ptrs);
        words.push(0);
        words.extend(&env_ptrs);
        words.push(0);
        for &(kind, value) in auxv {
            words.extend([kind, value]);
        }
        words.extend([sys::AT_EXECFN, arg_ptrs.first().copied().unwrap_or(0)]);
        words.extend([sys::AT_RANDOM, random_at as u64]);
        words.extend([sys::AT_NULL, 0]);

        let sp = (cursor & !7)
            .checked_sub(words.len() * 8)
            .map(|sp| sp & !(align - 1))
            .filter(|&sp| sp > self.low)
            .ok_or_else(too_small)?;
        // SAFETY: `[sp, high)` lies in the stack's writable pages (each
        // address was checked against `low`), and nothing else uses them.
        unsafe {
            for (at, bytes) in strings {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
            }
            core::ptr::copy_nonoverlapping(words.as_ptr(), sp as *mut u64, words.len());
        }
        Ok(sp)
    }
}
