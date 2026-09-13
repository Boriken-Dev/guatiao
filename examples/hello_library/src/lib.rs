// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library, in about a hundred lines of substance.
//!
//! It offers one provider of kind `greeter`, declares the configuration
//! that provider takes as an ordinary schema, and answers `greet` with a
//! map it built through **its own allocator** — which the host then adds
//! a key to and frees, without ever naming that allocator.
//!
//! That last sentence is the whole point of the design, and
//! `tests/library_load.rs` in the `guatiao` crate is where it is checked
//! rather than asserted.
//!
//! # What a library author actually writes
//!
//! A `describe` function and one macro invocation. The entry symbol, the
//! panic discipline and the null-on-decline rule come from the macro, so
//! a misspelled symbol name — a library that loads and offers nothing —
//! is not a mistake that can be made here.

#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

use guatiao::library::{HostInfo, LibraryInfo, ProviderInfo, Providers};
use guatiao::schema::{KindBuilder, OptionBuilder, SchemaBuilder};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::str_or;
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};

// --- the kind ----------------------------------------------------------

/// The function table every `greeter` provider supplies.
///
/// **The envelope does not define this; the kind does.** A host that
/// knows what a greeter is compiles against this declaration, checks the
/// size the library compiled it at against its own floor, and calls
/// through it.
///
/// Slots are **appended, never changed**. `struct_size` can say that a
/// slot is absent; it cannot say that a slot's arguments changed, so a
/// changed signature is silent memory corruption in every already-built
/// consumer. `outstanding` below is an appended slot, and it is here to
/// make that arrangement real rather than described.
#[repr(C)]
pub struct GreeterVtable {
    /// `sizeof(GreeterVtable)` as the library compiled it. Always first.
    pub struct_size: u32,
    /// Builds a greeting from a configuration, as an owned map the caller
    /// then owns.
    ///
    /// Spelled out inline rather than through a type alias: cbindgen
    /// renders an aliased function-pointer field as an opaque struct used
    /// by value, which is an incomplete type that compiles nowhere.
    pub greet: Option<
        unsafe extern "C" fn(ctx: *mut c_void, config: *const Value, out: *mut Value) -> Status,
    >,
    /// How many blocks this library's allocator has outstanding.
    ///
    /// **Appended after `greet`**, which is what makes this table a
    /// worked example of the rule rather than a statement of it: a host
    /// built against the one-slot version reads a smaller `struct_size`,
    /// never looks at this field, and works.
    pub outstanding: Option<unsafe extern "C" fn(ctx: *mut c_void) -> i64>,
}

impl GreeterVtable {
    /// The smallest `struct_size` a usable greeter can declare: the
    /// original slot, and nothing after it. **Frozen.** A floor that
    /// tracked the newest slot would refuse every library built before
    /// that slot existed.
    pub const fn floor() -> usize {
        std::mem::offset_of!(GreeterVtable, greet) + size_of::<usize>()
    }

    /// Where the appended slot ends, for the guard that reads it.
    pub const fn outstanding_end() -> usize {
        std::mem::offset_of!(GreeterVtable, outstanding) + size_of::<usize>()
    }
}

// --- this library's own allocator ---------------------------------------

/// Blocks this library has handed out and not taken back.
///
/// A library may hold whatever state it likes; the rule against
/// process-global state binds `guatiao` itself, so that a host and a
/// library can each link their own copy of it with nothing to disagree
/// about.
static OUTSTANDING: AtomicI64 = AtomicI64::new(0);

