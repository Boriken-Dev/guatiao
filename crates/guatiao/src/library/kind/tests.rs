// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The kind runtime end to end, through one hand-written kind.

use super::*;
use std::mem::offset_of;

// A kind written by hand with only these helpers: what the macro
// will emit, spelled out once.

trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<String, ProviderError>;
    /// An appended slot with a default body: an older table lacks it
    /// and the proxy runs the default.
    fn shout(&self) -> i64 {
        -1
    }
}

#[repr(C)]
struct GreeterVtable {
    header: KindHeader,
    greet: Option<unsafe extern "C" fn(*mut c_void, Str, *mut Text, *mut ProviderError) -> Status>,
    shout: Option<unsafe extern "C" fn(*mut c_void, *mut i64) -> Status>,
}

const GREET_END: usize = offset_of!(GreeterVtable, greet) + size_of::<usize>();
const SHOUT_END: usize = offset_of!(GreeterVtable, shout) + size_of::<usize>();
const HASH: u32 = fnv1a("greet(&self, name: &str) -> Result<String, ProviderError>");

unsafe extern "C" fn greet_shim<T: Greeter>(
    ctx: *mut c_void,
    name: Str,
    out: *mut Text,
    err: *mut ProviderError,
) -> Status {
    catch(|| {
        // SAFETY: the table was built for `T` and the descriptor's
        // `ctx` is a `&T`.
        let Some(this) = (unsafe { ctx_ref::<T>(ctx) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: the proxy passes a readable view.
        let name = match std::str::from_utf8(name.into()) {
            Ok(n) => n,
            Err(e) => return e.into(),
        };
        match this.greet(name) {
            // SAFETY: the proxy passes writable out-pointers.
            Ok(text) => unsafe { write_out(out, Text::new(&text)) },
            // SAFETY: as above.
            Err(e) => unsafe { write_err(err, e) },
        }
    })
}

unsafe extern "C" fn shout_shim<T: Greeter>(ctx: *mut c_void, out: *mut i64) -> Status {
    catch(|| {
        // SAFETY: as in `greet_shim`.
        let Some(this) = (unsafe { ctx_ref::<T>(ctx) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { write_out(out, this.shout()) }
    })
}

impl GreeterVtable {
    const fn of<T: Greeter>() -> GreeterVtable {
        GreeterVtable {
            header: KindHeader::new(size_of::<GreeterVtable>(), HASH),
            greet: Some(greet_shim::<T>),
            shout: Some(shout_shim::<T>),
        }
    }
}

impl Kind for dyn Greeter {
    const NAME: &'static str = "greeter";
    type Vtable = GreeterVtable;
    const FLOOR: usize = GREET_END;
    const FLOOR_HASH: u32 = HASH;
    const REQUIRED: &'static [(&'static str, usize)] = &[("greet", GREET_END)];

    fn as_dyn(remote: &Remote<Self>) -> &Self {
        remote
    }
    fn as_dyn_mut(remote: &mut Remote<Self>) -> &mut Self {
        remote
    }
    fn boxed(remote: Remote<Self>) -> Box<Self> {
        Box::new(remote)
    }
    fn shared(remote: Remote<Self>) -> Arc<Self> {
        Arc::new(remote)
    }
}

impl Greeter for Remote<dyn Greeter> {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        type Slot = unsafe extern "C" fn(*mut c_void, Str, *mut Text, *mut ProviderError) -> Status;
        // SAFETY: `greet` is the slot at that offset in the table
        // validation checked.
        let f: Slot = unsafe { self.slot(offset_of!(GreeterVtable, greet), GREET_END) }
            .expect("validated: a required slot is present and non-null");
        let mut out = Text::new("");
        let mut err = ProviderError::none();
        // SAFETY: the slot's own signature; the locals are writable.
        let status = unsafe { f(self.ctx(), Str::new(name), &mut out, &mut err) };
        if status == Status::GUATIAO_OK {
            Ok(text_ret(out)?.to_string())
        } else {
            Err(take_err(err, status))
        }
    }

    fn shout(&self) -> i64 {
        type Slot = unsafe extern "C" fn(*mut c_void, *mut i64) -> Status;
        // SAFETY: as `greet`; an absent slot is the default body.
        match unsafe { self.slot::<Slot>(offset_of!(GreeterVtable, shout), SHOUT_END) } {
            Some(f) => {
                let mut out = 0i64;
                // SAFETY: the slot's own signature.
                if unsafe { f(self.ctx(), &mut out) } == Status::GUATIAO_OK {
                    out
                } else {
                    -1
                }
            }
            None => -1,
        }
    }
}

struct Hello;
impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        Ok(format!("hello, {name}"))
    }
    fn shout(&self) -> i64 {
        11
    }
}

struct Panics;
impl Greeter for Panics {
    fn greet(&self, _name: &str) -> Result<String, ProviderError> {
        panic!("a provider bug");
    }
}

