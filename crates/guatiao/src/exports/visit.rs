// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Values as events, from C: a `guatiao_visitor` is a table of one
//! function per event, which a walk of a value or of wire bytes calls;
//! a `guatiao_walker` is a function that hands the caller's own data to
//! a visitor this library provides, which builds a value or writes wire
//! bytes. No tree is built between the two ends.
//!
//! The events are the value model's: absent, null, a boolean, a number
//! as its text, a string, bytes, a list's begin and end, and a map's
//! begin, each key, and end. A container's length is given at its begin,
//! or `GUATIAO_LEN_UNKNOWN`.
//!
//! **A status other than `GUATIAO_OK` from a callback stops the walk**
//! and is what the call returns. A slot left null refuses its event with
//! `GUATIAO_ERR_WRONG_KIND`. A callback must return, never unwind or jump
//! out (a C++ exception, `longjmp`): it is called through the C ABI.
//! The tables are read for the call alone and on the calling thread.

use core::ffi::c_void;
use core::ptr;

use super::{entry, guard, out};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::status::Status;
use crate::value::types::{Buffer, Bytes, Str, Value};
use crate::value::visit::{Flat, Sink, TreeSink, Walk};
use crate::value::wire::{Encoded, Encoder};

/// A container's length when it is not known: `list_begin` and
/// `map_begin` are told it rather than a count.
pub const GUATIAO_LEN_UNKNOWN: u64 = u64::MAX;

/// What receives a walk's events, in C.
///
/// `struct_size` is `sizeof(guatiao_visitor)` as the caller compiled it;
/// a smaller one is refused. `ctx` is handed back to every callback. A
/// callback returns `GUATIAO_OK` to go on, anything else to stop the walk
/// with that status; a slot left null refuses its event with
/// `GUATIAO_ERR_WRONG_KIND`. A text the walk hands over is UTF-8, a
/// number's text is in the JSON number grammar, and each view is valid
/// only for the call.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VisitorTable {
    /// `sizeof(guatiao_visitor)` as the caller compiled it.
    pub struct_size: u32,
    /// Handed back to every callback.
    pub ctx: *mut c_void,
    /// `GUATIAO_ABSENT`: there is no value here.
    pub absent: Option<unsafe extern "C" fn(ctx: *mut c_void) -> Status>,
    /// Null.
    pub null: Option<unsafe extern "C" fn(ctx: *mut c_void) -> Status>,
    /// A boolean.
    pub boolean: Option<unsafe extern "C" fn(ctx: *mut c_void, value: bool) -> Status>,
    /// A number, as its text in the JSON number grammar.
    pub number: Option<unsafe extern "C" fn(ctx: *mut c_void, text: Str<'_>) -> Status>,
    /// A string.
    pub string: Option<unsafe extern "C" fn(ctx: *mut c_void, text: Str<'_>) -> Status>,
    /// A byte string.
    pub bytes: Option<unsafe extern "C" fn(ctx: *mut c_void, bytes: Bytes<'_>) -> Status>,
    /// A list opens; `len` values follow, or `GUATIAO_LEN_UNKNOWN`.
    pub list_begin: Option<unsafe extern "C" fn(ctx: *mut c_void, len: u64) -> Status>,
    /// The innermost open list closes.
    pub list_end: Option<unsafe extern "C" fn(ctx: *mut c_void) -> Status>,
    /// A map opens; `len` entries follow, or `GUATIAO_LEN_UNKNOWN`.
    pub map_begin: Option<unsafe extern "C" fn(ctx: *mut c_void, len: u64) -> Status>,
    /// The next entry's key; its value is the next value.
    pub key: Option<unsafe extern "C" fn(ctx: *mut c_void, key: Str<'_>) -> Status>,
    /// The innermost open map closes.
    pub map_end: Option<unsafe extern "C" fn(ctx: *mut c_void) -> Status>,
}

/// What hands its own data to a visitor, in C.
///
/// `walk` is called once with `ctx` and a visitor this library made; it
/// calls the visitor's functions for one value (a container's begin, its
/// elements, its end), then returns `GUATIAO_OK`, or the status a
/// callback answered to stop. The visitor is valid only during `walk`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalkerTable {
    /// `sizeof(guatiao_walker)` as the caller compiled it.
    pub struct_size: u32,
    /// Handed back to `walk`.
    pub ctx: *mut c_void,
    /// Hands one value's events to `into`.
    pub walk: Option<unsafe extern "C" fn(ctx: *mut c_void, into: *const VisitorTable) -> Status>,
}

// The layout, pinned: a size field, then pointers.
const _: () = {
    const P: usize = size_of::<usize>();
    assert!(core::mem::offset_of!(VisitorTable, ctx) == P);
    assert!(core::mem::offset_of!(VisitorTable, absent) == 2 * P);
    assert!(core::mem::offset_of!(VisitorTable, map_end) == 12 * P);
    assert!(size_of::<VisitorTable>() == 13 * P);
    assert!(core::mem::offset_of!(WalkerTable, walk) == 2 * P);
    assert!(size_of::<WalkerTable>() == 3 * P);
};

/// The table at `ptr`, once its `struct_size` says it is whole.
///
/// # Safety
///
/// `ptr` is non-null and readable for its own `struct_size` bytes.
unsafe fn table<T: Copy>(ptr: *const T) -> Option<T> {
    // SAFETY: the size field leads every table and is read alone first.
    let size = unsafe { ptr.cast::<u32>().read() };
    if (size as usize) < size_of::<T>() {
        return None;
    }
    // SAFETY: the caller's table is at least as large as this build's.
    Some(unsafe { ptr.read() })
}

// --- a walk, into a C visitor --------------------------------------------------

/// Why a walk into a C visitor stopped: the status a callback answered,
/// or what the walk itself refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped(pub Status);

impl From<ValueError> for Stopped {
    fn from(e: ValueError) -> Stopped {
        Stopped(e.into())
    }
}

impl From<Stopped> for Status {
    fn from(stopped: Stopped) -> Status {
        stopped.0
    }
}

fn answered(status: Status) -> Result<(), Stopped> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(Stopped(status))
    }
}

