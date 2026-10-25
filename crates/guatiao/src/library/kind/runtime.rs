// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Kind tables built at run time from a [`KindDescription`], and any table
//! called from one. Behind the `dynamic-kinds` feature.
//!
//! A kind compiled into a program has its table, shims and proxy written
//! by `#[guatiao::kind]`. A kind an engine learns while it runs -- a Lua or
//! JavaScript provider, a remote peer -- has only its description, so its
//! slots are built here: each a libffi closure whose call interface is
//! read from the slot's description, all of a table's closures calling one
//! [`Serve`] handler with the slot and its arguments as [`Arg`]s, and
//! writing the [`Answer`] back as the kind's C table promises (a tree in
//! the caller's carrier, an error through `err`). [`Caller`] goes the
//! other way: it calls any table of a described kind, so an engine can
//! call an object it was handed, or a provider a registry offers.
//!
//! The description is the producer's contract, as a header is: a table is
//! checked against its floor hash and size, and its slots are then called
//! with the shapes the description gives.

use alloc::sync::Arc;
use core::ffi::c_void;
use core::mem::ManuallyDrop;

use libffi::middle::{Cif, Type};
use libffi::raw as ffi;

use super::description::{AnswerType, ArgType, KindDescription, Pass, Scalar, Tree};
use super::*;
use crate::library::raw::{Hold, ProviderView};
use crate::value::convert::FromValue;
use crate::value::types::Bytes;

