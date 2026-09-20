// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A kind declared with `#[guatiao::kind]`, implemented, tabled with
//! `of::<Impl>()`, and called through a `Remote` — in one process, with
//! no `unsafe` in this file beyond the one raw door.

#![cfg(feature = "provider")]
// `shapes` takes every crossing shape at once, which is the point.
#![allow(clippy::too_many_arguments)]

use guatiao::value::convert::TryAsRef;
use std::ffi::c_void;

use guatiao::library::{Kind, KindMismatch, ProviderError, Remote};
use guatiao::{FromValue, Map, Schema, Status, ToValue, Value};

/// Something that crosses by value, through the derives.
#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
pub struct Greeting {
    text: String,
    count: i64,
}

#[guatiao::kind]
pub trait Greeter: Send + Sync {
    /// The one required method.
    fn greet(&self, name: &str) -> Result<String, ProviderError>;
    /// Every crossing shape at once.
    fn shapes(
        &self,
        flag: bool,
        n: u32,
        f: f64,
        bytes: &[u8],
        config: &Value,
        maybe: Option<&Value>,
        map: &Map,
        greeting: Greeting,
    ) -> Result<Greeting, ProviderError>;
    /// A map comes back owned.
    fn describe(&self, name: &str) -> Result<Map, ProviderError>;
    /// Cannot fail, and answers a scalar.
    fn count(&self) -> i64;
    /// Appended: a default body an older table lacks.
    fn shout(&self, name: &str) -> String {
        format!("{}!", self.greet(name).unwrap_or_default().to_uppercase())
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

    fn shapes(
        &self,
        flag: bool,
        n: u32,
        f: f64,
        bytes: &[u8],
        config: &Value,
        maybe: Option<&Value>,
        map: &Map,
        greeting: Greeting,
    ) -> Result<Greeting, ProviderError> {
        let port: i64 = TryAsRef::<Map>::try_as_ref(config)
            .and_then(|m| m.get("port"))
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(0);
        Ok(Greeting {
            text: format!(
                "{flag} {n} {f} {} {port} {} {}",
                bytes.len(),
                maybe.map_or("none", |_| "some"),
                map.len()
            ),
            count: greeting.count + 1,
        })
    }

    fn describe(&self, name: &str) -> Result<Map, ProviderError> {
        let mut map = Map::new();
        map.set("name", name)?;
        Ok(map)
    }

    fn count(&self) -> i64 {
        7
    }
}

struct Panics;

impl Greeter for Panics {
    fn greet(&self, _name: &str) -> Result<String, ProviderError> {
        panic!("a provider bug");
    }
    fn shapes(
        &self,
        _: bool,
        _: u32,
        _: f64,
        _: &[u8],
        _: &Value,
        _: Option<&Value>,
        _: &Map,
        _: Greeting,
    ) -> Result<Greeting, ProviderError> {
        unreachable!()
    }
    fn describe(&self, _: &str) -> Result<Map, ProviderError> {
        unreachable!()
    }
    fn count(&self) -> i64 {
        unreachable!()
    }
}

static HELLO_TABLE: GreeterVtable = GreeterVtable::of::<Hello>();
static PANICS_TABLE: GreeterVtable = GreeterVtable::of::<Panics>();
static HELLO: Hello = Hello;
static PANICS: Panics = Panics;

fn remote(
    table: &'static GreeterVtable,
    ctx: *const c_void,
    size: usize,
) -> Result<Remote<dyn Greeter>, KindMismatch> {
    // SAFETY: a table `of::<T>()` built for this kind, with the `&T` it
    // was built for, both `'static`.
    unsafe {
        Remote::<dyn Greeter>::from_raw(
            table as *const GreeterVtable as *const c_void,
            size,
            ctx.cast_mut(),
        )
    }
}

fn hello() -> Remote<dyn Greeter> {
    remote(
        &HELLO_TABLE,
        &HELLO as *const Hello as *const c_void,
        size_of::<GreeterVtable>(),
    )
    .expect("a full table validates")
}

#[test]
fn the_kind_declares_itself() {
    assert_eq!(<dyn Greeter as Kind>::NAME, "greeter");
    assert_eq!(<dyn Greeter as Kind>::FLOOR, GreeterVtable::count_end());
    assert_eq!(
        <dyn Greeter as Kind>::REQUIRED
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        ["greet", "shapes", "describe", "count"]
    );
    assert_ne!(<dyn Greeter as Kind>::FLOOR_HASH, 0);
    assert_eq!(
        HELLO_TABLE.header.floor_hash,
        <dyn Greeter as Kind>::FLOOR_HASH
    );
    assert_eq!(
        HELLO_TABLE.header.struct_size as usize,
        size_of::<GreeterVtable>()
    );
}

#[test]
fn a_call_round_trips_through_the_proxy() {
    let r = hello();
    assert_eq!(r.greet("ana").unwrap(), "hello, ana");
    assert_eq!(r.count(), 7);
    assert_eq!(r.shout("bo"), "HELLO, BO!", "the appended slot is called");

    let e = r.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet");

    let described = r.describe("ana").unwrap();
    assert_eq!(
        described.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );

    let mut config = Map::new();
    config.set("port", 5900).unwrap();
    let config = Value::from(config);
    let mut map = Map::new();
    map.set("a", 1).unwrap();
    let got = r
        .shapes(
            true,
            3,
            1.5,
            b"xyz",
            &config,
            None,
            &map,
            Greeting {
                text: "in".into(),
                count: 41,
            },
        )
        .unwrap();
    assert_eq!(
        got,
        Greeting {
            text: "true 3 1.5 3 5900 none 1".into(),
            count: 42
        }
    );
    let got = r
        .shapes(
            false,
            0,
            0.0,
            b"",
            &config,
            Some(&config),
            &map,
            Greeting {
                text: String::new(),
                count: 0,
            },
        )
        .unwrap();
    assert_eq!(got.text, "false 0 0 0 5900 some 1");
}

#[test]
fn the_proxy_is_the_trait() {
    let r = hello();
    let kept: Box<dyn Greeter> = r.into();
    assert_eq!(kept.greet("cy").unwrap(), "hello, cy");
    let shared: std::sync::Arc<dyn Greeter> = <dyn Greeter as Kind>::shared(hello());
    assert_eq!(shared.count(), 7);
    let again: Box<dyn Greeter> = <dyn Greeter as Kind>::boxed(hello());
    assert_eq!(again.shout("di"), "HELLO, DI!");
}

#[test]
fn a_panic_inside_the_implementation_is_a_status() {
    let r = remote(
        &PANICS_TABLE,
        &PANICS as *const Panics as *const c_void,
        size_of::<GreeterVtable>(),
    )
    .unwrap();
    let e = r.greet("x").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL);
}