fn missing() -> Stopped {
    Stopped(Status::GUATIAO_ERR_WRONG_KIND)
}

/// A C visitor as a [`Sink`]: each event calls the table's function for
/// it, and a status other than `GUATIAO_OK` is [`Stopped`]. What a crate
/// building a C surface on this one hands a walk or a document to.
#[derive(Debug, Clone, Copy)]
pub struct TableSink(VisitorTable);

impl TableSink {
    /// The table at `ptr`, or `None` when its `struct_size` is smaller
    /// than this build's.
    ///
    /// # Safety
    ///
    /// `ptr` is non-null and readable for its `struct_size` bytes, and its
    /// functions keep the contract on [`VisitorTable`] for as long as the
    /// sink is used.
    pub unsafe fn from_ptr(ptr: *const VisitorTable) -> Option<TableSink> {
        // SAFETY: the caller's contract.
        unsafe { table(ptr) }.map(TableSink)
    }
}

impl Sink for TableSink {
    type Error = Stopped;

    fn absent(&mut self) -> Result<(), Stopped> {
        let f = self.0.absent.ok_or_else(missing)?;
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx) })
    }

    fn null(&mut self) -> Result<(), Stopped> {
        let f = self.0.null.ok_or_else(missing)?;
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx) })
    }

    fn bool(&mut self, value: bool) -> Result<(), Stopped> {
        let f = self.0.boolean.ok_or_else(missing)?;
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx, value) })
    }

    fn number(&mut self, text: &str) -> Result<(), Stopped> {
        let f = self.0.number.ok_or_else(missing)?;
        // SAFETY: the caller's function; the view outlives the call.
        answered(unsafe { f(self.0.ctx, Str::new(text)) })
    }

    fn string(&mut self, text: &str) -> Result<(), Stopped> {
        let f = self.0.string.ok_or_else(missing)?;
        // SAFETY: the caller's function; the view outlives the call.
        answered(unsafe { f(self.0.ctx, Str::new(text)) })
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Stopped> {
        let f = self.0.bytes.ok_or_else(missing)?;
        // SAFETY: the caller's function; the view outlives the call.
        answered(unsafe { f(self.0.ctx, Bytes::new(bytes)) })
    }

    fn list_begin(&mut self, len: Option<usize>) -> Result<(), Stopped> {
        let f = self.0.list_begin.ok_or_else(missing)?;
        let len = len.map_or(GUATIAO_LEN_UNKNOWN, |n| n as u64);
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx, len) })
    }

    fn list_end(&mut self) -> Result<(), Stopped> {
        let f = self.0.list_end.ok_or_else(missing)?;
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx) })
    }

    fn map_begin(&mut self, len: Option<usize>) -> Result<(), Stopped> {
        let f = self.0.map_begin.ok_or_else(missing)?;
        let len = len.map_or(GUATIAO_LEN_UNKNOWN, |n| n as u64);
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx, len) })
    }

    fn key(&mut self, key: &str) -> Result<(), Stopped> {
        let f = self.0.key.ok_or_else(missing)?;
        // SAFETY: the caller's function; the view outlives the call.
        answered(unsafe { f(self.0.ctx, Str::new(key)) })
    }

    fn map_end(&mut self) -> Result<(), Stopped> {
        let f = self.0.map_end.ok_or_else(missing)?;
        // SAFETY: the caller's function, with the caller's `ctx`.
        answered(unsafe { f(self.0.ctx) })
    }
}