/// One argument a slot was called with, or one a [`Caller`] passes.
///
/// Scalars arrive widened: every signed integer as [`Arg::Int`], every
/// unsigned one as [`Arg::Uint`], `f32` as [`Arg::Float`]; the slot's
/// description keeps the width, and a [`Caller`] narrows to it, refusing
/// a number that does not fit.
#[derive(Debug)]
#[non_exhaustive]
pub enum Arg<'a> {
    /// `bool`.
    Bool(bool),
    /// `i8` to `i64`, `isize`.
    Int(i64),
    /// `u8` to `u64`, `usize`.
    Uint(u64),
    /// `f32`, `f64`.
    Float(f64),
    /// Borrowed text, checked as UTF-8.
    Str(&'a str),
    /// Borrowed bytes.
    Bytes(&'a [u8]),
    /// The caller's buffer, to write into.
    Buffer(&'a mut [u8]),
    /// A borrowed value; `None` only for a nullable one.
    Value(Option<&'a Value>),
    /// A borrowed map.
    Map(&'a Map),
    /// A borrowed list.
    List(&'a List),
    /// A value handed over.
    TakenValue(Value),
    /// A map handed over.
    TakenMap(Map),
    /// A list handed over.
    TakenList(List),
    /// The caller's value, lent for the call.
    LentValue(&'a mut Value),
    /// The caller's map, lent for the call.
    LentMap(&'a mut Map),
    /// The caller's list, lent for the call.
    LentList(&'a mut List),
    /// An object handed over: destroyed when dropped.
    Object(RawObject),
    /// An object lent for the call: called, never destroyed or kept.
    LentObject(LentObject<'a>),
}

/// What a slot answers, or what a [`Caller`] got back.
///
/// A slot's handler may answer a [`Value`] for a scalar or a text slot:
/// it is converted as `FromValue` converts it, exactly, and refused when it
/// does not fit.
#[derive(Debug)]
#[non_exhaustive]
pub enum Answer {
    /// For a slot that answers nothing.
    Nothing,
    /// A flag.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A float.
    Float(f64),
    /// Text.
    Text(String),
    /// A value, or anything a value converts to for the slot.
    Value(Value),
    /// A map.
    Map(Map),
    /// A list.
    List(List),
    /// An object, handed to the caller.
    Object(RawObject),
}

impl Answer {
    fn what(&self) -> &'static str {
        match self {
            Answer::Nothing => "nothing",
            Answer::Bool(_) => "a bool",
            Answer::Int(_) => "an integer",
            Answer::Uint(_) => "an unsigned integer",
            Answer::Float(_) => "a float",
            Answer::Text(_) => "text",
            Answer::Value(_) => "a value",
            Answer::Map(_) => "a map",
            Answer::List(_) => "a list",
            Answer::Object(_) => "an object",
        }
    }
}

/// What every slot of a [`ServedTable`] calls.
///
/// Called from whichever thread calls the table, as a kind's C table may
/// be; a panic stops at the slot and answers `GUATIAO_ERR_INTERNAL`.
pub trait Serve: Send + Sync + 'static {
    /// Slot `slot` of `kind` (an index into [`KindDescription::slots`]),
    /// called with `ctx` as the caller passed it (the provider's, an
    /// instance's or an object's, as the table's builder chose) and its
    /// arguments in order. An error is written through the slot's `err`;
    /// for a slot that cannot fail, its status alone answers, which a Rust
    /// caller treats as a broken contract.
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError>;

    /// An object kind's `destroy`: the object `ctx` is let go. Nothing by
    /// default.
    fn destroy(&self, ctx: usize) {
        let _ = ctx;
    }
}

/// Why a table could not be built or called from a description.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableError {
    /// A type the run-time tables do not carry (`i128`, `u128`), named.
    Unsupported(String),
    /// A required slot the builder was told is not implemented, named.
    MissingRequired(String),
    /// libffi refused a call interface or a closure, with its status.
    Ffi(u32),
}

impl core::fmt::Display for TableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TableError::Unsupported(what) => {
                write!(f, "{what} cannot cross a table built at run time")
            }
            TableError::MissingRequired(slot) => {
                write!(f, "the required slot `{slot}` is not implemented")
            }
            TableError::Ffi(status) => write!(f, "libffi refused the call interface ({status})"),
        }
    }
}

impl core::error::Error for TableError {}

// --- call interfaces --------------------------------------------------------

fn scalar_type(s: Scalar) -> Result<Type, TableError> {
    Ok(match s {
        Scalar::Bool | Scalar::U8 => Type::u8(),
        Scalar::I8 => Type::i8(),
        Scalar::I16 => Type::i16(),
        Scalar::I32 => Type::i32(),
        Scalar::I64 => Type::i64(),
        Scalar::Isize => Type::isize(),
        Scalar::U16 => Type::u16(),
        Scalar::U32 => Type::u32(),
        Scalar::U64 => Type::u64(),
        Scalar::Usize => Type::usize(),
        Scalar::F32 => Type::f32(),
        Scalar::F64 => Type::f64(),
        Scalar::I128 | Scalar::U128 => {
            return Err(TableError::Unsupported(format!("`{}`", s.name())));
        }
    })
}

fn view_type() -> Type {
    Type::structure([Type::pointer(), Type::usize()])
}

fn arg_type(ty: &ArgType) -> Result<Type, TableError> {
    Ok(match ty {
        ArgType::Scalar(s) => scalar_type(*s)?,
        ArgType::Str | ArgType::Bytes | ArgType::BytesMut => view_type(),
        ArgType::Tree { .. } => Type::pointer(),
        ArgType::Object {
            pass: Pass::Taken, ..
        } => Type::structure([Type::pointer(), Type::usize(), Type::pointer()]),
        ArgType::Object { .. } => Type::pointer(),
    })
}

/// A call interface libffi prepared, with the types it points into.
struct Interface(Cif);

// SAFETY: prepared once and only read after, by libffi, from any thread.
unsafe impl Send for Interface {}
// SAFETY: as above.
unsafe impl Sync for Interface {}

/// The call interface of slot `slot`: `ctx`, its arguments, `out` when it
/// answers, `err` when it can fail; a status back.
fn slot_interface(kind: &KindDescription, slot: usize) -> Result<Interface, TableError> {
    let s = &kind.slots()[slot];
    let mut args = vec![Type::pointer()];
    for a in s.args() {
        args.push(arg_type(a.ty())?);
    }
    if let Some(answer) = s.answer() {
        if let AnswerType::Scalar(scalar) = answer {
            scalar_type(*scalar)?;
        }
        args.push(Type::pointer());
    }
    if s.fallible() {
        args.push(Type::pointer());
    }
    Ok(Interface(Cif::new(args, Type::u32())))
}

/// An object kind's `destroy`: `ctx`, nothing back.
fn destroy_interface() -> Interface {
    Interface(Cif::new([Type::pointer()], Type::void()))
}

// --- the served side ----------------------------------------------------------

/// What one closure is prepared with.
struct SlotCell {
    serve: Arc<dyn Serve>,
    kind: Arc<KindDescription>,
    /// An index into the kind's slots, or `None` for `destroy`.
    slot: Option<usize>,
}

/// One libffi closure, freed with the table.
struct Closure {
    raw: *mut c_void,
}

impl Drop for Closure {
    fn drop(&mut self) {
        // SAFETY: from `ffi_closure_alloc`, freed once; no caller holds
        // the table any longer, by the `ServedTable` contract.
        unsafe { ffi::ffi_closure_free(self.raw) };
    }
}

/// A kind's table whose slots call one [`Serve`] handler, built from the
/// kind's description.
///
/// The table, its closures and the cells they read live exactly as long
/// as this value: keep it as long as anything may call the table (a
/// provider keeps it in its parts with
/// [`ProviderPartsBuilder::keep`](super::ProviderPartsBuilder::keep)).
pub struct ServedTable {
    // Dropped first: the closures read the cells and interfaces below.
    _closures: Vec<Closure>,
    _interfaces: Vec<Interface>,
    _cells: Vec<SlotCell>,
    /// The header's bytes, then a code pointer or null per slot.
    words: Vec<*const c_void>,
    kind: Arc<KindDescription>,
}

// SAFETY: built once and only read after; the closures are callable from
// any thread and call a handler that is `Send + Sync`.
unsafe impl Send for ServedTable {}
// SAFETY: as above.
unsafe impl Sync for ServedTable {}

impl core::fmt::Debug for ServedTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ServedTable")
            .field("kind", &self.kind.name())
            .field("size", &self.size())
            .finish()
    }
}

/// The words a table's header takes before its first slot.
const HEADER_WORDS: usize = size_of::<KindHeader>().div_ceil(size_of::<usize>());

impl ServedTable {
    /// Builds `kind`'s table. A slot `implemented` answers `false` for is
    /// null, so a caller runs the kind's default for it; a required one is
    /// refused. An object kind's `destroy` calls [`Serve::destroy`].
    pub fn new(
        kind: Arc<KindDescription>,
        implemented: impl Fn(&SlotDescription) -> bool,
        serve: Arc<dyn Serve>,
    ) -> Result<ServedTable, TableError> {
        let mut cells = Vec::new();
        let mut interfaces = Vec::new();
        if kind.object() {
            cells.push(SlotCell {
                serve: Arc::clone(&serve),
                kind: Arc::clone(&kind),
                slot: None,
            });
            interfaces.push(destroy_interface());
        }
        for (i, slot) in kind.slots().iter().enumerate() {
            let interface = slot_interface(&kind, i)?;
            if !implemented(slot) {
                if slot.required() {
                    return Err(TableError::MissingRequired(slot.name().to_string()));
                }
                continue;
            }
            cells.push(SlotCell {
                serve: Arc::clone(&serve),
                kind: Arc::clone(&kind),
                slot: Some(i),
            });
            interfaces.push(interface);
        }
        let words_len = HEADER_WORDS + kind.slots().len() + usize::from(kind.object());
        let mut words = vec![core::ptr::null::<c_void>(); words_len];
        let header = KindHeader::new(kind.size(), kind.floor_hash());
        let mut bytes = [0u8; size_of::<KindHeader>()];
        bytes[..4].copy_from_slice(&header.struct_size.to_ne_bytes());
        bytes[4..].copy_from_slice(&header.floor_hash.to_ne_bytes());
        for (w, chunk) in words[..HEADER_WORDS]
            .iter_mut()
            .zip(bytes.chunks(size_of::<usize>()))
        {
            let mut word = [0u8; size_of::<usize>()];
            word[..chunk.len()].copy_from_slice(chunk);
            *w = core::ptr::without_provenance(usize::from_ne_bytes(word));
        }
        let mut closures = Vec::with_capacity(cells.len());
        for (cell, interface) in cells.iter().zip(&interfaces) {
            let mut code: *mut c_void = core::ptr::null_mut();
            // SAFETY: allocates a closure and its code address, or answers
            // null.
            let raw = unsafe { ffi::ffi_closure_alloc(size_of::<ffi::ffi_closure>(), &mut code) };
            if raw.is_null() {
                return Err(TableError::Ffi(u32::MAX));
            }
            let closure = Closure { raw };
            // SAFETY: a closure just allocated; the interface and the cell
            // live in vectors this table keeps and never grows again, so
            // their addresses outlive the closure, which is freed first.
            let status = unsafe {
                ffi::ffi_prep_closure_loc(
                    raw.cast::<ffi::ffi_closure>(),
                    interface.0.as_raw_ptr(),
                    Some(on_call),
                    (cell as *const SlotCell).cast_mut().cast::<c_void>(),
                    code,
                )
            };
            if status != ffi::ffi_status_FFI_OK {
                return Err(TableError::Ffi(status));
            }
            closures.push(closure);
            let word = match cell.slot {
                None => 0,
                Some(i) => kind.word_of(i),
            };
            words[HEADER_WORDS + word] = code.cast_const();
        }
        Ok(ServedTable {
            _closures: closures,
            _interfaces: interfaces,
            _cells: cells,
            words,
            kind,
        })
    }

    /// The table, as a provider's `tables` or an object's `table` names it.
    pub fn as_ptr(&self) -> *const c_void {
        self.words.as_ptr().cast::<c_void>()
    }

    /// `sizeof` the table, as its header says.
    pub fn size(&self) -> usize {
        self.kind.size()
    }

    /// The kind it serves.
    pub fn kind(&self) -> &Arc<KindDescription> {
        &self.kind
    }

    /// This table as a provider offers it.
    pub fn kind_table(&self) -> KindTable<'_> {
        KindTable::new(self.kind.name(), self.as_ptr(), self.size())
    }

    /// An object of this kind for `ctx`, owned: what a slot answering an
    /// object hands back ([`Answer::Object`]), or what a caller passes for
    /// one. Dropping it, or the `destroy` of whoever holds it last, reaches
    /// [`Serve::destroy`] with `ctx`. `None` for a kind that is not an
    /// object kind.
    ///
    /// The table is the builder's to keep for as long as any object of it
    /// lives, as a provider's tables are: keep it with the parts of the
    /// library whose slots answer the objects, which every handle into the
    /// library holds.
    pub fn object(&self, ctx: usize) -> Option<RawObject> {
        self.kind.object().then(|| RawObject {
            raw: ObjectRaw {
                table: self.as_ptr(),
                size: self.size(),
                ctx: core::ptr::without_provenance_mut(ctx),
            },
        })
    }
}

