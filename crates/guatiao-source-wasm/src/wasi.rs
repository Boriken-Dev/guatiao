// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The WASI preview-1 functions a reactor built by Go's own toolchain
//! imports, granting nothing: no files (no directory is preopened), no
//! sockets, arguments or environment. A clock, the OS's randomness, fds
//! 0 to 2 as character devices with fd 1 discarded and fd 2 kept (bounded)
//! to name a failure. A module that imports any other WASI function is
//! refused before it runs.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wasmtime::{Caller, Linker, Memory};

use crate::instance::{State, charge, in_time};

/// The module the functions are imported from.
pub(crate) const MODULE: &str = "wasi_snapshot_preview1";

/// Every name this shim answers.
pub(crate) const NAMES: [&str; 15] = [
    "sched_yield",
    "proc_exit",
    "args_get",
    "args_sizes_get",
    "environ_get",
    "environ_sizes_get",
    "clock_time_get",
    "fd_write",
    "random_get",
    "poll_oneoff",
    "fd_close",
    "fd_fdstat_get",
    "fd_fdstat_set_flags",
    "fd_prestat_get",
    "fd_prestat_dir_name",
];

const SUCCESS: i32 = 0;
const BADF: i32 = 8;
const FAULT: i32 = 21;
const INVAL: i32 = 28;
const IO: i32 = 29;

/// How much of fd 2 a guest's instance keeps.
const KEPT: usize = 4096;
/// The most subscriptions one `poll_oneoff` answers.
const MOST_SUBSCRIPTIONS: u32 = 1024;
/// The most buffers one `fd_write` takes.
const MOST_IOVS: u32 = 1024;
/// The most bytes one `random_get` fills: the host does it between the
/// instructions a time limit interrupts.
const MOST_RANDOM: u32 = 1 << 20;
/// Bytes of host work per unit of fuel charged.
const BYTES_PER_FUEL: u64 = 8;

/// What WASI keeps for one instance.
#[derive(Debug)]
pub(crate) struct WasiState {
    started: Instant,
    /// The start of what the guest wrote to fd 2.
    pub(crate) stderr: Vec<u8>,
    /// The code it passed `proc_exit`.
    pub(crate) exited: Option<i32>,
}

impl WasiState {
    pub(crate) fn new() -> WasiState {
        WasiState {
            started: Instant::now(),
            stderr: Vec::new(),
            exited: None,
        }
    }
}

fn memory(caller: &mut Caller<'_, State>) -> Option<Memory> {
    caller
        .get_export(crate::abi::export::MEMORY)
        .and_then(|e| e.into_memory())
}

fn at(address: i32) -> usize {
    address as u32 as usize
}

fn write(caller: &mut Caller<'_, State>, address: i32, bytes: &[u8]) -> i32 {
    match memory(caller) {
        Some(m) if m.write(&mut *caller, at(address), bytes).is_ok() => SUCCESS,
        _ => FAULT,
    }
}

fn read(caller: &mut Caller<'_, State>, address: i32, len: usize) -> Option<Vec<u8>> {
    let m = memory(caller)?;
    let start = at(address);
    m.data(&*caller)
        .get(start..start.checked_add(len)?)
        .map(<[u8]>::to_vec)
}

fn u32_le(bytes: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(word)
}

fn span(memory: &[u8], address: u32, len: usize) -> Option<&[u8]> {
    let start = address as usize;
    memory.get(start..start.checked_add(len)?)
}

fn fd_write(
    caller: &mut Caller<'_, State>,
    fd: i32,
    iovs: i32,
    count: i32,
    out: i32,
) -> wasmtime::Result<i32> {
    if fd != 1 && fd != 2 {
        return Ok(BADF);
    }
    if count as u32 > MOST_IOVS {
        return Ok(INVAL);
    }
    charge(caller, u64::from(count as u32))?;
    let Some(m) = memory(caller) else {
        return Ok(FAULT);
    };
    let room = KEPT.saturating_sub(caller.data().wasi.stderr.len());
    let mut kept = Vec::new();
    let mut total: u32 = 0;
    {
        let data = m.data(&*caller);
        let Some(table) = usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(8))
            .and_then(|len| span(data, iovs as u32, len))
        else {
            return Ok(FAULT);
        };
        for iov in table.as_chunks::<8>().0 {
            let (ptr, len) = (u32_le(iov, 0), u32_le(iov, 4));
            let Some(bytes) = span(data, ptr, len as usize) else {
                return Ok(FAULT);
            };
            if fd == 2 {
                let take = bytes.len().min(room - kept.len());
                kept.extend_from_slice(&bytes[..take]);
            }
            total = total.saturating_add(len);
        }
    }
    caller.data_mut().wasi.stderr.extend_from_slice(&kept);
    charge(caller, u64::from(total) / BYTES_PER_FUEL)?;
    in_time(caller)?;
    Ok(write(caller, out, &total.to_le_bytes()))
}

fn clock_time_get(caller: &mut Caller<'_, State>, id: i32, out: i32) -> i32 {
    let nanos = match id {
        0 => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        1..=3 => caller.data().wasi.started.elapsed().as_nanos(),
        _ => return INVAL,
    };
    write(
        caller,
        out,
        &u64::try_from(nanos).unwrap_or(u64::MAX).to_le_bytes(),
    )
}