/// Walks `walked` into the visitor at `visitor`.
///
/// # Safety
///
/// `visitor` is non-null and readable for its `struct_size` bytes, and
/// its functions keep the contract on [`VisitorTable`].
unsafe fn walk_into<W: Walk + ?Sized>(walked: &W, visitor: *const VisitorTable) -> Status {
    // SAFETY: the caller's contract.
    let Some(mut sink) = (unsafe { TableSink::from_ptr(visitor) }) else {
        return Status::GUATIAO_ERR_BAD_VALUE;
    };
    match walked.walk(Flat(&mut sink)) {
        Ok(()) => Status::GUATIAO_OK,
        Err(Stopped(status)) => status,
    }
}

/// Walks `value` into `visitor`: one call per event, in order, nothing
/// built.
///
/// A callback's status stops the walk and is returned. A value nested
/// deeper than `GUATIAO_MAX_DEPTH`, a node whose payload its own reader
/// refuses, and a visitor table smaller than this build's are
/// `GUATIAO_ERR_BAD_VALUE`; a tag this build does not know is
/// `GUATIAO_ERR_WRONG_KIND`.
///
/// # Safety
///
/// `value` is null or a well-formed value; `visitor` is null or a table
/// readable for its `struct_size` bytes whose functions return rather
/// than unwind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_visit(
    value: *const Value,
    visitor: *const VisitorTable,
) -> Status {
    entry!(value, visitor => {
        // SAFETY: both checked non-null; the rest is the caller's contract.
        unsafe { walk_into(&*value, visitor) }
    })
}

/// Walks the value the wire bytes `bytes` hold into `visitor`, checking
/// them as `guatiao_wire_decode` does, with nothing built.
///
/// Bytes that are not a value are `GUATIAO_ERR_BAD_VALUE`, refused where
/// the fault is: the visitor has seen the events before it. A value
/// nested deeper than `GUATIAO_MAX_DEPTH` is refused.
///
/// # Safety
///
/// `bytes` is readable for the call; `visitor` is null or a table
/// readable for its `struct_size` bytes whose functions return rather
/// than unwind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_visit(
    bytes: Bytes<'_>,
    visitor: *const VisitorTable,
) -> Status {
    entry!(visitor => {
        // SAFETY: checked non-null; the rest is the caller's contract.
        unsafe { walk_into(&Encoded(bytes.into()), visitor) }
    })
}

// --- a C walker, into a sink -----------------------------------------------------

/// What a C walker's events go into: a sink, and the first status it
/// refused an event with, which every later event answers too.
struct Feeding<'a, S: ?Sized> {
    sink: &'a mut S,
    refused: Option<Status>,
}

/// One event from C, handed to the sink behind `ctx`.
fn feed<S: Sink + ?Sized>(
    ctx: *mut c_void,
    event: impl FnOnce(&mut S) -> Result<(), S::Error>,
) -> Status
where
    Status: From<S::Error>,
{
    guard(|| {
        if ctx.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        // SAFETY: `ctx` is the `Feeding` `run` put in the table it handed
        // the walker; it lives for the walk, and the walker calls on the
        // calling thread, one call at a time.
        let feeding = unsafe { &mut *ctx.cast::<Feeding<'_, S>>() };
        if let Some(status) = feeding.refused {
            return status;
        }
        match event(feeding.sink) {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => {
                let status = Status::from(e);
                feeding.refused = Some(status);
                status
            }
        }
    })
}

/// A text from C, checked: the walker's promise is only the memory.
fn checked(text: Str<'_>) -> Result<&str, ValueError> {
    core::str::from_utf8(text.into()).map_err(|_| ValueError::NotUtf8)
}

/// The length C gave, as a sink takes it.
fn length(len: u64) -> Option<usize> {
    if len == GUATIAO_LEN_UNKNOWN {
        return None;
    }
    // A length a target cannot address is not a promise it can keep.
    usize::try_from(len).ok()
}

unsafe extern "C" fn on_absent<S: Sink>(ctx: *mut c_void) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.absent())
}

unsafe extern "C" fn on_null<S: Sink>(ctx: *mut c_void) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.null())
}

unsafe extern "C" fn on_bool<S: Sink>(ctx: *mut c_void, value: bool) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.bool(value))
}

unsafe extern "C" fn on_number<S: Sink>(ctx: *mut c_void, text: Str<'_>) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.number(checked(text)?))
}

unsafe extern "C" fn on_string<S: Sink>(ctx: *mut c_void, text: Str<'_>) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.string(checked(text)?))
}

unsafe extern "C" fn on_bytes<S: Sink>(ctx: *mut c_void, bytes: Bytes<'_>) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.bytes(bytes.into()))
}

unsafe extern "C" fn on_list_begin<S: Sink>(ctx: *mut c_void, len: u64) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.list_begin(length(len)))
}