/// Every closure's entry: the cell names the slot.
unsafe extern "C" fn on_call(
    _cif: *mut ffi::ffi_cif,
    ret: *mut c_void,
    args: *mut *mut c_void,
    data: *mut c_void,
) {
    // SAFETY: the cell this closure was prepared with, alive while the
    // table is, which its builder keeps for as long as it can be called.
    let cell = unsafe { &*data.cast::<SlotCell>() };
    match cell.slot {
        None => {
            // SAFETY: `destroy(ctx)`: one pointer argument.
            let ctx = unsafe { args.cast::<*const *mut c_void>().read().read() };
            catch_unit(|| cell.serve.destroy(ctx.addr()));
        }
        Some(slot) => {
            // SAFETY: libffi's argument array for this slot's interface.
            let status = unsafe { dispatch(&*cell.serve, &cell.kind, slot, args.cast_const()) };
            // SAFETY: libffi's return buffer, at least `ffi_arg` wide; an
            // integer answer is written widened to it.
            unsafe { ret.cast::<usize>().write(u32::from(status) as usize) };
        }
    }
}

/// Reads one argument of the shape `ty` at `at`.
///
/// # Safety
///
/// `at` addresses an argument of that shape, valid for the call.
unsafe fn read_arg<'a>(ty: &ArgType, at: *const c_void) -> Result<Arg<'a>, ProviderError> {
    // SAFETY (every arm): the caller's contract.
    unsafe {
        Ok(match ty {
            ArgType::Scalar(s) => match s {
                Scalar::Bool => Arg::Bool(at.cast::<u8>().read() != 0),
                Scalar::I8 => Arg::Int(at.cast::<i8>().read().into()),
                Scalar::I16 => Arg::Int(at.cast::<i16>().read().into()),
                Scalar::I32 => Arg::Int(at.cast::<i32>().read().into()),
                Scalar::I64 => Arg::Int(at.cast::<i64>().read()),
                Scalar::Isize => Arg::Int(at.cast::<isize>().read() as i64),
                Scalar::U8 => Arg::Uint(at.cast::<u8>().read().into()),
                Scalar::U16 => Arg::Uint(at.cast::<u16>().read().into()),
                Scalar::U32 => Arg::Uint(at.cast::<u32>().read().into()),
                Scalar::U64 => Arg::Uint(at.cast::<u64>().read()),
                Scalar::Usize => Arg::Uint(at.cast::<usize>().read() as u64),
                Scalar::F32 => Arg::Float(at.cast::<f32>().read().into()),
                Scalar::F64 => Arg::Float(at.cast::<f64>().read()),
                _ => return Err(ProviderError::from(Status::GUATIAO_ERR_UNSUPPORTED)),
            },
            ArgType::Str => {
                let bytes: &[u8] = at.cast::<Str<'a>>().read().into();
                Arg::Str(core::str::from_utf8(bytes).map_err(|_| {
                    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "an argument is not UTF-8")
                })?)
            }
            ArgType::Bytes => Arg::Bytes(at.cast::<Bytes<'a>>().read().into()),
            ArgType::BytesMut => Arg::Buffer(at.cast::<BytesMut<'a>>().read().into()),
            ArgType::Tree {
                tree,
                pass,
                nullable,
            } => {
                let p = at.cast::<*mut c_void>().read();
                match (tree, pass) {
                    (Tree::Value, Pass::Borrowed) if *nullable => {
                        Arg::Value(value_opt(p.cast::<Value>()))
                    }
                    (Tree::Value, Pass::Borrowed) => Arg::Value(Some(value_arg(p.cast())?)),
                    (Tree::Map, Pass::Borrowed) => Arg::Map(map_arg(p.cast())?),
                    (Tree::List, Pass::Borrowed) => {
                        Arg::List(p.cast::<List>().as_ref().ok_or(Status::GUATIAO_ERR_NULL)?)
                    }
                    (Tree::Value, Pass::Lent) => Arg::LentValue(value_mut_arg(p.cast())?),
                    (Tree::Map, Pass::Lent) => Arg::LentMap(map_mut_arg(p.cast())?),
                    (Tree::List, Pass::Lent) => Arg::LentList(list_mut_arg(p.cast())?),
                    // Taken trees were read before anything else.
                    _ => return Err(ProviderError::from(Status::GUATIAO_ERR_INTERNAL)),
                }
            }
            ArgType::Object {
                pass: Pass::Lent, ..
            } => {
                let raw = at.cast::<*const ObjectRaw>().read();
                Arg::LentObject(LentObject {
                    raw: raw.as_ref().ok_or_else(|| {
                        ProviderError::new(Status::GUATIAO_ERR_NULL, "no object was lent")
                    })?,
                })
            }
            _ => return Err(ProviderError::from(Status::GUATIAO_ERR_INTERNAL)),
        })
    }
}

/// Whether an argument is handed over: read before anything else, so an
/// early refusal still frees it.
fn is_taken(ty: &ArgType) -> bool {
    matches!(
        ty,
        ArgType::Tree {
            pass: Pass::Taken,
            ..
        } | ArgType::Object {
            pass: Pass::Taken,
            ..
        }
    )
}

/// Takes ownership of a handed-over argument at `at`.
///
/// # Safety
///
/// `at` addresses an argument of that shape the caller gave up.
unsafe fn take_arg<'a>(ty: &ArgType, at: *const c_void) -> Result<Arg<'a>, ProviderError> {
    // SAFETY (every arm): the caller's contract.
    unsafe {
        Ok(match ty {
            ArgType::Tree { tree, .. } => {
                let p = at.cast::<*mut c_void>().read();
                match tree {
                    Tree::Value => Arg::TakenValue(value_owned(p.cast())?),
                    Tree::Map => Arg::TakenMap(map_owned(p.cast())?),
                    Tree::List => Arg::TakenList(list_owned(p.cast())?),
                }
            }
            _ => Arg::Object(RawObject::from_raw(at.cast::<ObjectRaw>().read())),
        })
    }
}

/// One served slot called: reads its arguments from libffi's array, calls
/// the handler, writes the answer or the error. Plain Rust over the array,
/// so it runs under Miri without libffi.
///
/// # Safety
///
/// `args` is the argument array of a call to `kind`'s slot `slot`, as
/// libffi hands a closure one: a pointer per argument (`ctx` first, then
/// the slot's arguments, `out` if it answers, `err` if it can fail), each
/// addressing an argument of the slot's described shape.
pub(crate) unsafe fn dispatch(
    serve: &dyn Serve,
    kind: &KindDescription,
    slot: usize,
    args: *const *mut c_void,
) -> Status {
    let s = &kind.slots()[slot];
    let n = s.args().len();
    // SAFETY: the caller's contract, for every read of `args` below.
    let at = |i: usize| unsafe { args.add(i).read().cast_const() };
    // SAFETY: as above: the first argument is `ctx`.
    let ctx = unsafe { at(0).cast::<*mut c_void>().read() };
    let out = if s.answer().is_some() {
        // SAFETY: as above.
        unsafe { at(1 + n).cast::<*mut c_void>().read() }
    } else {
        core::ptr::null_mut()
    };
    let err = if s.fallible() {
        let i = 1 + n + usize::from(s.answer().is_some());
        // SAFETY: as above.
        unsafe { at(i).cast::<*mut ProviderError>().read() }
    } else {
        core::ptr::null_mut()
    };
    catch(|| {
        // Handed-over arguments first, so a refusal below drops them.
        let mut taken: Vec<Option<Arg<'_>>> = Vec::with_capacity(n);
        let mut refused = None;
        for (i, a) in s.args().iter().enumerate() {
            if is_taken(a.ty()) {
                // SAFETY: the caller's contract.
                match unsafe { take_arg(a.ty(), at(1 + i)) } {
                    Ok(arg) => taken.push(Some(arg)),
                    Err(e) => {
                        taken.push(None);
                        refused.get_or_insert(e);
                    }
                }
            } else {
                taken.push(None);
            }
        }
        let mut read = Vec::with_capacity(n);
        if refused.is_none() {
            for (i, a) in s.args().iter().enumerate() {
                if let Some(arg) = taken[i].take() {
                    read.push(arg);
                    continue;
                }
                // SAFETY: the caller's contract.
                match unsafe { read_arg(a.ty(), at(1 + i)) } {
                    Ok(arg) => read.push(arg),
                    Err(e) => {
                        refused = Some(e);
                        break;
                    }
                }
            }
        }
        let answered = match refused {
            Some(e) => Err(e),
            None => {
                drop(taken);
                serve.call(ctx.addr(), kind, slot, read)
            }
        };
        // SAFETY: `out` and `err` as the slot's interface gives them.
        unsafe { answer(s, answered, out, err) }
    })
}

/// Writes what a handler answered through the slot's `out`, or its error
/// through `err`.
///
/// # Safety
///
/// `out` is null or the slot's out slot of its described answer type,
/// holding a carrier for a tree; `err` is null or writable.
unsafe fn answer(
    s: &SlotDescription,
    answered: Result<Answer, ProviderError>,
    out: *mut c_void,
    err: *mut ProviderError,
) -> Status {
    let written = answered.and_then(|a| {
        // SAFETY: forwarded.
        unsafe { write_answer(s.answer(), a, out) }
    });
    match written {
        Ok(status) => status,
        Err(mut e) => {
            // An error that says OK would read as an answer never written.
            if e.status == Status::GUATIAO_OK {
                e.status = Status::GUATIAO_ERR_INTERNAL;
            }
            if s.fallible() {
                // SAFETY: the caller's contract.
                unsafe { write_err(err, e) }
            } else {
                e.status
            }
        }
    }
}

fn wrong(answer: &Answer, want: &str) -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_WRONG_KIND,
        &format!(
            "the provider answered {} where the kind answers {want}",
            answer.what()
        ),
    )
}

