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
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{OnceLock, PoisonError, RwLock};

use guatiao::library::{Host, KindTables, Kinds, LibraryInfo, ProviderInfo, Providers};
use guatiao::schema::{FieldBuilder, FormBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::str_or;
use guatiao::value::status::Status;
use guatiao::value::types::{Map, MaybeNull, Str, Value};

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

/// A `Str` holds a `*const u8`, so an array of them is not `Sync` and
/// cannot be a `static` without saying why either.
struct Names<const N: usize>([Str; N]);

// SAFETY: a compile-time constant that is never written, whose every
// pointer addresses a string literal in this library's own image — which
// is never unloaded, so the borrow outlives every reader.
unsafe impl<const N: usize> Sync for Names<N> {}

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
/// The almanac cannot run here, and says why.
///
/// A real provider would be asking about a device, a codec or an optional
/// dependency. This one refuses unconditionally, because what the test
/// needs to pin is the MECHANISM: a refusal crosses with its reason
/// intact, and a host can tell it apart from nothing claiming the kind.
///
/// The reason is a literal in this library's image, which is never
/// unloaded — the "immortal, never a shared buffer" half of the contract.
///
/// # Safety
///
/// Called through the descriptor with the `ctx` this library declared,
/// and `reason` addresses writable storage for one `Str`.
unsafe extern "C" fn almanac_available(_ctx: *mut c_void, reason: *mut Str) -> bool {
    if !reason.is_null() {
        // SAFETY: the caller's contract says it is writable.
        unsafe {
            reason.write(Str::borrowed(
                "this almanac needs a calendar this host has not set",
            ))
        };
    }
    false
}

/// The sundial claims `timekeeper` and cannot run here.
///
/// # Safety
///
/// As [`almanac_available`].
unsafe extern "C" fn sundial_available(_ctx: *mut c_void, reason: *mut Str) -> bool {
    if !reason.is_null() {
        // SAFETY: the caller's contract says it is writable.
        unsafe { reason.write(Str::borrowed("the sun is not up")) };
    }
    false
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

// --- a provider that reaches another through the host -------------------

/// The function table of the `echo` kind: one slot that greets by name,
/// by finding a `greeter` through the host and calling it.
#[repr(C)]
pub struct EchoVtable {
    /// `sizeof(EchoVtable)` as the library compiled it. Always first.
    pub struct_size: u32,
    /// Writes the greeter's answer for `name` to `out`, an owned map the
    /// caller then owns.
    pub echo: Option<unsafe extern "C" fn(ctx: *mut c_void, name: Str, out: *mut Value) -> Status>,
}

/// The host this library was loaded by, kept from the latest entry call.
/// A `Host` is `Copy + Send + Sync + 'static`, which is what makes keeping
/// it possible with no `unsafe`. A process has one host; a test suite that
/// loads this library from several registries gets the most recent one.
static HOST: RwLock<Option<Host>> = RwLock::new(None);

/// Finds `hello_library_greeter` through the host's services and greets
/// through it. **Looked up on every call, never at `describe`**: what the
/// host holds changes as it loads, and this provider may be asked before
/// the greeter's library was — even though here it is the same library.
///
/// # Safety
///
/// `name` is readable for the call and `out` points at a writable node
/// the caller will free.
unsafe extern "C" fn echo(_ctx: *mut c_void, name: Str, out: *mut Value) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    let Some(host) = *HOST.read().unwrap_or_else(PoisonError::into_inner) else {
        return Status::GUATIAO_ERR_INTERNAL;
    };
    // Every greeter the host holds, in the host's order; this one wants a
    // particular implementation, so it picks by id rather than taking the
    // head.
    let greeters = match host.list("greeter") {
        Ok(found) => found,
        Err(status) => return status,
    };
    let Some(greeter) = greeters
        .into_iter()
        .filter_map(ProviderInfo::view)
        .find(|p| p.id == "hello_library_greeter")
    else {
        return Status::GUATIAO_ERR_NOT_FOUND;
    };
    // SAFETY: `greeter` claims the `greeter` kind, whose table this crate
    // itself declares; `vtable_as` refuses a table shorter than the type.
    let Some(table) = (unsafe { greeter.vtable_as::<GreeterVtable>() }) else {
        return Status::GUATIAO_ERR_WRONG_KIND;
    };
    let Some(greet) = table.greet else {
        return Status::GUATIAO_ERR_WRONG_KIND;
    };

    // SAFETY: the caller's contract says `name` is readable for the call;
    // an empty view may carry any pointer and is never dereferenced.
    let bytes: &[u8] = if name.len == 0 {
        &[]
    } else if name.ptr.is_null() {
        return Status::GUATIAO_ERR_NULL;
    } else {
        unsafe { std::slice::from_raw_parts(name.ptr, name.len) }
    };
    let Ok(name) = std::str::from_utf8(bytes) else {
        return Status::GUATIAO_ERR_BAD_VALUE;
    };
    let alloc = library_alloc();
    let mut config = Value::map_in(alloc);
    let text = match Value::string_in(alloc, name) {
        Ok(t) => t,
        Err(e) => return Status::from(e),
    };
    if let Err(e) = config.set("name", text) {
        return Status::from(e);
    }
    let mut answer = Value::absent();
    // SAFETY: the greeter's contract, as declared on `greet` above; the
    // config is a well-formed value and `answer` is a writable local.
    let status = unsafe { greet(greeter.ctx, &config, &mut answer) };
    if status != Status::GUATIAO_OK {
        return status;
    }
    // SAFETY: `out` is writable, and whatever it held is the caller's.
    unsafe { out.write(answer) };
    Status::GUATIAO_OK
}

static ECHO: EchoVtable = EchoVtable {
    struct_size: size_of::<EchoVtable>() as u32,
    echo: Some(echo),
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
    /// The metadata this library declares. Boxed for the same reason the
    /// schema is: the descriptor points at it, so it needs an address
    /// that does not move when this struct does.
    #[allow(dead_code)]
    meta: Box<Map>,
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
///
/// Declining is an answer: a host speaking another envelope version gets
/// null, which its loader reports as a skip rather than a failure. The
/// host is kept, because the echo provider reaches the greeter through it
/// on every call.
fn describe(host: Host) -> Option<&'static LibraryInfo> {
    if host.abi_version() != guatiao::library::ABI_VERSION {
        return None;
    }
    *HOST.write().unwrap_or_else(PoisonError::into_inner) = Some(host);
    let registered = REGISTERED.get_or_init(|| {
        let alloc = library_alloc();

        // The configuration a greeter takes, declared as an ordinary
        // schema value. A host reads it with `SchemaRef` and checks a
        // configuration against it with `validate_map`, having never
        // heard of this library.
        let schema = Box::new(
            SchemaBuilder::new_in(alloc)
                .field(
                    FieldBuilder::new_in(alloc, "name", KindBuilder::string_in(alloc))
                        .label("Name")
                        .help("Who to greet.")
                        .required(),
                )
                .finish()
                .expect("a schema this small does not exhaust an allocator"),
        );

        // What the envelope did not think of. A host that does not know
        // these keys skips them — which is the point of the slot.
        let mut declared = Map::new_in(alloc);
        declared
            .set("built-with", env!("CARGO_PKG_NAME"))
            .expect("a two-key map does not exhaust an allocator");
        declared.set("greeting-language", "en").expect("as above");
        let meta = Box::new(declared);

        // One provider, two kinds. It greets, and it writes what it
        // greeted — one implementation with one identity, which is why it
        // is one provider answering to both rather than two registrations
        // a host would have to know are the same thing.
        // Each provider serves its own kinds AND one they share, so a host
        // ranking several implementations of one kind has a real case to
        // work on rather than a contrived one.
        static GREETER_KINDS: Names<3> = Names([
            Str::borrowed("greeter"),
            Str::borrowed("writer"),
            Str::borrowed("everything"),
        ]);
        static SUNDIAL_KINDS: Names<2> =
            Names([Str::borrowed("timekeeper"), Str::borrowed("everything")]);
        static ALMANAC_KINDS: Names<1> = Names([Str::borrowed("everything")]);
        static ECHO_KINDS: Names<1> = Names([Str::borrowed("echo")]);

        let providers = vec![
            ProviderInfo {
                struct_size: size_of::<ProviderInfo>() as u32,
                vtable_size: size_of::<GreeterVtable>() as u32,
                kinds: Kinds::new(&GREETER_KINDS.0),
                // `{library id}_{name}`, the convention that makes an id
                // unique without a central register.
                id: Str::borrowed("hello_library_greeter"),
                display_name: Str::borrowed("Hello"),
                config: &*schema as *const Value,
                vtable: &GREETER as *const GreeterVtable as *const c_void,
                ctx: std::ptr::null_mut(),
                meta: MaybeNull::null(),
                // Empty: this provider ships in this library and moves
                // with it, so its version is the library's.
                version: Str::borrowed(""),
                // No slot: this greeter is available whenever it loaded,
                // which is the common case and the right default.
                available: None,
                tables: KindTables::empty(),
            },
            ProviderInfo {
                struct_size: size_of::<ProviderInfo>() as u32,
                vtable_size: 0,
                kinds: Kinds::new(&ALMANAC_KINDS.0),
                id: Str::borrowed("hello_library_almanac"),
                display_name: Str::borrowed("Almanac"),
                config: std::ptr::null(),
                vtable: std::ptr::null(),
                ctx: std::ptr::null_mut(),
                meta: MaybeNull::null(),
                // Its own, because its contract froze while the library
                // around it went on. This is the case the field exists for.
                version: Str::borrowed("1.0.0"),
                // And it refuses, with a reason a host can show.
                available: Some(almanac_available),
                tables: KindTables::empty(),
            },
            // Claims a kind AND cannot run here, which is the case a host
            // must tell apart from nobody claiming the kind at all: the
            // remedy is to fix this one, not to install another.
            ProviderInfo {
                struct_size: size_of::<ProviderInfo>() as u32,
                vtable_size: 0,
                kinds: Kinds::new(&SUNDIAL_KINDS.0),
                id: Str::borrowed("hello_library_sundial"),
                display_name: Str::borrowed("Sundial"),
                config: std::ptr::null(),
                vtable: std::ptr::null(),
                ctx: std::ptr::null_mut(),
                meta: MaybeNull::null(),
                version: Str::borrowed(""),
                available: Some(sundial_available),
                tables: KindTables::empty(),
            },
            // Reaches the greeter through the host's services, which is
            // the one thing a library could not do before it kept a
            // `Host`.
            ProviderInfo {
                struct_size: size_of::<ProviderInfo>() as u32,
                vtable_size: size_of::<EchoVtable>() as u32,
                kinds: Kinds::new(&ECHO_KINDS.0),
                id: Str::borrowed("hello_library_echo"),
                display_name: Str::borrowed("Echo"),
                config: std::ptr::null(),
                vtable: &ECHO as *const EchoVtable as *const c_void,
                ctx: std::ptr::null_mut(),
                meta: MaybeNull::null(),
                version: Str::borrowed(""),
                available: None,
                tables: KindTables::empty(),
            },
        ];

        let desc = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: guatiao::library::ABI_VERSION,
            id: Str::borrowed("hello_library"),
            version: Str::borrowed(env!("CARGO_PKG_VERSION")),
            providers: Providers {
                ptr: providers.as_ptr(),
                len: providers.len(),
                // The size THIS build lays the array out at, which is what
                // lets a newer host walk it correctly.
                stride: size_of::<ProviderInfo>(),
            },
            // SAFETY-adjacent: the box outlives the process, because
            // `Registered` is held in a `OnceLock` that is never cleared.
            meta: MaybeNull::of(unsafe { &*(&*meta as *const Map) }),
        };

        Registered {
            schema,
            meta,
            providers,
            desc,
        }
    });
    Some(&registered.desc)
}

guatiao::guatiao_library!(describe);

// --- a described type, reachable from any language ----------------------
//
// Nothing here is a provider's configuration. These are ordinary types
// this library can describe, exported so a caller with no Rust can ask
// what they look like — which is what a schema is for, config being only
// one of the things it describes.

/// What a greeting comes back as.
#[derive(guatiao::Schema)]
#[allow(dead_code)]
struct Greeting {
    /// The text to show.
    greeting: String,
    /// Who asked.
    seen_by: Option<String>,
}

/// Something this library knows about that is not configuration at all.
#[derive(guatiao::Schema)]
#[allow(dead_code)]
struct Ledger {
    /// How many greetings have been handed out.
    count: i64,
}

// The default: the calling crate's name is the prefix.
guatiao::export_schema!(Greeting, "greeting");
// Two in one crate, which is the case that would collide if the macro
// named the Rust function rather than only the symbol.
guatiao::export_schema!(Ledger, "ledger");
// And the version in the name, so two builds of this library can sit in
// one process. `minor` rather than `major` because this is a 0.x crate —
// below 1.0 the major is always 0 and separates nothing.
guatiao::export_schema!(Ledger, "ledger", version = minor);