unsafe extern "C" fn counted_alloc(_ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    OUTSTANDING.fetch_add(1, Ordering::Relaxed);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request to the allocator this one
    // wraps.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counted_free(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    OUTSTANDING.fetch_sub(1, Ordering::Relaxed);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout it was
    // allocated with.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

/// This library's allocator, as a constant.
///
/// A `static` rather than a local, because **every tree built through it
/// records this address** and calls back into it to grow and to free. It
/// therefore has to outlive every such tree, which for a library that is
/// never unloaded means the life of the process.
static LIBRARY_ALLOC: VTable = VTable(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: std::ptr::null_mut(),
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

/// An `Allocator` holds a `*mut c_void`, so it is not `Sync` and cannot
/// be a `static` without saying why.
struct VTable(Allocator);

// SAFETY: the value is a compile-time constant that is never written, its
// `ctx` is null so nothing is shared through it, and the two functions it
// names are thread-safe.
unsafe impl Sync for VTable {}

fn library_alloc() -> Alloc {
    // SAFETY: `LIBRARY_ALLOC.0` is a fully initialised constant that lives
    // for the whole process and declares its own size.
    unsafe { Alloc::from_raw(&LIBRARY_ALLOC.0) }.expect("the constant vtable is complete")
}

// --- the provider ------------------------------------------------------

/// Builds `{"greeting": "hello, <name>"}` through this library's own
/// allocator.
///
/// # Safety
///
/// `config` is null or a well-formed value, and `out` points at a writable
/// node the caller will free.
unsafe extern "C" fn greet(_ctx: *mut c_void, config: *const Value, out: *mut Value) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    // SAFETY: the caller's side of the contract is that `config` is null
    // or addresses a well-formed value.
    let name = match unsafe { config.as_ref() } {
        Some(c) => str_or(c.get("name"), "world"),
        None => "world",
    };

    let alloc = library_alloc();
    let mut answer = Value::map_in(alloc);
    let text = match Value::string_in(alloc, &format!("hello, {name}")) {
        Ok(t) => t,
        Err(e) => return Status::from(e),
    };
    if let Err(e) = answer.set("greeting", text) {
        return Status::from(e);
    }

    // Handed over, so the drop must not run: the caller owns this tree
    // now and will free it through the allocator recorded inside it.
    // SAFETY: `out` is writable, and whatever it held is the caller's to
    // have dealt with.
    unsafe { out.write(answer) };
    Status::GUATIAO_OK
}

/// # Safety
///
/// Called through the vtable with the `ctx` this library declared.
unsafe extern "C" fn outstanding(_ctx: *mut c_void) -> i64 {
    OUTSTANDING.load(Ordering::Relaxed)
}

static GREETER: GreeterVtable = GreeterVtable {
    struct_size: size_of::<GreeterVtable>() as u32,
    greet: Some(greet),
    outstanding: Some(outstanding),
};

// --- the descriptor ----------------------------------------------------

/// Everything the entry point hands back, kept alive for the life of the
/// process.
///
/// The `Box` and the `Vec` are not decoration: the descriptor points at
/// the schema value and at the provider array, so both need addresses
/// that do not move when this struct is moved into the `OnceLock`.
struct Registered {
    #[allow(dead_code)]
    schema: Box<Value>,
    #[allow(dead_code)]
    providers: Vec<ProviderInfo>,
    desc: LibraryInfo,
}

// SAFETY: built once inside `OnceLock::get_or_init`, never written again,
// and every pointer in it addresses something this struct owns and keeps.
unsafe impl Sync for Registered {}
// SAFETY: as above.
unsafe impl Send for Registered {}

static REGISTERED: OnceLock<Registered> = OnceLock::new();

/// What this library offers. The one function an author writes.
fn describe(_host: &HostInfo) -> Option<&'static LibraryInfo> {
    let registered = REGISTERED.get_or_init(|| {
        let alloc = library_alloc();

        // The configuration a greeter takes, declared as an ordinary
        // schema value. A host reads it with `SchemaRef` and checks a
        // configuration against it with `validate_map`, having never
        // heard of this library.
        let schema = Box::new(
            SchemaBuilder::new(alloc)
                .option(
                    OptionBuilder::new(alloc, "name", KindBuilder::string(alloc))
                        .label("Name")
                        .help("Who to greet.")
                        .required(),
                )
                .finish()
                .expect("a schema this small does not exhaust an allocator"),
        );

        let providers = vec![ProviderInfo {
            struct_size: size_of::<ProviderInfo>() as u32,
            vtable_size: size_of::<GreeterVtable>() as u32,
            kind: Str::borrowed("greeter"),
            id: Str::borrowed("hello"),
            display_name: Str::borrowed("Hello"),
            config: &*schema as *const Value,
            vtable: &GREETER as *const GreeterVtable as *const c_void,
            ctx: std::ptr::null_mut(),
        }];

        let desc = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: guatiao::library::ABI_VERSION,
            id: Str::borrowed("hello_library"),
            version: Str::borrowed(env!("CARGO_PKG_VERSION")),
            providers: Providers {
                ptr: providers.as_ptr(),
                len: providers.len(),
            },
        };

        Registered {
            schema,
            providers,
            desc,
        }
    });
    Some(&registered.desc)
}

guatiao::guatiao_library!(describe);