fn out_of_range(scalar: Scalar) -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_BAD_VALUE,
        &format!("the provider's answer does not fit `{}`", scalar.name()),
    )
}

/// The answer as `T`, from whatever number or value it holds.
fn number<T>(a: Answer, scalar: Scalar) -> Result<T, ProviderError>
where
    T: TryFrom<i64> + TryFrom<u64> + FromValue,
{
    match a {
        Answer::Int(n) => T::try_from(n).map_err(|_| out_of_range(scalar)),
        Answer::Uint(n) => T::try_from(n).map_err(|_| out_of_range(scalar)),
        Answer::Value(v) => T::from_value(&v).map_err(|_| out_of_range(scalar)),
        other => Err(wrong(&other, scalar.name())),
    }
}

/// # Safety
///
/// `out` is null or addresses the slot's out slot for `ty`.
unsafe fn write_scalar(
    scalar: Scalar,
    a: Answer,
    out: *mut c_void,
) -> Result<Status, ProviderError> {
    // SAFETY (every arm): the caller's contract.
    unsafe {
        Ok(match scalar {
            Scalar::Bool => match a {
                Answer::Bool(b) => write_out(out.cast::<bool>(), b),
                Answer::Value(v) => write_out(
                    out.cast::<bool>(),
                    bool::from_value(&v).map_err(|_| out_of_range(scalar))?,
                ),
                other => return Err(wrong(&other, "a bool")),
            },
            Scalar::I8 => write_out(out.cast::<i8>(), number(a, scalar)?),
            Scalar::I16 => write_out(out.cast::<i16>(), number(a, scalar)?),
            Scalar::I32 => write_out(out.cast::<i32>(), number(a, scalar)?),
            Scalar::I64 => write_out(out.cast::<i64>(), number(a, scalar)?),
            Scalar::Isize => write_out(out.cast::<isize>(), number(a, scalar)?),
            Scalar::U8 => write_out(out.cast::<u8>(), number(a, scalar)?),
            Scalar::U16 => write_out(out.cast::<u16>(), number(a, scalar)?),
            Scalar::U32 => write_out(out.cast::<u32>(), number(a, scalar)?),
            Scalar::U64 => write_out(out.cast::<u64>(), number(a, scalar)?),
            Scalar::Usize => write_out(out.cast::<usize>(), number(a, scalar)?),
            Scalar::F32 | Scalar::F64 => {
                let f = match a {
                    Answer::Float(f) => f,
                    Answer::Int(n) => n as f64,
                    Answer::Uint(n) => n as f64,
                    Answer::Value(v) => f64::from_value(&v).map_err(|_| out_of_range(scalar))?,
                    other => return Err(wrong(&other, "a float")),
                };
                if scalar == Scalar::F32 {
                    write_out(out.cast::<f32>(), f as f32)
                } else {
                    write_out(out.cast::<f64>(), f)
                }
            }
            _ => return Err(ProviderError::from(Status::GUATIAO_ERR_UNSUPPORTED)),
        })
    }
}

/// # Safety
///
/// As [`answer`].
unsafe fn write_answer(
    ty: Option<&AnswerType>,
    a: Answer,
    out: *mut c_void,
) -> Result<Status, ProviderError> {
    let Some(ty) = ty else {
        return Ok(Status::GUATIAO_OK);
    };
    // SAFETY (every arm): the caller's contract.
    unsafe {
        Ok(match ty {
            AnswerType::Scalar(scalar) => write_scalar(*scalar, a, out)?,
            AnswerType::Text => {
                let text = match a {
                    Answer::Text(t) => t,
                    Answer::Value(v) => {
                        String::from_value(&v).map_err(|_| wrong(&Answer::Value(v), "text"))?
                    }
                    other => return Err(wrong(&other, "text")),
                };
                write_out(out.cast::<Text>(), Text::new(&text))
            }
            AnswerType::Tree(Tree::Map) => {
                let map = match a {
                    Answer::Map(m) => m,
                    Answer::Value(v) => Map::try_from(v)
                        .map_err(|_| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, "the provider answered a value that is not a map where the kind answers a map"))?,
                    other => return Err(wrong(&other, "a map")),
                };
                if out.is_null() {
                    return Ok(Status::GUATIAO_ERR_NULL);
                }
                let home = return_alloc(out.cast_const().cast::<Map>());
                write_out(out.cast::<Map>(), map_ret(map, home)?)
            }
            AnswerType::Tree(Tree::List) => {
                let list = match a {
                    Answer::List(l) => l,
                    Answer::Value(v) => List::try_from(v)
                        .map_err(|_| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, "the provider answered a value that is not a list where the kind answers a list"))?,
                    other => return Err(wrong(&other, "a list")),
                };
                if out.is_null() {
                    return Ok(Status::GUATIAO_ERR_NULL);
                }
                let home = return_alloc(out.cast_const().cast::<List>());
                write_out(out.cast::<List>(), list_ret(list, home)?)
            }
            AnswerType::Tree(Tree::Value) => {
                let value = match a {
                    Answer::Value(v) => v,
                    Answer::Map(m) => Value::from(m),
                    Answer::List(l) => Value::from(l),
                    Answer::Bool(b) => Value::from(b),
                    Answer::Int(n) => Value::from(n),
                    Answer::Uint(n) => Value::from(n),
                    Answer::Float(f) => Value::from(
                        crate::value::types::Number::try_from(f).map_err(ProviderError::from)?,
                    ),
                    Answer::Text(t) => Value::from(t.as_str()),
                    other => return Err(wrong(&other, "a value")),
                };
                if out.is_null() {
                    return Ok(Status::GUATIAO_ERR_NULL);
                }
                let home = return_alloc(out.cast_const().cast::<Value>());
                write_out(out.cast::<Value>(), value_ret(value, home)?)
            }
            AnswerType::Object { .. } => match a {
                Answer::Object(object) => {
                    if out.is_null() {
                        // Dropped: destroyed, since nobody can receive it.
                        drop(object);
                        return Ok(Status::GUATIAO_ERR_NULL);
                    }
                    write_out(out.cast::<ObjectRaw>(), object.into_raw())
                }
                other => return Err(wrong(&other, "an object")),
            },
        })
    }
}