fn random_get(caller: &mut Caller<'_, State>, address: i32, len: i32) -> wasmtime::Result<i32> {
    if len as u32 > MOST_RANDOM {
        return Ok(INVAL);
    }
    charge(caller, u64::from(len as u32) / BYTES_PER_FUEL)?;
    let Some(m) = memory(caller) else {
        return Ok(FAULT);
    };
    let start = at(address);
    let Some(bytes) = start
        .checked_add(at(len))
        .and_then(|end| m.data_mut(&mut *caller).get_mut(start..end))
    else {
        return Ok(FAULT);
    };
    let filled = match getrandom::fill(bytes) {
        Ok(()) => SUCCESS,
        Err(_) => IO,
    };
    in_time(caller)?;
    Ok(filled)
}

/// Every subscription answered at once: a clock's as elapsed (nothing
/// sleeps on the host's thread), a file descriptor's with `BADF`.
fn poll_oneoff(
    caller: &mut Caller<'_, State>,
    input: i32,
    output: i32,
    n: i32,
    out: i32,
) -> wasmtime::Result<i32> {
    let n = n as u32;
    if n == 0 || n > MOST_SUBSCRIPTIONS {
        return Ok(INVAL);
    }
    charge(caller, u64::from(n))?;
    let Some(subscriptions) = read(caller, input, n as usize * 48) else {
        return Ok(FAULT);
    };
    let mut events = vec![0u8; n as usize * 32];
    for (sub, event) in subscriptions
        .as_chunks::<48>()
        .0
        .iter()
        .zip(events.as_chunks_mut::<32>().0)
    {
        event[..8].copy_from_slice(&sub[..8]);
        let tag = sub[8];
        let errno: u16 = if tag == 0 { 0 } else { BADF as u16 };
        event[8..10].copy_from_slice(&errno.to_le_bytes());
        event[10] = tag;
    }
    in_time(caller)?;
    let written = write(caller, output, &events);
    if written != SUCCESS {
        return Ok(written);
    }
    Ok(write(caller, out, &n.to_le_bytes()))
}

/// fds 0 to 2: character devices, stdin readable, the others writable.
fn fd_fdstat_get(caller: &mut Caller<'_, State>, fd: i32, out: i32) -> i32 {
    const CHARACTER_DEVICE: u8 = 2;
    const FD_READ: u64 = 1 << 1;
    const FD_WRITE: u64 = 1 << 6;
    let rights = match fd {
        0 => FD_READ,
        1 | 2 => FD_WRITE,
        _ => return BADF,
    };
    let mut stat = [0u8; 24];
    stat[0] = CHARACTER_DEVICE;
    stat[8..16].copy_from_slice(&rights.to_le_bytes());
    write(caller, out, &stat)
}

fn stdio(fd: i32) -> i32 {
    if (0..=2).contains(&fd) { SUCCESS } else { BADF }
}

/// Adds the shim to `linker`.
pub(crate) fn add(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    linker.func_wrap(MODULE, "sched_yield", || SUCCESS)?;
    linker.func_wrap(
        MODULE,
        "proc_exit",
        |mut caller: Caller<'_, State>, code: i32| -> wasmtime::Result<()> {
            caller.data_mut().wasi.exited = Some(code);
            Err(wasmtime::Error::msg(format!("exited with {code}")))
        },
    )?;
    for name in ["args_get", "environ_get"] {
        linker.func_wrap(MODULE, name, |_: i32, _: i32| SUCCESS)?;
    }
    for name in ["args_sizes_get", "environ_sizes_get"] {
        linker.func_wrap(
            MODULE,
            name,
            |mut caller: Caller<'_, State>, count: i32, size: i32| -> i32 {
                match write(&mut caller, count, &0u32.to_le_bytes()) {
                    SUCCESS => write(&mut caller, size, &0u32.to_le_bytes()),
                    fault => fault,
                }
            },
        )?;
    }
    linker.func_wrap(
        MODULE,
        "clock_time_get",
        |mut caller: Caller<'_, State>, id: i32, _precision: i64, out: i32| {
            clock_time_get(&mut caller, id, out)
        },
    )?;
    linker.func_wrap(
        MODULE,
        "fd_write",
        |mut caller: Caller<'_, State>, fd: i32, iovs: i32, n: i32, out: i32| {
            fd_write(&mut caller, fd, iovs, n, out)
        },
    )?;
    linker.func_wrap(
        MODULE,
        "random_get",
        |mut caller: Caller<'_, State>, address: i32, len: i32| {
            random_get(&mut caller, address, len)
        },
    )?;
    linker.func_wrap(MODULE, "fd_close", stdio)?;
    linker.func_wrap(MODULE, "fd_fdstat_set_flags", |fd: i32, _flags: i32| {
        stdio(fd)
    })?;
    // No directory is preopened: the first fd asked about ends the list.
    linker.func_wrap(MODULE, "fd_prestat_get", |_: i32, _: i32| BADF)?;
    linker.func_wrap(MODULE, "fd_prestat_dir_name", |_: i32, _: i32, _: i32| BADF)?;
    linker.func_wrap(
        MODULE,
        "fd_fdstat_get",
        |mut caller: Caller<'_, State>, fd: i32, out: i32| fd_fdstat_get(&mut caller, fd, out),
    )?;
    linker.func_wrap(
        MODULE,
        "poll_oneoff",
        |mut caller: Caller<'_, State>, input: i32, output: i32, n: i32, out: i32| {
            poll_oneoff(&mut caller, input, output, n, out)
        },
    )?;
    Ok(())
}