unsafe extern "C" fn on_list_end<S: Sink>(ctx: *mut c_void) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.list_end())
}

unsafe extern "C" fn on_map_begin<S: Sink>(ctx: *mut c_void, len: u64) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.map_begin(length(len)))
}

unsafe extern "C" fn on_key<S: Sink>(ctx: *mut c_void, key: Str<'_>) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.key(checked(key)?))
}

unsafe extern "C" fn on_map_end<S: Sink>(ctx: *mut c_void) -> Status
where
    Status: From<S::Error>,
{
    feed::<S>(ctx, |s| s.map_end())
}

/// Runs the C walker at `walker` into `sink`: the walker's status, or the
/// first the sink refused an event with, which every later event is
/// answered with too. What a crate building a C surface on this one
/// feeds a sink from C with.
///
/// # Safety
///
/// `walker` is non-null and readable for its `struct_size` bytes, and its
/// `walk` keeps the contract on [`WalkerTable`].
pub unsafe fn run_walker<S: Sink>(walker: *const WalkerTable, sink: &mut S) -> Status
where
    Status: From<S::Error>,
{
    // SAFETY: the caller's contract.
    let Some(walker) = (unsafe { table(walker) }) else {
        return Status::GUATIAO_ERR_BAD_VALUE;
    };
    let Some(walk) = walker.walk else {
        return Status::GUATIAO_ERR_NULL;
    };
    let mut feeding = Feeding {
        sink,
        refused: None,
    };
    let into = VisitorTable {
        struct_size: size_of::<VisitorTable>() as u32,
        ctx: (&raw mut feeding).cast(),
        absent: Some(on_absent::<S>),
        null: Some(on_null::<S>),
        boolean: Some(on_bool::<S>),
        number: Some(on_number::<S>),
        string: Some(on_string::<S>),
        bytes: Some(on_bytes::<S>),
        list_begin: Some(on_list_begin::<S>),
        list_end: Some(on_list_end::<S>),
        map_begin: Some(on_map_begin::<S>),
        key: Some(on_key::<S>),
        map_end: Some(on_map_end::<S>),
    };
    // SAFETY: the caller's function, with its own `ctx` and a table that
    // lives for the call.
    let walked = unsafe { walk(walker.ctx, &into) };
    match feeding.refused {
        Some(refused) => refused,
        None => walked,
    }
}

/// Builds the value a C walker's events describe, in `alloc`, through
/// `out`.
///
/// `out` is written the absent marker on entry. A walker's own status
/// stops the build and is returned, and so does the first refusal of an
/// event, which every later event is answered with too: events out of
/// order (an end that matches nothing, a value where a key is due) and a
/// walk that gave no one complete value are `GUATIAO_ERR_WRONG_KIND`;
/// text that is not UTF-8 and a number outside the grammar
/// `GUATIAO_ERR_BAD_VALUE`; a null or unusable allocator, or one that
/// refuses, `GUATIAO_ERR_ALLOC`. A repeated key replaces the first.
///
/// # Safety
///
/// `walker` is null or a table readable for its `struct_size` bytes
/// whose `walk` returns rather than unwinds; `out` addresses writable
/// storage for one value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_from_walker(
    alloc: *const Allocator,
    walker: *const WalkerTable,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(walker, out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let mut sink = TreeSink::new_in(a);
        // SAFETY: checked non-null; the rest is the caller's contract.
        let walked = unsafe { run_walker(walker, &mut sink) };
        if !walked.is_ok() {
            return walked;
        }
        match sink.into_value() {
            // SAFETY: `out` is writable and holds no value the caller owns.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}

/// Writes the wire bytes of a C walker's events, as a BYTES value built
/// in `alloc`, through `out`: the bytes `guatiao_wire_encode` writes for
/// the value the events describe, with no value built.
///
/// `out` is written the absent marker on entry. A walker's own status
/// stops the encoding and is returned, and so does the first refusal of
/// an event: events out of order, a length given at a begin that the
/// events do not keep, and a walk that gave no one complete value are
/// `GUATIAO_ERR_WRONG_KIND`; text that is not UTF-8, a number outside the
/// grammar and a key repeated in its map `GUATIAO_ERR_BAD_VALUE`; a null
/// or unusable allocator `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// As for `guatiao_value_from_walker`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_encode_walker(
    alloc: *const Allocator,
    walker: *const WalkerTable,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(walker, out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes);
        // SAFETY: checked non-null; the rest is the caller's contract.
        let walked = unsafe { run_walker(walker, &mut encoder) };
        if !walked.is_ok() {
            return walked;
        }
        if !encoder.is_complete() {
            return Status::GUATIAO_ERR_WRONG_KIND;
        }
        match Buffer::new_in(a, &bytes).map(Value::from) {
            // SAFETY: `out` is writable and holds no value the caller owns.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}