// --- objects ------------------------------------------------------------------

/// An object handed over through a slot: its table, size and `ctx`, owned.
/// Dropping it calls the table's `destroy` once.
#[derive(Debug)]
pub struct RawObject {
    raw: ObjectRaw,
}

// SAFETY: an object crosses the C ABI to whichever thread calls next; its
// kind's contract is one caller at a time, which an owner gives.
unsafe impl Send for RawObject {}

impl RawObject {
    /// Takes ownership of an object a C caller handed over.
    ///
    /// # Safety
    ///
    /// `raw.table` is null or addresses `raw.size` readable bytes of an
    /// object kind's table (a header, then `destroy`) valid while the
    /// object lives; `ctx` is what its slots take; nothing else destroys it.
    pub unsafe fn from_raw(raw: ObjectRaw) -> RawObject {
        RawObject { raw }
    }

    /// Gives the object up without destroying it.
    pub fn into_raw(self) -> ObjectRaw {
        let this = ManuallyDrop::new(self);
        this.raw
    }

    /// The table, size and `ctx`.
    pub fn as_raw(&self) -> &ObjectRaw {
        &self.raw
    }

    /// Lends the object to a slot that takes a lent one: it stays this
    /// owner's, which outlives the lend.
    pub fn lend(&self) -> LentObject<'_> {
        LentObject { raw: &self.raw }
    }

    /// Calls slot `slot` of the object through `caller`, whose kind it must
    /// be.
    pub fn call(
        &mut self,
        caller: &Caller,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        // SAFETY: an owned object's table and `ctx`, by `from_raw`'s
        // contract; `call` checks the header against the kind.
        unsafe { caller.call_raw(self.raw.table, self.raw.size, self.raw.ctx, slot, args) }
    }
}

impl Drop for RawObject {
    fn drop(&mut self) {
        let raw = self.raw;
        if raw.table.is_null() || raw.size < size_of::<KindHeader>() + size_of::<usize>() {
            return;
        }
        // SAFETY: an object kind's table holds `destroy` right after its
        // header, by `from_raw`'s contract; a null one is skipped.
        let destroy = unsafe {
            raw.table
                .cast::<u8>()
                .add(size_of::<KindHeader>())
                .cast::<Option<unsafe extern "C" fn(*mut c_void)>>()
                .read_unaligned()
        };
        if let Some(destroy) = destroy {
            // SAFETY: the object's own `destroy`, called once, here.
            unsafe { destroy(raw.ctx) };
        }
    }
}

/// An object lent for one call: called, never destroyed or kept.
#[derive(Debug, Clone, Copy)]
pub struct LentObject<'a> {
    raw: &'a ObjectRaw,
}

impl<'a> LentObject<'a> {
    /// Lends an object a caller holds.
    ///
    /// # Safety
    ///
    /// As [`RawObject::from_raw`], for `'a`, without the ownership.
    pub unsafe fn from_raw(raw: &'a ObjectRaw) -> LentObject<'a> {
        LentObject { raw }
    }

    /// The table, size and `ctx`.
    pub fn as_raw(&self) -> &'a ObjectRaw {
        self.raw
    }

    /// Calls slot `slot` of the lent object through `caller`.
    pub fn call(
        &self,
        caller: &Caller,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        // SAFETY: a lent object's table and `ctx`, by `from_raw`'s contract.
        unsafe { caller.call_raw(self.raw.table, self.raw.size, self.raw.ctx, slot, args) }
    }

    /// Runs `f` with a handle on the lent object that another thread may
    /// call while `f` runs (an engine's own thread, while this one waits),
    /// and that refuses every call once `f` returns.
    pub fn share<R>(&self, f: impl FnOnce(LentHandle) -> R) -> R {
        let cell = Arc::new(std::sync::RwLock::new(Lend(Some(*self.raw))));
        let answer = f(LentHandle {
            cell: Arc::clone(&cell),
        });
        // A call in flight holds the read side: the lend ends after it.
        cell.write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0 = None;
        answer
    }
}

/// A lent object, callable from any thread until the lend that made it
/// ends ([`LentObject::share`]); every call after answers
/// `GUATIAO_ERR_GONE`.
#[derive(Debug, Clone)]
pub struct LentHandle {
    cell: Arc<std::sync::RwLock<Lend>>,
}

/// The lent object while the lend lasts.
#[derive(Debug)]
struct Lend(Option<ObjectRaw>);

// SAFETY: the object is the lender's for the length of its call, and the
// lender waits while another thread calls it; once the lend ends nothing
// is read through it.
unsafe impl Send for Lend {}
// SAFETY: as above.
unsafe impl Sync for Lend {}

impl LentHandle {
    /// Calls slot `slot` of the lent object through `caller`, or answers
    /// `GUATIAO_ERR_GONE` after the lend.
    pub fn call(
        &self,
        caller: &Caller,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let lent = self
            .cell
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(raw) = lent.0 else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the object was lent for a call that has returned",
            ));
        };
        // SAFETY: the lent object's table and `ctx`, alive while the lend
        // lasts, which the read side held across the call guarantees.
        unsafe { caller.call_raw(raw.table, raw.size, raw.ctx, slot, args) }
    }

    /// The lent object, passed on to a slot that takes a lent object, for
    /// as long as `f` runs. `None` after the lend.
    pub fn lend<R>(&self, f: impl FnOnce(Option<LentObject<'_>>) -> R) -> R {
        let lent = self
            .cell
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(lent.0.as_ref().map(|raw| LentObject { raw }))
    }
}

// --- the calling side -----------------------------------------------------------

/// Calls any table of one described kind: the call interfaces, prepared
/// once.
pub struct Caller {
    kind: Arc<KindDescription>,
    interfaces: Vec<Option<Interface>>,
}

impl core::fmt::Debug for Caller {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Caller")
            .field("kind", &self.kind.name())
            .finish()
    }
}

