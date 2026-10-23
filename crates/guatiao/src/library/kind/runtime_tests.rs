// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables built at run time. The dispatch tests hand `dispatch` the
//! argument array libffi would, so they run under Miri; the closures and
//! `ffi_call` run natively only.

use std::sync::Mutex;

use super::*;
use crate::value::convert::TryAsRef;

/// The committed description of the example kinds, read as a value.
fn kinds() -> Vec<Arc<KindDescription>> {
    let text = include_str!("../../../../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let json: serde_json::Value = serde_json::from_str(text).expect("JSON");
    KindDescription::document(&value_of(&json))
        .expect("a description")
        .into_iter()
        .map(Arc::new)
        .collect()
}

fn kind(name: &str) -> Arc<KindDescription> {
    kinds()
        .into_iter()
        .find(|k| k.name() == name)
        .expect("described")
}

fn value_of(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::null(),
        serde_json::Value::Bool(b) => Value::from(*b),
        serde_json::Value::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => Value::from(u),
            (None, Some(i)) => Value::from(i),
            _ => Value::from(
                crate::value::types::Number::try_from(n.as_f64().expect("a float"))
                    .expect("finite"),
            ),
        },
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Array(items) => {
            let mut list = List::new();
            for item in items {
                list.push(value_of(item));
            }
            Value::from(list)
        }
        serde_json::Value::Object(entries) => {
            let mut map = Map::new();
            for (k, v) in entries {
                map.set(k.as_str(), value_of(v));
            }
            Value::from(map)
        }
    }
}

/// A greeter, a counter and a wide kind served by one handler, counting
/// what it was asked and what it destroyed.
#[derive(Default)]
struct Served {
    calls: Mutex<usize>,
    destroyed: Mutex<usize>,
}

impl Serve for Served {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        match (kind.name(), kind.slots()[slot].name()) {
            ("greeter", "greet") => {
                let [Arg::Str(name)] = args.as_slice() else {
                    panic!("{args:?}");
                };
                if name.is_empty() {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_BAD_VALUE,
                        "nobody to greet",
                    ));
                }
                if *name == "panic" {
                    panic!("asked to");
                }
                if *name == "text" {
                    return Ok(Answer::Text("not a map".to_string()));
                }
                let mut map = Map::new();
                map.set("greeting", format!("hello, {name} ({ctx})").as_str());
                Ok(Answer::Map(map))
            }
            ("greeter", "shout") => {
                let [Arg::Str(name)] = args.as_slice() else {
                    panic!("{args:?}");
                };
                Ok(Answer::Text(name.to_uppercase()))
            }
            ("greeter", "check") => {
                let Some(Arg::LentMap(config)) = args.pop() else {
                    panic!("a lent map");
                };
                if config.get("name").is_none() {
                    config.set("name", "ana");
                }
                Ok(Answer::Bool(true))
            }
            ("greeter", "greet_from") => {
                let Some(Arg::TakenMap(config)) = args.pop() else {
                    panic!("a taken map");
                };
                let name = config
                    .get("name")
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("")
                    .to_string();
                drop(config);
                let mut map = Map::new();
                map.set("greeting", format!("hello, {name}").as_str());
                Ok(Answer::Value(Value::from(map)))
            }
            ("greeter", "start") => {
                let Some(Arg::Object(listener)) = args.pop() else {
                    panic!("a taken object");
                };
                drop(listener);
                Err(ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    "no conversations here",
                ))
            }
            ("counter", "count") => Ok(Answer::Uint(u64::MAX)),
            ("wide", "echo_i64") => match args.pop() {
                Some(Arg::Int(x)) => Ok(Answer::Int(x)),
                other => panic!("{other:?}"),
            },
            ("wide", "echo_u64") => match args.pop() {
                Some(Arg::Uint(x)) => Ok(Answer::Value(Value::from(x))),
                other => panic!("{other:?}"),
            },
            ("listener", "heard") => Ok(Answer::Nothing),
            (k, s) => panic!("{k}.{s} was not expected"),
        }
    }

    fn destroy(&self, _ctx: usize) {
        *self.destroyed.lock().unwrap() += 1;
    }
}

/// Builds the argument array libffi hands a closure: one address per
/// argument.
fn array(parts: &mut [&mut dyn ArgSlot]) -> Vec<*mut c_void> {
    // Through the trait object, to the argument's own storage.
    parts.iter_mut().map(|p| (**p).address()).collect()
}

