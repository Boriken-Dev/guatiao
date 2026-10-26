// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables built at run time through libffi: a slot called as a plain C
//! function pointer, every argument shape through `ffi_call`, an object's
//! `destroy` reached through its closure, and a provider's own `create`,
//! `destroy` and `available` called as the C slots they are.

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::c_void;
use std::sync::Mutex;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, Arg, CSignature, Caller, Carrier, KindDescription, KindHeader, LentObject, Mechanism,
    Serve, ServedProvider, ServedTable, TableError,
};
use guatiao::value::convert::TryAsRef;
use guatiao::value::types::Str;
use guatiao::{Alloc, Map, Status, Value};
use guatiao_libffi::Libffi;

/// The committed description of the example kind `name`.
fn kind(name: &str) -> Arc<KindDescription> {
    let text = include_str!("../../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let document = guatiao_serde::text::json::from_str(
        text,
        Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .expect("JSON");
    let kind = KindDescription::document(&document)
        .expect("a description")
        .into_iter()
        .find(|k| k.name() == name)
        .expect("described");
    Arc::new(kind)
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

fn ctx(n: usize) -> *mut c_void {
    core::ptr::without_provenance_mut(n)
}

type GreetFn = unsafe extern "C" fn(*mut c_void, Str, *mut Map, *mut ProviderError) -> Status;

#[test]
fn a_table_is_built_with_its_header_and_a_null_for_each_slot_left_out() {
    let greeter = kind("greeter");
    let serve: Arc<dyn Serve> = Arc::new(Served::default());
    let table = ServedTable::new(
        Arc::clone(&greeter),
        |s| s.name() != "shout",
        serve,
        &Libffi,
    )
    .expect("built");
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
        &Libffi,
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
fn a_caller_reaches_a_served_table_with_every_shape() {
    let serve = Arc::new(Served::default());
    let wide = kind("wide");
    let table =
        ServedTable::new(Arc::clone(&wide), |_| true, serve.clone(), &Libffi).expect("built");
    let caller = Caller::new(Arc::clone(&wide), &Libffi).expect("interfaces");
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
    let table =
        ServedTable::new(Arc::clone(&greeter), |_| true, serve.clone(), &Libffi).expect("built");
    let caller = Caller::new(Arc::clone(&greeter), &Libffi).expect("interfaces");
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
fn an_object_table_built_at_run_time_is_destroyed_through_its_handler() {
    let serve = Arc::new(Served::default());
    let listener = kind("listener");
    let table =
        ServedTable::new(Arc::clone(&listener), |_| true, serve.clone(), &Libffi).expect("built");
    let caller = Caller::new(listener, &Libffi).expect("interfaces");
    let mut object = table.object(11).expect("an object kind");
    assert!(matches!(
        object.call(&caller, 0, vec![Arg::Str("hi")]),
        Ok(Answer::Nothing)
    ));
    drop(object);
    assert_eq!(*serve.destroyed.lock().unwrap(), 1);
}

#[test]
fn a_lent_object_is_called_from_another_thread_while_lent_and_refused_after() {
    let serve = Arc::new(Served::default());
    let listener = kind("listener");
    let table =
        ServedTable::new(Arc::clone(&listener), |_| true, serve.clone(), &Libffi).expect("built");
    let caller = Arc::new(Caller::new(listener, &Libffi).expect("interfaces"));
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

/// A provider whose instances count up from 40 and whose `available`
/// answers what it is told.
#[derive(Default)]
struct Factory {
    made: Mutex<Vec<(usize, String)>>,
    gone: Mutex<Vec<(usize, usize)>>,
    down: Mutex<Option<String>>,
}

impl Serve for Factory {
    fn call(
        &self,
        _: usize,
        _: &KindDescription,
        _: usize,
        _: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        Ok(Answer::Nothing)
    }

    fn create(&self, provider: usize, config: &Value) -> Result<usize, ProviderError> {
        let word = TryAsRef::<Map>::try_as_ref(config)
            .and_then(|m| m.get("word"))
            .and_then(TryAsRef::<str>::try_as_ref)
            .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "no word"))?;
        let mut made = self.made.lock().unwrap();
        made.push((provider, word.to_string()));
        Ok(39 + made.len())
    }

    fn destroy_instance(&self, provider: usize, instance: usize) {
        self.gone.lock().unwrap().push((provider, instance));
    }

    fn available(&self, _: usize) -> Result<(), String> {
        self.down.lock().unwrap().clone().map_or(Ok(()), Err)
    }
}

#[test]
fn a_providers_own_slots_are_called_as_c_functions() {
    let serve = Arc::new(Factory::default());
    let slots = ServedProvider::new(9, true, true, serve.clone(), &Libffi).expect("built");
    let (create, destroy, available) = (
        slots.create_slot().expect("create"),
        slots.destroy_slot().expect("destroy"),
        slots.available_slot().expect("available"),
    );

    let mut config = Map::new();
    config.set("word", "hi");
    let config = Value::from(config);
    let mut out = core::ptr::null_mut::<c_void>();
    let mut err = ProviderError::none();
    // SAFETY: the slot's own signature, every pointer valid for the call.
    let status = unsafe { create(core::ptr::null_mut(), &config, &mut out, &mut err) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out.addr(), 40);
    assert_eq!(*serve.made.lock().unwrap(), [(9, "hi".to_string())]);

    let empty = Value::from(Map::new());
    // SAFETY: as above.
    let status = unsafe { create(core::ptr::null_mut(), &empty, &mut out, &mut err) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(err.message(), "no word");

    // SAFETY: as above; 40 is the instance `create` answered.
    unsafe { destroy(core::ptr::null_mut(), core::ptr::without_provenance_mut(40)) };
    assert_eq!(*serve.gone.lock().unwrap(), [(9, 40)]);

    let mut reason = Str::empty();
    // SAFETY: as above.
    assert!(unsafe { available(core::ptr::null_mut(), &mut reason) });
    *serve.down.lock().unwrap() = Some("unplugged".to_string());
    // SAFETY: as above.
    assert!(!unsafe { available(core::ptr::null_mut(), &mut reason) });
    assert_eq!(<&[u8]>::from(reason), b"unplugged");

    // The same slot through `ffi_call`, as a caller of a `bool` signature.
    let prepared = Libffi.prepare(&CSignature::available()).expect("prepared");
    let mut c = core::ptr::null_mut::<c_void>();
    let mut reason_p: *mut Str = &mut reason;
    let args = [
        (&mut c as *mut *mut c_void).cast::<c_void>(),
        (&mut reason_p as *mut *mut Str).cast::<c_void>(),
    ];
    // SAFETY: the code of a closure prepared with this signature.
    let code = unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*mut c_void, *mut Str) -> bool,
            unsafe extern "C" fn(),
        >(available)
    };
    // SAFETY: `code` has the signature; each argument addresses its value.
    assert_eq!(unsafe { prepared.call(code, &args) }, 0);
    *serve.down.lock().unwrap() = None;
    // SAFETY: as above.
    assert_eq!(unsafe { prepared.call(code, &args) }, 1);
}