/// One argument's storage for `ffi_call`.
enum Cell<'a> {
    U8(u8),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    Isize(isize),
    U16(u16),
    U32(u32),
    U64(u64),
    Usize(usize),
    F32(f32),
    F64(f64),
    Str(Str<'a>),
    Bytes(Bytes<'a>),
    BytesMut(BytesMut<'a>),
    Ptr(*mut c_void),
    Object(ObjectRaw),
}

impl Cell<'_> {
    fn address(&mut self) -> *mut c_void {
        match self {
            Cell::U8(v) => (v as *mut u8).cast(),
            Cell::I8(v) => (v as *mut i8).cast(),
            Cell::I16(v) => (v as *mut i16).cast(),
            Cell::I32(v) => (v as *mut i32).cast(),
            Cell::I64(v) => (v as *mut i64).cast(),
            Cell::Isize(v) => (v as *mut isize).cast(),
            Cell::U16(v) => (v as *mut u16).cast(),
            Cell::U32(v) => (v as *mut u32).cast(),
            Cell::U64(v) => (v as *mut u64).cast(),
            Cell::Usize(v) => (v as *mut usize).cast(),
            Cell::F32(v) => (v as *mut f32).cast(),
            Cell::F64(v) => (v as *mut f64).cast(),
            Cell::Str(v) => (v as *mut Str<'_>).cast(),
            Cell::Bytes(v) => (v as *mut Bytes<'_>).cast(),
            Cell::BytesMut(v) => (v as *mut BytesMut<'_>).cast(),
            Cell::Ptr(v) => (v as *mut *mut c_void).cast(),
            Cell::Object(v) => (v as *mut ObjectRaw).cast(),
        }
    }
}

fn mismatch(i: usize, want: &str) -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_WRONG_KIND,
        &format!("argument {i} is not {want}"),
    )
}

fn narrow<T: TryFrom<i64> + TryFrom<u64>>(
    arg: &Arg<'_>,
    i: usize,
    s: Scalar,
) -> Result<T, ProviderError> {
    let fits = || {
        ProviderError::new(
            Status::GUATIAO_ERR_BAD_VALUE,
            &format!("argument {i} does not fit `{}`", s.name()),
        )
    };
    match arg {
        Arg::Int(n) => T::try_from(*n).map_err(|_| fits()),
        Arg::Uint(n) => T::try_from(*n).map_err(|_| fits()),
        _ => Err(mismatch(i, "a number")),
    }
}

/// What a tree or object argument becomes for the call, and what is done
/// after it. A handed-over one is dropped here only when the call never
/// happened; once it is made, the callee owns it.
enum Pending<'a> {
    Nothing,
    Moved(Value),
    MovedMap(Map),
    MovedList(List),
    Object(RawObject),
    /// Lent: settled into its home after the call.
    LentValue(&'a mut Value, Alloc),
    LentMap(&'a mut Map, Alloc),
    LentList(&'a mut List, Alloc),
}

impl Caller {
    /// Prepares the call interfaces of every slot of `kind`.
    pub fn new(kind: Arc<KindDescription>) -> Result<Caller, TableError> {
        let mut interfaces = Vec::new();
        for i in 0..kind.slots().len() {
            interfaces.push(Some(slot_interface(&kind, i)?));
        }
        Ok(Caller { kind, interfaces })
    }

    /// The kind it calls.
    pub fn kind(&self) -> &Arc<KindDescription> {
        &self.kind
    }

    /// Calls slot `slot` of a table of this kind.
    ///
    /// The table's header must carry the kind's floor hash and a size
    /// reaching the slot; a slot the table leaves null answers
    /// `GUATIAO_ERR_UNSUPPORTED`, the caller choosing a default. Arguments
    /// must match the slot's description in number and shape.
    ///
    /// # Safety
    ///
    /// `table` is null or addresses `size` readable bytes of a table built
    /// for this kind, valid for the call; `ctx` is what its slots take.
    pub unsafe fn call_raw(
        &self,
        table: *const c_void,
        size: usize,
        ctx: *mut c_void,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let kind = &*self.kind;
        let Some(s) = kind.slots().get(slot) else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                "the kind has no such slot",
            ));
        };
        if table.is_null() || size < size_of::<KindHeader>() {
            return Err(ProviderError::new(Status::GUATIAO_ERR_NULL, "no table"));
        }
        // SAFETY: the caller's contract: at least the header is readable.
        let header = unsafe { table.cast::<KindHeader>().read_unaligned() };
        if header.floor_hash != kind.floor_hash() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                "the table was not built for this kind",
            ));
        }
        // The table's own size and the one given agree, or the smaller wins.
        let declared = match header.struct_size as usize {
            0 => size,
            own => own.min(size),
        };
        if declared < kind.floor() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                "the table is shorter than the kind's last required slot",
            ));
        }
        let word = kind.word_of(slot);
        let end = size_of::<KindHeader>() + (word + 1) * size_of::<usize>();
        if declared < end {
            return Err(ProviderError::from(Status::GUATIAO_ERR_UNSUPPORTED));
        }
        // SAFETY: within the declared size, a pointer-sized slot.
        let f = unsafe {
            table
                .cast::<u8>()
                .add(size_of::<KindHeader>() + word * size_of::<usize>())
                .cast::<Option<unsafe extern "C" fn()>>()
                .read_unaligned()
        };
        let Some(f) = f else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                &format!("the table does not fill `{}`", s.name()),
            ));
        };
        if args.len() != s.args().len() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!(
                    "`{}` takes {} arguments, and {} were given",
                    s.name(),
                    s.args().len(),
                    args.len()
                ),
            ));
        }
        let Some(Some(interface)) = self.interfaces.get(slot) else {
            return Err(ProviderError::from(Status::GUATIAO_ERR_INTERNAL));
        };

        // Every argument's storage, laid out before any address is taken.
        let mut cells: Vec<Cell<'_>> = Vec::with_capacity(s.args().len() + 3);
        let mut pending: Vec<Pending<'_>> = Vec::with_capacity(s.args().len());
        cells.push(Cell::Ptr(ctx));
        for (i, (a, d)) in args.iter_mut().zip(s.args()).enumerate() {
            let taken = core::mem::replace(a, Arg::Bool(false));
            let (cell, after) = match (d.ty(), taken) {
                (ArgType::Scalar(sc), arg) => (
                    match sc {
                        Scalar::Bool => match arg {
                            Arg::Bool(b) => Cell::U8(u8::from(b)),
                            _ => return Err(mismatch(i, "a bool")),
                        },
                        Scalar::I8 => Cell::I8(narrow(&arg, i, *sc)?),
                        Scalar::I16 => Cell::I16(narrow(&arg, i, *sc)?),
                        Scalar::I32 => Cell::I32(narrow(&arg, i, *sc)?),
                        Scalar::I64 => Cell::I64(narrow(&arg, i, *sc)?),
                        Scalar::Isize => Cell::Isize(narrow(&arg, i, *sc)?),
                        Scalar::U8 => Cell::U8(narrow(&arg, i, *sc)?),
                        Scalar::U16 => Cell::U16(narrow(&arg, i, *sc)?),
                        Scalar::U32 => Cell::U32(narrow(&arg, i, *sc)?),
                        Scalar::U64 => Cell::U64(narrow(&arg, i, *sc)?),
                        Scalar::Usize => Cell::Usize(narrow(&arg, i, *sc)?),
                        Scalar::F32 | Scalar::F64 => {
                            let f = match arg {
                                Arg::Float(f) => f,
                                Arg::Int(n) => n as f64,
                                Arg::Uint(n) => n as f64,
                                _ => return Err(mismatch(i, "a number")),
                            };
                            if *sc == Scalar::F32 {
                                Cell::F32(f as f32)
                            } else {
                                Cell::F64(f)
                            }
                        }
                        _ => return Err(ProviderError::from(Status::GUATIAO_ERR_UNSUPPORTED)),
                    },
                    Pending::Nothing,
                ),
                (ArgType::Str, Arg::Str(t)) => (Cell::Str(Str::new(t)), Pending::Nothing),
                (ArgType::Bytes, Arg::Bytes(b)) => (Cell::Bytes(Bytes::new(b)), Pending::Nothing),
                (ArgType::BytesMut, Arg::Buffer(b)) => {
                    (Cell::BytesMut(BytesMut::new(b)), Pending::Nothing)
                }
                (
                    ArgType::Tree {
                        tree,
                        pass,
                        nullable,
                    },
                    arg,
                ) => match (tree, pass, arg) {
                    (Tree::Value, Pass::Borrowed, Arg::Value(v)) => {
                        if v.is_none() && !nullable {
                            return Err(mismatch(i, "a value"));
                        }
                        (
                            Cell::Ptr(v.map_or(core::ptr::null_mut(), |v| {
                                (v as *const Value).cast_mut().cast()
                            })),
                            Pending::Nothing,
                        )
                    }
                    (Tree::Map, Pass::Borrowed, Arg::Map(m)) => (
                        Cell::Ptr((m as *const Map).cast_mut().cast()),
                        Pending::Nothing,
                    ),
                    (Tree::List, Pass::Borrowed, Arg::List(l)) => (
                        Cell::Ptr((l as *const List).cast_mut().cast()),
                        Pending::Nothing,
                    ),
                    (Tree::Value, Pass::Taken, Arg::TakenValue(v)) => {
                        (Cell::Ptr(core::ptr::null_mut()), Pending::Moved(v))
                    }
                    (Tree::Map, Pass::Taken, Arg::TakenMap(m)) => {
                        (Cell::Ptr(core::ptr::null_mut()), Pending::MovedMap(m))
                    }
                    (Tree::List, Pass::Taken, Arg::TakenList(l)) => {
                        (Cell::Ptr(core::ptr::null_mut()), Pending::MovedList(l))
                    }
                    (Tree::Value, Pass::Lent, Arg::LentValue(v)) => {
                        let home = lent_home(&*v);
                        (
                            Cell::Ptr(core::ptr::null_mut()),
                            Pending::LentValue(v, home),
                        )
                    }
                    (Tree::Map, Pass::Lent, Arg::LentMap(m)) => {
                        let home = lent_home(&*m);
                        (Cell::Ptr(core::ptr::null_mut()), Pending::LentMap(m, home))
                    }
                    (Tree::List, Pass::Lent, Arg::LentList(l)) => {
                        let home = lent_home(&*l);
                        (Cell::Ptr(core::ptr::null_mut()), Pending::LentList(l, home))
                    }
                    _ => return Err(mismatch(i, "the tree the slot takes")),
                },
                (
                    ArgType::Object {
                        pass: Pass::Taken, ..
                    },
                    Arg::Object(o),
                ) => (Cell::Object(*o.as_raw()), Pending::Object(o)),
                (ArgType::Object { .. }, Arg::LentObject(o)) => (
                    Cell::Ptr((o.raw as *const ObjectRaw).cast_mut().cast()),
                    Pending::Nothing,
                ),
                _ => return Err(mismatch(i, "what the slot takes")),
            };
            cells.push(cell);
            pending.push(after);
        }
        // Trees: their addresses, now that nothing moves them again.
        for (k, p) in pending.iter_mut().enumerate() {
            let at = match p {
                Pending::Nothing | Pending::Object(_) => continue,
                Pending::Moved(v) => (v as *mut Value).cast(),
                Pending::MovedMap(m) => (m as *mut Map).cast(),
                Pending::MovedList(l) => (l as *mut List).cast(),
                Pending::LentValue(v, _) => (*v as *mut Value).cast(),
                Pending::LentMap(m, _) => (*m as *mut Map).cast(),
                Pending::LentList(l, _) => (*l as *mut List).cast(),
            };
            cells[1 + k] = Cell::Ptr(at);
        }
        // The answer's storage: a carrier in this side's allocator.
        let mut out = Out::for_answer(s.answer())?;
        if let Some(at) = out.address() {
            cells.push(Cell::Ptr(at));
        }
        let mut err = ProviderError::none();
        if s.fallible() {
            cells.push(Cell::Ptr((&mut err as *mut ProviderError).cast()));
        }
        let mut addresses: Vec<*mut c_void> = cells.iter_mut().map(Cell::address).collect();
        let mut ret: usize = 0;
        // SAFETY: the interface was prepared for this slot's description,
        // which the table was built for (its hash checked); each address
        // holds an argument of the shape the interface names, and lives
        // until the call returns.
        unsafe {
            ffi::ffi_call(
                interface.0.as_raw_ptr(),
                Some(f),
                (&mut ret as *mut usize).cast(),
                addresses.as_mut_ptr(),
            );
        }
        let status = Status::from(ret as u32);
        // What was handed over is the callee's, whatever it answered; lent
        // trees go home, whatever it answered.
        let mut settled = Ok(());
        for p in pending {
            let one = match p {
                Pending::Moved(v) => {
                    core::mem::forget(v);
                    Ok(())
                }
                Pending::MovedMap(m) => {
                    core::mem::forget(m);
                    Ok(())
                }
                Pending::MovedList(l) => {
                    core::mem::forget(l);
                    Ok(())
                }
                Pending::Object(o) => {
                    o.into_raw();
                    Ok(())
                }
                Pending::LentValue(v, home) => lent_settle(v, home),
                Pending::LentMap(m, home) => lent_settle(m, home),
                Pending::LentList(l, home) => lent_settle(l, home),
                Pending::Nothing => Ok(()),
            };
            settled = settled.and(one);
        }
        if status != Status::GUATIAO_OK {
            let e = if s.fallible() {
                take_err(err, status)
            } else {
                drop(err);
                ProviderError::from(status)
            };
            // SAFETY: a refused call wrote no answer: the carrier owns nothing.
            unsafe { out.forget() };
            return Err(e);
        }
        drop(err);
        settled?;
        out.into_answer()
    }
}