static HELLO_TABLE: GreeterVtable = GreeterVtable::of::<Hello>();
static PANICS_TABLE: GreeterVtable = GreeterVtable::of::<Panics>();
static HELLO: Hello = Hello;
static PANICS: Panics = Panics;

fn remote_of(
    table: &'static GreeterVtable,
    ctx: *const c_void,
    size: usize,
) -> Result<Remote<dyn Greeter>, KindMismatch> {
    // SAFETY: a table this test built for the kind, with the ctx it
    // was built for.
    unsafe {
        Remote::<dyn Greeter>::from_raw(
            table as *const GreeterVtable as *const c_void,
            size,
            ctx.cast_mut(),
        )
    }
}

/// A text or a map a shim wrote itself is checked before a proxy
/// hands it out, and a message that is not UTF-8 is dropped.
#[test]
fn what_a_shim_writes_itself_is_checked() {
    let alloc = Alloc::rust();
    let raw = |bytes: &[u8]| {
        let (p, l, c, a) = crate::value::types::Buffer::new_in(alloc, bytes)
            .unwrap()
            .into_raw_parts();
        // SAFETY: consistent storage; the bytes are under test.
        unsafe { Text::from_raw_parts(p, l, c, a) }
    };
    assert_eq!(&*text_ret(raw(b"ok")).unwrap(), "ok");
    let refused = text_ret(raw(b"\xff")).expect_err("not UTF-8");
    assert_eq!(refused.status, Status::from(ValueError::NotUtf8));

    let written = ProviderError {
        status: Status::GUATIAO_ERR_BAD_VALUE,
        message: raw(b"\xfe"),
    };
    let taken = take_err(written, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        taken.status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "the status stays"
    );
    assert_eq!(taken.message(), "", "the unreadable words go");

    let map: Map = [("k", 1)].into_iter().collect();
    assert!(map_ret(map).is_ok());
}

#[test]
fn a_call_round_trips_through_a_remote() {
    let r = remote_of(
        &HELLO_TABLE,
        &HELLO as *const Hello as *const c_void,
        size_of::<GreeterVtable>(),
    )
    .expect("a full table validates");
    assert_eq!(r.greet("ana").unwrap(), "hello, ana");
    assert_eq!(r.shout(), 11, "the appended slot is present and called");

    let e = r.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet", "the message crosses intact");

    let kept: Box<dyn Greeter> = <dyn Greeter as Kind>::boxed(r);
    assert_eq!(kept.greet("bo").unwrap(), "hello, bo");
    let shared: Arc<dyn Greeter> = <dyn Greeter as Kind>::shared(r);
    assert_eq!(shared.shout(), 11);
}

#[test]
fn a_panic_in_the_implementation_is_a_status_and_the_process_lives() {
    let r = remote_of(
        &PANICS_TABLE,
        &PANICS as *const Panics as *const c_void,
        size_of::<GreeterVtable>(),
    )
    .unwrap();
    let e = r.greet("x").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(e.message(), "", "a panic has no words the shim could write");
}

#[test]
fn a_truncated_table_runs_the_default_body_for_an_appended_slot() {
    let r = remote_of(
        &HELLO_TABLE,
        &HELLO as *const Hello as *const c_void,
        GREET_END,
    )
    .expect("the floor is enough");
    assert_eq!(r.greet("ana").unwrap(), "hello, ana");
    assert_eq!(
        r.shout(),
        -1,
        "the slot is past the table, so the default runs"
    );
    assert_eq!(r.size(), GREET_END);
}

#[test]
fn each_check_names_itself() {
    let ctx = &HELLO as *const Hello as *const c_void;
    assert_eq!(
        remote_of(&HELLO_TABLE, ctx, GREET_END - 1).unwrap_err(),
        KindMismatch::BelowFloor {
            size: GREET_END - 1,
            floor: GREET_END
        }
    );

    let mut forged = GreeterVtable::of::<Hello>();
    forged.header.floor_hash = HASH ^ 1;
    let forged: &'static GreeterVtable = Box::leak(Box::new(forged));
    assert_eq!(
        remote_of(forged, ctx, size_of::<GreeterVtable>()).unwrap_err(),
        KindMismatch::HashMismatch {
            expected: HASH,
            found: HASH ^ 1
        }
    );

    let mut nulled = GreeterVtable::of::<Hello>();
    nulled.greet = None;
    let nulled: &'static GreeterVtable = Box::leak(Box::new(nulled));
    assert_eq!(
        remote_of(nulled, ctx, size_of::<GreeterVtable>()).unwrap_err(),
        KindMismatch::NullRequiredSlot("greet")
    );

    // SAFETY: null is refused before anything is read.
    assert_eq!(
        unsafe { Remote::<dyn Greeter>::from_raw(std::ptr::null(), 64, std::ptr::null_mut()) }
            .unwrap_err(),
        KindMismatch::NoTable
    );
}