trait ArgSlot {
    fn address(&mut self) -> *mut c_void;
}

impl<T> ArgSlot for T {
    fn address(&mut self) -> *mut c_void {
        (self as *mut T).cast()
    }
}

fn ctx(n: usize) -> *mut c_void {
    core::ptr::without_provenance_mut(n)
}

fn greet_through_dispatch(serve: &Served, name: &[u8]) -> (Status, Map, ProviderError) {
    let greeter = kind("greeter");
    let (slot, _) = greeter.slot("greet").expect("a slot");
    let mut c = ctx(7);
    // SAFETY: the bytes outlive the call; `greet` checks them as UTF-8.
    let mut arg = unsafe { Str::from_raw_parts(name.as_ptr(), name.len()) };
    let mut out = Map::carrier(Alloc::rust());
    let mut out_p: *mut Map = &mut out;
    let mut err = ProviderError::none();
    let mut err_p: *mut ProviderError = &mut err;
    let args = array(&mut [&mut c, &mut arg, &mut out_p, &mut err_p]);
    // SAFETY: the array of a call to `greet`: ctx, str, out, err.
    let status = unsafe { dispatch(serve, &greeter, slot, args.as_ptr()) };
    (status, out, err)
}

#[test]
fn a_slot_answers_a_map_and_refuses_through_err() {
    let serve = Served::default();
    let (status, out, _) = greet_through_dispatch(&serve, b"ana");
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(
        out.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ana (7)")
    );

    let (status, out, err) = greet_through_dispatch(&serve, b"");
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(err.message(), "nobody to greet");
    assert!(out.is_empty());

    let (status, _, err) = greet_through_dispatch(&serve, &[0xff, 0xfe]);
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(err.message().contains("UTF-8"), "{}", err.message());
    assert_eq!(
        *serve.calls.lock().unwrap(),
        2,
        "the handler never saw the bad text"
    );
}

#[test]
fn a_panic_or_the_wrong_answer_is_a_status() {
    let serve = Served::default();
    let (status, _, err) = greet_through_dispatch(&serve, b"panic");
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(err.status, Status::GUATIAO_OK, "nothing written");

    let (status, _, err) = greet_through_dispatch(&serve, b"text");
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert!(err.message().contains("text"), "{}", err.message());
}