#[test]
fn a_truncated_table_runs_the_default_body() {
    let r = remote(
        &HELLO_TABLE,
        &HELLO as *const Hello as *const c_void,
        GreeterVtable::floor(),
    )
    .expect("the floor is enough");
    assert_eq!(r.greet("ana").unwrap(), "hello, ana");
    assert_eq!(
        r.shout("ana"),
        "HELLO, ANA!",
        "the slot is past the table, so the default body runs — through the proxy's own `greet`"
    );
}

#[test]
fn each_mismatch_is_named() {
    let ctx = &HELLO as *const Hello as *const c_void;
    assert_eq!(
        remote(&HELLO_TABLE, ctx, GreeterVtable::floor() - 1).unwrap_err(),
        KindMismatch::BelowFloor {
            size: GreeterVtable::floor() - 1,
            floor: GreeterVtable::floor()
        }
    );

    let mut forged = GreeterVtable::of::<Hello>();
    forged.header.floor_hash ^= 1;
    let forged: &'static GreeterVtable = Box::leak(Box::new(forged));
    assert!(matches!(
        remote(forged, ctx, size_of::<GreeterVtable>()).unwrap_err(),
        KindMismatch::HashMismatch { .. }
    ));

    let mut nulled = GreeterVtable::of::<Hello>();
    nulled.greet = None;
    let nulled: &'static GreeterVtable = Box::leak(Box::new(nulled));
    assert_eq!(
        remote(nulled, ctx, size_of::<GreeterVtable>()).unwrap_err(),
        KindMismatch::NullRequiredSlot("greet")
    );
}

// --- object kinds, in one process ------------------------------------------

/// A handle one caller owns: `&mut self`, an out-buffer, a `destroy`.
#[guatiao::kind(object)]
pub trait Tally: Send {
    /// Adds and answers the running total.
    fn add(&mut self, n: i64) -> i64;
    /// Copies the total's decimal text into `dst`; answers bytes written.
    fn render(&mut self, dst: &mut [u8]) -> i64;
    /// Appended: an older table lacks it.
    fn total(&self) -> i64 {
        -1
    }
}