/// A header hash of zero is "unchecked" through `from_raw` alone; a
/// registry or host never offers such a table as a kind.
#[test]
fn a_zero_hash_is_accepted_only_at_the_raw_door() {
    let mut headerless = GreeterVtable::of::<Hello>();
    headerless.header.floor_hash = 0;
    let headerless: &'static GreeterVtable = Box::leak(Box::new(headerless));
    let ctx = &HELLO as *const Hello as *const c_void;
    assert!(remote_of(headerless, ctx, size_of::<GreeterVtable>()).is_ok());

    let view = ProviderView {
        kinds: vec!["greeter".to_string()],
        id: "x".to_string(),
        display_name: String::new(),
        config: None,
        vtable: std::ptr::null(),
        vtable_size: 0,
        ctx: ctx.cast_mut(),
        meta: None,
        version: None,
        available: None,
        raw: std::ptr::null(),
        create: None,
        destroy: None,
        tables: vec![(
            "greeter".to_string(),
            headerless as *const GreeterVtable as *const c_void,
            size_of::<GreeterVtable>(),
        )],
    };
    assert_eq!(
        Remote::<dyn Greeter>::from_view(&view).unwrap_err(),
        KindMismatch::HashMismatch {
            expected: HASH,
            found: 0
        }
    );

    // And the legacy `vtable` is never a typed table: no per-kind
    // entry means no table, whatever `vtable` holds.
    let legacy = ProviderView {
        tables: Vec::new(),
        vtable: &HELLO_TABLE as *const GreeterVtable as *const c_void,
        vtable_size: size_of::<GreeterVtable>(),
        ..view.clone()
    };
    assert_eq!(
        Remote::<dyn Greeter>::from_view(&legacy).unwrap_err(),
        KindMismatch::NoTable
    );
}

/// Three providers claiming one kind: one available, one refusing,
/// one with a short table. `offers` lists the first two in registry
/// order; `mismatches` holds the third.
#[cfg(feature = "load")]
#[test]
fn a_registry_offers_every_valid_provider_and_reports_the_rest() {
    use super::super::raw::{LibraryView, Opened};
    use super::super::registry::Registry;
    use std::path::Path;

    unsafe extern "C" fn refuses(_ctx: *mut c_void, reason: *mut Str) -> bool {
        if !reason.is_null() {
            // SAFETY: the test passes writable storage.
            unsafe { reason.write(Str::new("not today")) };
        }
        false
    }

    let ctx = (&HELLO as *const Hello).cast_mut().cast::<c_void>();
    let table = &HELLO_TABLE as *const GreeterVtable as *const c_void;
    let provider = |id: &str, size: usize, available| ProviderView {
        kinds: vec!["greeter".to_string()],
        id: id.to_string(),
        display_name: String::new(),
        config: None,
        vtable: std::ptr::null(),
        vtable_size: 0,
        ctx,
        meta: None,
        version: None,
        available,
        raw: std::ptr::null(),
        create: None,
        destroy: None,
        tables: vec![("greeter".to_string(), table, size)],
    };

    let mut registry = Registry::new("kind-tests", "1.0");
    registry
        .absorb(
            Path::new("kinds.so"),
            Opened::Loaded(
                LibraryView {
                    id: "kinds".to_string(),
                    version: "1.0.0".to_string(),
                    meta: None,
                    unload: None,
                    providers: vec![
                        provider("kinds_short", GREET_END - 1, None),
                        provider("kinds_refuses", size_of::<GreeterVtable>(), Some(refuses)),
                        provider("kinds_ok", size_of::<GreeterVtable>(), None),
                    ],
                },
                super::super::raw::Origin::Linked,
            ),
        )
        .unwrap();

    let offers: Vec<Offer<dyn Greeter>> = registry.offers::<dyn Greeter>().collect();
    let ids: Vec<&str> = offers.iter().map(Offer::id).collect();
    assert_eq!(
        ids,
        ["kinds_ok", "kinds_refuses"],
        "registry order, both offered"
    );
    assert_eq!(offers[0].available(), Ok(()));
    assert_eq!(offers[1].available(), Err("not today"));
    assert_eq!(offers[0].greet("ana").unwrap(), "hello, ana");
    assert_eq!(offers[0].library(), Some("kinds"));
    assert_eq!(offers[0].key(), Some("kinds_ok"));

    let mismatches: Vec<(&str, KindMismatch)> = registry
        .mismatches::<dyn Greeter>()
        .map(|(p, why)| (p.id(), why))
        .collect();
    assert_eq!(
        mismatches,
        [(
            "kinds_short",
            KindMismatch::BelowFloor {
                size: GREET_END - 1,
                floor: GREET_END
            }
        )]
    );

    let one = registry
        .offer::<dyn Greeter>("kinds_ok")
        .expect("filed under that key")
        .expect("valid");
    assert_eq!(one.greet("bo").unwrap(), "hello, bo");
    assert!(registry.offer::<dyn Greeter>("nobody").is_none());
    let kept: Box<dyn Greeter> = one.boxed();
    assert_eq!(kept.shout(), 11);
}