#[test]
fn a_lent_map_is_completed_in_place() {
    let serve = Served::default();
    let greeter = kind("greeter");
    let (slot, _) = greeter.slot("check").expect("a slot");
    let mut c = ctx(1);
    let mut config = Map::new();
    let mut config_p: *mut Map = &mut config;
    let mut out = false;
    let mut out_p: *mut bool = &mut out;
    let mut err = ProviderError::none();
    let mut err_p: *mut ProviderError = &mut err;
    let args = array(&mut [&mut c, &mut config_p, &mut out_p, &mut err_p]);
    // SAFETY: the array of a call to `check`: ctx, map, out, err.
    let status = unsafe { dispatch(&serve, &greeter, slot, args.as_ptr()) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert!(out);
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
}

#[test]
fn a_taken_map_is_owned_by_the_slot() {
    let serve = Served::default();
    let greeter = kind("greeter");
    let (slot, _) = greeter.slot("greet_from").expect("a slot");
    let mut c = ctx(1);
    let mut config = core::mem::ManuallyDrop::new({
        let mut m = Map::new();
        m.set("name", "bo");
        m
    });
    let mut config_p: *mut Map = &mut *config;
    let mut out = Map::carrier(Alloc::rust());
    let mut out_p: *mut Map = &mut out;
    let mut err = ProviderError::none();
    let mut err_p: *mut ProviderError = &mut err;
    let args = array(&mut [&mut c, &mut config_p, &mut out_p, &mut err_p]);
    // SAFETY: the array of a call to `greet_from`: ctx, a map handed over,
    // out, err. `config` is never dropped here.
    let status = unsafe { dispatch(&serve, &greeter, slot, args.as_ptr()) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(
        out.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, bo")
    );
}

/// A listener's `destroy`: its `ctx` is the test's count.
unsafe extern "C" fn listener_destroy(ctx: *mut c_void) {
    // SAFETY: the `Mutex<usize>` the test made the object's `ctx`.
    *unsafe { &*ctx.cast::<Mutex<usize>>() }.lock().unwrap() += 1;
}

#[test]
fn a_taken_object_is_destroyed_once_when_the_slot_lets_it_go() {
    #[repr(C)]
    struct ListenerTable {
        header: KindHeader,
        destroy: Option<unsafe extern "C" fn(*mut c_void)>,
        heard: Option<unsafe extern "C" fn(*mut c_void, Str) -> Status>,
    }
    let listener = kind("listener");
    let table = ListenerTable {
        header: KindHeader::new(size_of::<ListenerTable>(), listener.floor_hash()),
        destroy: Some(listener_destroy),
        heard: None,
    };
    let serve = Served::default();
    let greeter = kind("greeter");
    let (slot, _) = greeter.slot("start").expect("a slot");
    let mut c = ctx(1);
    let destroyed = Mutex::new(0usize);
    let mut object = ObjectRaw {
        table: (&table as *const ListenerTable).cast(),
        size: size_of::<ListenerTable>(),
        ctx: (&destroyed as *const Mutex<usize>).cast_mut().cast(),
    };
    let mut out = ObjectRaw::null();
    let mut out_p: *mut ObjectRaw = &mut out;
    let mut err = ProviderError::none();
    let mut err_p: *mut ProviderError = &mut err;
    let args = array(&mut [&mut c, &mut object, &mut out_p, &mut err_p]);
    // SAFETY: the array of a call to `start`: ctx, an object handed over,
    // out, err.
    let status = unsafe { dispatch(&serve, &greeter, slot, args.as_ptr()) };
    assert_eq!(status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert_eq!(err.message(), "no conversations here");
    assert_eq!(*destroyed.lock().unwrap(), 1);
}

#[test]
fn a_number_is_narrowed_to_its_slot_or_refused() {
    let serve = Served::default();
    let counter = kind("counter");
    let mut c = ctx(1);
    let mut out = 0i64;
    let mut out_p: *mut i64 = &mut out;
    let args = array(&mut [&mut c, &mut out_p]);
    // SAFETY: the array of a call to `count`: ctx, out.
    let status = unsafe { dispatch(&serve, &counter, 0, args.as_ptr()) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "u64::MAX is no i64");

    let wide = kind("wide");
    let (slot, _) = wide.slot("echo_u64").expect("a slot");
    let mut x = u64::MAX;
    let mut out = 0u64;
    let mut out_p: *mut u64 = &mut out;
    let mut err = ProviderError::none();
    let mut err_p: *mut ProviderError = &mut err;
    let args = array(&mut [&mut c, &mut x, &mut out_p, &mut err_p]);
    // SAFETY: the array of a call to `echo_u64`: ctx, u64, out, err.
    let status = unsafe { dispatch(&serve, &wide, slot, args.as_ptr()) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out, u64::MAX);
}

// --- the closures and ffi_call, natively -----------------------------------------

type GreetFn = unsafe extern "C" fn(*mut c_void, Str, *mut Map, *mut ProviderError) -> Status;

#[test]
#[cfg_attr(miri, ignore = "libffi")]
fn a_table_is_built_with_its_header_and_a_null_for_each_slot_left_out() {
    let greeter = kind("greeter");
    let serve: Arc<dyn Serve> = Arc::new(Served::default());
    let table =
        ServedTable::new(Arc::clone(&greeter), |s| s.name() != "shout", serve).expect("built");
    // SAFETY: the table's own header.
    let header = unsafe { table.as_ptr().cast::<KindHeader>().read() };
    assert_eq!(header.floor_hash, greeter.floor_hash());
    assert_eq!(header.struct_size as usize, greeter.size());
    // SAFETY: two pointer-sized slots after the header.
    let (greet, shout) = unsafe {
        let slots = table.as_ptr().cast::<u8>().add(8).cast::<Option<GreetFn>>();
        (slots.read(), slots.add(1).read())
    };
    assert!(shout.is_none());
    let greet = greet.expect("greet is served");
    let mut out = Map::carrier(Alloc::rust());
    let mut err = ProviderError::none();
    // SAFETY: the slot's own signature, as the description gives it.
    let status = unsafe { greet(ctx(9), Str::new("cy"), &mut out, &mut err) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(
        out.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, cy (9)")
    );

    let required = ServedTable::new(
        greeter,
        |s| s.name() != "greet",
        Arc::new(Served::default()),
    );
    assert_eq!(
        required.unwrap_err(),
        TableError::MissingRequired("greet".to_string())
    );
}

fn call_on(
    caller: &Caller,
    table: &ServedTable,
    n: usize,
    slot: usize,
    args: Vec<Arg<'_>>,
) -> Result<Answer, ProviderError> {
    // SAFETY: a table built for the caller's kind, alive for the call.
    unsafe { caller.call_raw(table.as_ptr(), table.size(), ctx(n), slot, args) }
}

#[test]
#[cfg_attr(miri, ignore = "libffi")]
fn a_caller_reaches_a_served_table_with_every_shape() {
    let serve = Arc::new(Served::default());
    let wide = kind("wide");
    let table = ServedTable::new(Arc::clone(&wide), |_| true, serve.clone()).expect("built");
    let caller = Caller::new(Arc::clone(&wide)).expect("interfaces");
    let call = |slot, args| call_on(&caller, &table, 2, slot, args);
    assert!(matches!(
        call(0, vec![Arg::Int(i64::MIN)]),
        Ok(Answer::Int(i64::MIN))
    ));
    assert!(matches!(
        call(1, vec![Arg::Uint(u64::MAX)]),
        Ok(Answer::Uint(u64::MAX))
    ));
    assert_eq!(
        call(0, vec![Arg::Uint(u64::MAX)]).unwrap_err().status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "narrowed, and refused"
    );

    let greeter = kind("greeter");
    let table = ServedTable::new(Arc::clone(&greeter), |_| true, serve.clone()).expect("built");
    let caller = Caller::new(Arc::clone(&greeter)).expect("interfaces");
    let call = |slot, args: Vec<Arg<'static>>| call_on(&caller, &table, 4, slot, args);
    let (greet, _) = greeter.slot("greet").expect("a slot");
    let Ok(Answer::Map(map)) = call(greet, vec![Arg::Str("di")]) else {
        panic!("a map");
    };
    assert_eq!(
        map.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, di (4)")
    );
    let refused = call(greet, vec![Arg::Str("")]).unwrap_err();
    assert_eq!(refused.message(), "nobody to greet");
    let (check, _) = greeter.slot("check").expect("a slot");
    let mut config = Map::new();
    assert!(matches!(
        call_on(&caller, &table, 4, check, vec![Arg::LentMap(&mut config)]),
        Ok(Answer::Bool(true))
    ));
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
    let (shout, _) = greeter.slot("shout").expect("a slot");
    assert!(matches!(call(shout, vec![Arg::Str("ab")]), Ok(Answer::Text(t)) if t == "AB"));
    let (from, _) = greeter.slot("greet_from").expect("a slot");
    let mut given = Map::new();
    given.set("name", "ed");
    assert!(matches!(
        call(from, vec![Arg::TakenMap(given)]),
        Ok(Answer::Map(_))
    ));
}

#[test]
#[cfg_attr(miri, ignore = "libffi")]
fn an_object_table_built_at_run_time_is_destroyed_through_its_handler() {
    let serve = Arc::new(Served::default());
    let listener = kind("listener");
    let table = ServedTable::new(Arc::clone(&listener), |_| true, serve.clone()).expect("built");
    let caller = Caller::new(listener).expect("interfaces");
    let mut object = table.object(11).expect("an object kind");
    assert!(matches!(
        object.call(&caller, 0, vec![Arg::Str("hi")]),
        Ok(Answer::Nothing)
    ));
    drop(object);
    assert_eq!(*serve.destroyed.lock().unwrap(), 1);
}

#[test]
#[cfg_attr(miri, ignore = "libffi")]
fn a_lent_object_is_called_from_another_thread_while_lent_and_refused_after() {
    let serve = Arc::new(Served::default());
    let listener = kind("listener");
    let table = ServedTable::new(Arc::clone(&listener), |_| true, serve.clone()).expect("built");
    let caller = Arc::new(Caller::new(listener).expect("interfaces"));
    let object = table.object(12).expect("an object kind");
    // SAFETY: an object this test owns, lent for the scope below.
    let lent = unsafe { LentObject::from_raw(object.as_raw()) };
    let kept = lent.share(|handle| {
        let (h, c) = (handle.clone(), Arc::clone(&caller));
        let answered = std::thread::spawn(move || h.call(&c, 0, vec![Arg::Str("hi")]))
            .join()
            .expect("the thread ran");
        assert!(matches!(answered, Ok(Answer::Nothing)));
        handle
    });
    let after = kept.call(&caller, 0, vec![Arg::Str("late")]).unwrap_err();
    assert_eq!(after.status, Status::GUATIAO_ERR_GONE);
    assert_eq!(
        *serve.calls.lock().unwrap(),
        1,
        "the late call never reached it"
    );
    drop(object);
}