struct Sum {
    total: i64,
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Tally for Sum {
    fn add(&mut self, n: i64) -> i64 {
        self.total += n;
        self.total
    }
    fn render(&mut self, dst: &mut [u8]) -> i64 {
        let text = self.total.to_string();
        let n = text.len().min(dst.len());
        dst[..n].copy_from_slice(&text.as_bytes()[..n]);
        n as i64
    }
    fn total(&self) -> i64 {
        self.total
    }
}

impl Drop for Sum {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn dropped_flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

#[test]
fn an_object_kind_declares_itself() {
    const { assert!(<dyn Tally as Kind>::OBJECT) };
    const { assert!(!<dyn Greeter as Kind>::OBJECT) };
    assert_eq!(
        <dyn Tally as Kind>::REQUIRED
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["destroy", "add", "render"],
        "destroy is the first required slot"
    );
    assert_eq!(TallyVtable::destroy_end(), 8 + size_of::<usize>());
    assert_eq!(<dyn Tally as Kind>::FLOOR, TallyVtable::render_end());
    assert_ne!(
        <dyn Tally as Kind>::FLOOR_HASH,
        <dyn Greeter as Kind>::FLOOR_HASH
    );
}

/// The hash the attribute wrote as a literal (so a C header can carry
/// it) is the hash the runtime computes: the two implementations of
/// FNV-1a agree, on this exact input.
#[test]
fn the_literal_floor_hash_is_the_runtime_hash() {
    use guatiao::library::kind::fnv1a;
    assert_eq!(
        TallyVtable::FLOOR_HASH,
        fnv1a("object;add(i64)->i64;render(&mut[u8])->i64")
    );
    assert_eq!(<dyn Tally as Kind>::FLOOR_HASH, TallyVtable::FLOOR_HASH);
    assert_eq!(
        GreeterVtable::FLOOR_HASH,
        fnv1a(
            "provider;greet(&str)->Result<String,ProviderError>;\
             shapes(bool,u32,f64,&[u8],&Value,Option<&Value>,&Map,Greeting)->Result<Greeting,ProviderError>;\
             describe(&str)->Result<Map,ProviderError>;count()->i64"
        )
    );
    // The known vectors, so a wrong constant on either side shows here
    // and not only as a mismatch between the two.
    assert_eq!(fnv1a(""), 0x811c_9dc5);
    assert_eq!(fnv1a("a"), 0xe40c_292c);
}

#[test]
fn an_object_is_driven_through_its_handle_and_destroyed_once() {
    let dropped = dropped_flag();
    let mut tally = Sum {
        total: 0,
        dropped: dropped.clone(),
    }
    .into_object();
    assert_eq!(tally.add(2), 2);
    assert_eq!(tally.add(40), 42);
    assert_eq!(
        tally.total(),
        42,
        "the appended slot is present in a full table"
    );
    let mut buffer = [0u8; 8];
    assert_eq!(tally.render(&mut buffer), 2);
    assert_eq!(&buffer[..2], b"42");
    let mut short = [0u8; 1];
    assert_eq!(
        tally.render(&mut short),
        1,
        "a short buffer is filled, not overrun"
    );
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
    drop(tally);
    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "destroy ran once"
    );
}

#[test]
fn an_object_crosses_as_raw_parts_and_a_mismatch_destroys_it() {
    use guatiao::library::Object;

    // Out and back: what a shim writes and a proxy reads.
    let dropped = dropped_flag();
    let tally = Sum {
        total: 7,
        dropped: dropped.clone(),
    }
    .into_object();
    let raw = tally.into_raw();
    assert_eq!(raw.size, size_of::<TallyVtable>());
    // SAFETY: `raw` came from `into_raw` on a handle for this kind.
    let mut back = unsafe { Object::<dyn Tally>::from_raw(raw) }.expect("the same table");
    assert_eq!(back.add(1), 8);
    drop(back);
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));

    // Presented as the wrong kind: refused, and destroyed rather than
    // leaked, since whoever received it owned it.
    let dropped = dropped_flag();
    let raw = Sum {
        total: 0,
        dropped: dropped.clone(),
    }
    .into_object()
    .into_raw();
    // SAFETY: the table is a real one; the kind is deliberately wrong,
    // which is what the hash check is for.
    let why = unsafe { Object::<dyn Greeter>::from_raw(raw) }.unwrap_err();
    assert!(matches!(why, KindMismatch::HashMismatch { .. }), "{why:?}");
    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "destroyed on refusal"
    );

    // All-null is "no object", refused as no table, nothing to destroy.
    // SAFETY: null everywhere is the documented "no object".
    let why =
        unsafe { Object::<dyn Tally>::from_raw(guatiao::library::ObjectRaw::null()) }.unwrap_err();
    assert_eq!(why, KindMismatch::NoTable);
}