/// The storage a call's answer lands in.
enum Out {
    Nothing,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    Isize(isize),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    Usize(usize),
    F32(f32),
    F64(f64),
    Text(Text),
    Value(Value),
    Map(Map),
    List(List),
    Object(ObjectRaw),
}

impl Out {
    fn for_answer(ty: Option<&AnswerType>) -> Result<Out, ProviderError> {
        Ok(match ty {
            None => Out::Nothing,
            Some(AnswerType::Scalar(s)) => match s {
                Scalar::Bool => Out::Bool(false),
                Scalar::I8 => Out::I8(0),
                Scalar::I16 => Out::I16(0),
                Scalar::I32 => Out::I32(0),
                Scalar::I64 => Out::I64(0),
                Scalar::Isize => Out::Isize(0),
                Scalar::U8 => Out::U8(0),
                Scalar::U16 => Out::U16(0),
                Scalar::U32 => Out::U32(0),
                Scalar::U64 => Out::U64(0),
                Scalar::Usize => Out::Usize(0),
                Scalar::F32 => Out::F32(0.0),
                Scalar::F64 => Out::F64(0.0),
                _ => return Err(ProviderError::from(Status::GUATIAO_ERR_UNSUPPORTED)),
            },
            Some(AnswerType::Text) => Out::Text(Text::new("")),
            Some(AnswerType::Tree(Tree::Value)) => Out::Value(Value::carrier(Alloc::rust())),
            Some(AnswerType::Tree(Tree::Map)) => Out::Map(Map::carrier(Alloc::rust())),
            Some(AnswerType::Tree(Tree::List)) => Out::List(List::carrier(Alloc::rust())),
            Some(AnswerType::Object { .. }) => Out::Object(ObjectRaw::null()),
        })
    }

