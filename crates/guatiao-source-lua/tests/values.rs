// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! guatiao's values as Lua's and back: the shared vectors, and the rules
//! for what Lua writes itself.

use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Value};
use guatiao_bridge::Convert;
use guatiao_bridge::conformance::{Outcome, round_trip};
use guatiao_source_lua::LuaValues;
use guatiao_source_lua::mlua::prelude::*;

fn lua() -> (Lua, LuaValues) {
    let lua = Lua::new();
    let guatiao = guatiao_source_lua::helpers(&lua).expect("the helpers");
    lua.globals().set("guatiao", guatiao).expect("set");
    let values = LuaValues::new(&lua);
    (lua, values)
}

fn from_lua(lua: &Lua, values: &LuaValues, code: &str) -> Result<Value, String> {
    let v: LuaValue = lua.load(code).eval().map_err(|e| e.to_string())?;
    values.to_value(v).map_err(|e| e.to_string())
}

#[test]
fn every_shared_vector_comes_back_the_same() {
    let (_lua, values) = lua();
    let outcomes = round_trip(&values);
    assert!(outcomes.len() >= 20, "every vector ran");
    for (name, outcome) in outcomes {
        assert!(matches!(outcome, Outcome::Same), "{name}: {outcome:?}");
    }
}

#[test]
fn numbers_are_lua_numbers_where_one_holds_them_exactly() {
    let (lua, values) = lua();
    // Lua 5.4 and 5.5 hold a 64-bit integer; Luau's numbers are doubles.
    let i64_max = if cfg!(feature = "luau") {
        "userdata"
    } else {
        "number"
    };
    for (text, ty) in [
        ("0", "number"),
        ("-7", "number"),
        ("9007199254740992", "number"),
        ("9223372036854775807", i64_max),
        ("1.5", "number"),
        ("0.1", "number"),
        ("1.10", "userdata"),
        ("1e3", "userdata"),
        ("18446744073709551615", "userdata"),
    ] {
        let native = values
            .to_native(&Value::from(
                guatiao::value::types::Number::new(text).expect("a number"),
            ))
            .expect("converted");
        lua.globals().set("n", native).expect("set");
        let seen: String = lua.load("return type(n)").eval().expect("typed");
        assert_eq!(seen, ty, "{text}");
        let back = values
            .to_value(lua.globals().get("n").expect("n"))
            .expect("back");
        let back: &guatiao::value::types::Number = TryAsRef::try_as_ref(&back).expect("a number");
        assert_eq!(&**back, text, "comes back as it went");
    }
    // An exact number still computes, through its float.
    let sum: f64 = lua
        .load("return guatiao.number('1.10') + 1")
        .eval()
        .expect("added");
    assert!((sum - 2.1).abs() < 1e-12);
}

#[test]
fn a_plain_table_is_a_list_or_a_map_by_its_keys() {
    let (lua, values) = lua();
    let list = from_lua(&lua, &values, "return {1, 'two', true}").expect("a list");
    assert_eq!(
        TryAsRef::<List>::try_as_ref(&list).map(|l| l.len()),
        Some(3)
    );
    let map = from_lua(&lua, &values, "return {b = 1, a = 2}").expect("a map");
    let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&map)
        .expect("a map")
        .keys()
        .collect();
    assert_eq!(keys, ["a", "b"], "a plain table's keys in byte order");
    let empty = from_lua(&lua, &values, "return {}").expect("empty");
    assert!(
        TryAsRef::<Map>::try_as_ref(&empty).is_some(),
        "an empty table is a map"
    );
    let marked = from_lua(&lua, &values, "return guatiao.list{}").expect("empty list");
    assert!(TryAsRef::<List>::try_as_ref(&marked).is_some());

    let mixed = from_lua(&lua, &values, "return {1, x = 2}").unwrap_err();
    assert!(mixed.contains("neither a map nor a list"), "{mixed}");
    let holes = from_lua(&lua, &values, "return {[1] = 1, [3] = 3}").unwrap_err();
    assert!(holes.contains("neither a map nor a list"), "{holes}");
    let cycle = from_lua(&lua, &values, "local t = {}; t.me = t; return t").unwrap_err();
    assert!(cycle.contains("contains itself"), "{cycle}");
    let function = from_lua(&lua, &values, "return {f = print}").unwrap_err();
    assert!(function.contains("function has no value"), "{function}");
    let nan = from_lua(&lua, &values, "return 0/0").unwrap_err();
    assert!(nan.contains("not finite"), "{nan}");
}

#[test]
fn a_map_keeps_its_order_and_puts_new_keys_after() {
    let (lua, values) = lua();
    let mut map = Map::new();
    map.set("z", 1i64);
    map.set("a", 2i64);
    let native = values.to_native(&Value::from(map)).expect("converted");
    lua.globals().set("m", native).expect("set");
    let back = from_lua(&lua, &values, "m.a = nil; m.c = 3; m.b = 4; return m").expect("a map");
    let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&back)
        .expect("a map")
        .keys()
        .collect();
    assert_eq!(keys, ["z", "b", "c"]);
}

#[test]
fn text_bytes_null_and_absent_keep_apart() {
    let (lua, values) = lua();
    let v = from_lua(
        &lua,
        &values,
        "return guatiao.list{'text', guatiao.bytes('text'), '\\255', guatiao.null, guatiao.absent}",
    )
    .expect("a list");
    let list: &List = TryAsRef::try_as_ref(&v).expect("a list");
    let items: Vec<&Value> = list.iter().collect();
    assert_eq!(TryAsRef::<str>::try_as_ref(items[0]), Some("text"));
    assert_eq!(TryAsRef::<[u8]>::try_as_ref(items[1]), Some(&b"text"[..]));
    assert_eq!(
        TryAsRef::<[u8]>::try_as_ref(items[2]),
        Some(&b"\xff"[..]),
        "a string that is not UTF-8 is bytes"
    );
    assert_eq!(items[3].tag(), Ok(guatiao::Tag::GUATIAO_NULL));
    assert_eq!(items[4].tag(), Ok(guatiao::Tag::GUATIAO_ABSENT));
    let none = from_lua(&lua, &values, "return nil").expect("nil");
    assert_eq!(none.tag(), Ok(guatiao::Tag::GUATIAO_ABSENT));
}

#[test]
fn a_refusal_is_a_value_lua_code_reads() {
    let (lua, _) = lua();
    let (status, code, message): (String, u32, String) = lua
        .load("local e = guatiao.error('not_found', 'no such greeting'); return e.status, e.code, tostring(e)")
        .eval()
        .expect("read");
    assert_eq!(
        (status.as_str(), code, message.as_str()),
        ("not_found", 10, "no such greeting")
    );
    assert!(lua.load("guatiao.error('nonsense', 'x')").exec().is_err());
}
