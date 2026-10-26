// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! libffi as a guatiao [`Mechanism`].
//!
//! A kind table built at run time from a kind's description
//! ([`ServedTable`]), and a call through any table of a described kind
//! ([`Caller`]), need a function pointer for a C signature known only
//! while the program runs, and a call through one. Rust can do neither;
//! [`Libffi`] does both, with libffi's closures and `ffi_call`. Everything
//! else about such tables is in `guatiao`.
//!
//! ```ignore
//! use guatiao_libffi::Libffi;
//! let table = ServedTable::new(kind, |_| true, serve, &Libffi)?;
//! let caller = Caller::new(kind, &Libffi)?;
//! ```
//!
//! libffi builds from C source; it does not support `wasm32`.
//!
//! [`ServedTable`]: guatiao::library::kind::ServedTable
//! [`Caller`]: guatiao::library::kind::Caller

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use guatiao::library::kind::{
    CReturn, CSignature, CType, Closure, Mechanism, Prepared, SlotHandler, TableError,
};
use libffi::middle::{Cif, Type};
use libffi::raw as ffi;

/// libffi as a [`Mechanism`]: a signature becomes a call interface, a
/// closure a libffi closure, a call `ffi_call`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Libffi;

/// The libffi type of one argument.
fn type_of(c: &CType) -> Result<Type, TableError> {
    Ok(match c {
        CType::U8 => Type::u8(),
        CType::I8 => Type::i8(),
        CType::U16 => Type::u16(),
        CType::I16 => Type::i16(),
        CType::U32 => Type::u32(),
        CType::I32 => Type::i32(),
        CType::U64 => Type::u64(),
        CType::I64 => Type::i64(),
        CType::Usize => Type::usize(),
        CType::Isize => Type::isize(),
        CType::F32 => Type::f32(),
        CType::F64 => Type::f64(),
        CType::Pointer => Type::pointer(),
        CType::Struct(fields) => {
            Type::structure(fields.iter().map(type_of).collect::<Result<Vec<_>, _>>()?)
        }
        other => {
            return Err(TableError::Mechanism(format!(
                "libffi has no type here for {other:?}"
            )));
        }
    })
}

// SAFETY: every closure `Signature::closure` makes is a libffi closure
// prepared with the signature's own call interface, entering `on_call`,
// which hands the handler libffi's argument array (one address per
// argument, in order); `call` is `ffi_call` with the same interface.
unsafe impl Mechanism for Libffi {
    fn prepare(&self, signature: &CSignature) -> Result<Arc<dyn Prepared>, TableError> {
        let args = signature
            .args()
            .iter()
            .map(type_of)
            .collect::<Result<Vec<_>, _>>()?;
        // Every return kind it knows; a newer one is refused, never guessed.
        let (returns, answers) = match signature.returns() {
            CReturn::Status => (Type::u32(), true),
            // C `bool` is one byte, 0 or 1.
            CReturn::Bool => (Type::u8(), true),
            CReturn::Nothing => (Type::void(), false),
            other => {
                return Err(TableError::Mechanism(format!(
                    "libffi has no return type here for {other:?}"
                )));
            }
        };
        Ok(Arc::new(Signature {
            cif: Cif::new(args, returns),
            answers,
        }))
    }
}

/// A call interface libffi prepared, with the types it points into.
struct Signature {
    cif: Cif,
    /// Whether it returns an integer (a status or a `bool`).
    answers: bool,
}

// SAFETY: prepared once and only read after, by libffi, from any thread.
unsafe impl Send for Signature {}
// SAFETY: as above.
unsafe impl Sync for Signature {}

/// What one closure is prepared with.
struct Data {
    handler: SlotHandler,
    answers: bool,
}

/// One libffi closure, with what it reads: freed before them.
struct Made {
    raw: *mut c_void,
    _data: Box<Data>,
    _signature: Arc<Signature>,
}

// SAFETY: the closure is freed once, by whichever thread drops it last;
// what it reads is `Send + Sync`.
unsafe impl Send for Made {}
// SAFETY: as above; nothing reads through `raw` but libffi's free.
unsafe impl Sync for Made {}

impl Drop for Made {
    fn drop(&mut self) {
        // SAFETY: from `ffi_closure_alloc`, freed once; the `Closure` that
        // owned this is gone, and with it every caller's right to call it.
        unsafe { ffi::ffi_closure_free(self.raw) };
    }
}

// SAFETY: as `Libffi`.
unsafe impl Prepared for Signature {
    fn closure(self: Arc<Self>, handler: SlotHandler) -> Result<Closure, TableError> {
        let answers = self.answers;
        let mut code: *mut c_void = core::ptr::null_mut();
        // SAFETY: allocates a closure and its code address, or answers
        // null.
        let raw = unsafe { ffi::ffi_closure_alloc(size_of::<ffi::ffi_closure>(), &mut code) };
        if raw.is_null() || code.is_null() {
            return Err(TableError::Mechanism(
                "libffi could not allocate a closure".to_string(),
            ));
        }
        let made = Made {
            raw,
            _data: Box::new(Data { handler, answers }),
            _signature: self,
        };
        // SAFETY: a closure just allocated; the interface and the data are
        // boxed and kept by `made`, which frees the closure before them.
        let status = unsafe {
            ffi::ffi_prep_closure_loc(
                raw.cast::<ffi::ffi_closure>(),
                made._signature.cif.as_raw_ptr(),
                Some(on_call),
                (&*made._data as *const Data).cast_mut().cast::<c_void>(),
                code,
            )
        };
        if status != ffi::ffi_status_FFI_OK {
            return Err(TableError::Mechanism(format!(
                "libffi refused the closure ({status})"
            )));
        }
        // SAFETY: libffi's code address for the closure, non-null, callable
        // with the interface's signature once prepared.
        let code = unsafe { core::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(code) };
        // SAFETY: the code stays callable while `made` lives.
        Ok(unsafe { Closure::new(code, Box::new(made)) })
    }

    unsafe fn call(&self, f: unsafe extern "C" fn(), args: &[*mut c_void]) -> u32 {
        // libffi writes an integer return widened to `ffi_arg`, a word.
        let mut ret: usize = 0;
        // SAFETY: the caller's contract; libffi reads the argument array
        // and never writes it.
        unsafe {
            ffi::ffi_call(
                self.cif.as_raw_ptr(),
                Some(f),
                (&mut ret as *mut usize).cast::<c_void>(),
                args.as_ptr().cast_mut(),
            );
        }
        ret as u32
    }
}

/// Every closure's entry: the data names the slot.
unsafe extern "C" fn on_call(
    _cif: *mut ffi::ffi_cif,
    ret: *mut c_void,
    args: *mut *mut c_void,
    data: *mut c_void,
) {
    // SAFETY: the data this closure was prepared with, alive while the
    // closure is.
    let data = unsafe { &*data.cast::<Data>() };
    // SAFETY: libffi's argument array for the slot's signature.
    let status = unsafe { data.handler.call(args.cast_const()) };
    if data.answers {
        // SAFETY: libffi's return buffer, at least `ffi_arg` wide; an
        // integer answer (a status, or a `bool` as 0 or 1) is written
        // widened to it.
        unsafe { ret.cast::<usize>().write(status as usize) };
    }
}