    fn address(&mut self) -> Option<*mut c_void> {
        Some(match self {
            Out::Nothing => return None,
            Out::Bool(v) => (v as *mut bool).cast(),
            Out::I8(v) => (v as *mut i8).cast(),
            Out::I16(v) => (v as *mut i16).cast(),
            Out::I32(v) => (v as *mut i32).cast(),
            Out::I64(v) => (v as *mut i64).cast(),
            Out::Isize(v) => (v as *mut isize).cast(),
            Out::U8(v) => (v as *mut u8).cast(),
            Out::U16(v) => (v as *mut u16).cast(),
            Out::U32(v) => (v as *mut u32).cast(),
            Out::U64(v) => (v as *mut u64).cast(),
            Out::Usize(v) => (v as *mut usize).cast(),
            Out::F32(v) => (v as *mut f32).cast(),
            Out::F64(v) => (v as *mut f64).cast(),
            Out::Text(v) => (v as *mut Text).cast(),
            Out::Value(v) => (v as *mut Value).cast(),
            Out::Map(v) => (v as *mut Map).cast(),
            Out::List(v) => (v as *mut List).cast(),
            Out::Object(v) => (v as *mut ObjectRaw).cast(),
        })
    }

    /// Lets a carrier go unread after a refusal.
    ///
    /// # Safety
    ///
    /// The callee wrote nothing over it, as a refusing slot does not.
    unsafe fn forget(self) {
        core::mem::forget(self);
    }

    fn into_answer(self) -> Result<Answer, ProviderError> {
        Ok(match self {
            Out::Nothing => Answer::Nothing,
            Out::Bool(b) => Answer::Bool(b),
            Out::I8(n) => Answer::Int(n.into()),
            Out::I16(n) => Answer::Int(n.into()),
            Out::I32(n) => Answer::Int(n.into()),
            Out::I64(n) => Answer::Int(n),
            Out::Isize(n) => Answer::Int(n as i64),
            Out::U8(n) => Answer::Uint(n.into()),
            Out::U16(n) => Answer::Uint(n.into()),
            Out::U32(n) => Answer::Uint(n.into()),
            Out::U64(n) => Answer::Uint(n),
            Out::Usize(n) => Answer::Uint(n as u64),
            Out::F32(f) => Answer::Float(f.into()),
            Out::F64(f) => Answer::Float(f),
            Out::Text(t) => Answer::Text(text_ret(t)?.to_string()),
            Out::Value(v) => Answer::Value(value_ret(v, Alloc::rust())?),
            Out::Map(m) => Answer::Map(map_ret(m, Alloc::rust())?),
            Out::List(l) => Answer::List(list_ret(l, Alloc::rust())?),
            // SAFETY: the callee handed this object over through `out`.
            Out::Object(o) => Answer::Object(unsafe { RawObject::from_raw(o) }),
        })
    }
}

/// A provider's table of a described kind, held as a [`Remote`] holds one:
/// its library stays loaded while this lives.
pub struct DescribedRemote {
    caller: Arc<Caller>,
    table: *const c_void,
    size: usize,
    ctx: *mut c_void,
    _hold: Hold,
}

// SAFETY: a provider's table is the library's to have called from any
// thread, as a `Remote` is `Send + Sync`; the hold is an `Arc`.
unsafe impl Send for DescribedRemote {}
// SAFETY: as above.
unsafe impl Sync for DescribedRemote {}

impl core::fmt::Debug for DescribedRemote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DescribedRemote")
            .field("kind", &self.caller.kind().name())
            .finish()
    }
}

impl DescribedRemote {
    /// The provider's table for `caller`'s kind, checked as any door into
    /// a table checks it: present, its floor hash the description's, at
    /// least its floor in size. An object kind is never a provider's.
    pub fn new(view: &ProviderView, caller: Arc<Caller>) -> Result<DescribedRemote, KindMismatch> {
        let kind = caller.kind();
        if kind.object() {
            return Err(KindMismatch::ObjectKind);
        }
        let (table, size) = view
            .tables
            .iter()
            .find(|(k, t, _)| k == kind.name() && !t.is_null())
            .map(|&(_, t, s)| (t, s))
            .ok_or(KindMismatch::NoTable)?;
        if !(table as usize).is_multiple_of(align_of::<usize>()) {
            return Err(KindMismatch::Misaligned);
        }
        if size < kind.floor() {
            return Err(KindMismatch::BelowFloor {
                size,
                floor: kind.floor(),
            });
        }
        // SAFETY: a table the registry read under its checks, at least a
        // header long.
        let header = unsafe { table.cast::<KindHeader>().read_unaligned() };
        if header.floor_hash != kind.floor_hash() {
            return Err(KindMismatch::HashMismatch {
                expected: kind.floor_hash(),
                found: header.floor_hash,
            });
        }
        Ok(DescribedRemote {
            caller,
            table,
            size,
            ctx: view.ctx,
            _hold: view.hold.clone(),
        })
    }

    /// The kind this remote calls.
    pub fn kind(&self) -> &Arc<KindDescription> {
        self.caller.kind()
    }

    /// Whether the table fills slot `slot`: within its size and not null.
    /// A slot it leaves out answers `GUATIAO_ERR_UNSUPPORTED` when called,
    /// so a caller that has a default for it runs that instead.
    pub fn implements(&self, slot: usize) -> bool {
        let kind = self.caller.kind();
        if slot >= kind.slots().len() {
            return false;
        }
        // SAFETY: a table the registry read under its checks, at least a
        // header long (`new`).
        let header = unsafe { self.table.cast::<KindHeader>().read_unaligned() };
        let declared = match header.struct_size as usize {
            0 => self.size,
            own => own.min(self.size),
        };
        let word = kind.word_of(slot);
        if declared < size_of::<KindHeader>() + (word + 1) * size_of::<usize>() {
            return false;
        }
        // SAFETY: within the declared size, a pointer-sized slot.
        let f = unsafe {
            self.table
                .cast::<u8>()
                .add(size_of::<KindHeader>() + word * size_of::<usize>())
                .cast::<Option<unsafe extern "C" fn()>>()
                .read_unaligned()
        };
        f.is_some()
    }

    /// Calls slot `slot` (an index into the kind's slots).
    pub fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        // SAFETY: the table and `ctx` a registry offered, held loaded.
        unsafe {
            self.caller
                .call_raw(self.table, self.size, self.ctx, slot, args)
        }
    }

    /// Calls the slot named `name`.
    pub fn call_named(&self, name: &str, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        let (slot, _) = self.caller.kind().slot(name).ok_or_else(|| {
            ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "the kind has no such slot")
        })?;
        self.call(slot, args)
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
