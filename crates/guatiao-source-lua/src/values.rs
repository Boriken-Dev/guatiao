// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lua values and guatiao's, both ways, keeping every shape guatiao has
//! and Lua does not: absent and null, a number's exact text, bytes apart
//! from text, a list apart from a map, and a map's key order.

use alloc::collections::BTreeSet;

use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{Buffer, Number};
use guatiao::{List, Map, Tag, Value};
use guatiao_bridge::Convert;
use mlua::prelude::*;
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};

/// Trees nest at most this deep on their way across.
const DEPTH: usize = 512;

/// `absent` inside a table, where `nil` cannot be stored.
#[derive(Debug, Clone, Copy)]
pub struct Absent;

impl UserData for Absent {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.absent");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("guatiao.absent"));
    }
}

/// A number whose exact text no Lua number holds (`1.10`, a 40-digit
/// integer): `tostring` gives the text; arithmetic and comparison go
/// through its float.
#[derive(Debug, Clone)]
pub struct Exact(pub String);

impl Exact {
    fn float(&self) -> f64 {
        self.0.parse().unwrap_or(f64::NAN)
    }
}

/// A Lua number, or an exact one, as a float.
fn float_of(v: &LuaValue) -> LuaResult<f64> {
    match v {
        LuaValue::Integer(i) => Ok(*i as f64),
        LuaValue::Number(f) => Ok(*f),
        LuaValue::UserData(ud) if ud.is::<Exact>() => Ok(ud.borrow::<Exact>()?.float()),
        other => Err(LuaError::runtime(format!(
            "attempt to perform arithmetic on a {} value",
            other.type_name()
        ))),
    }
}

impl UserData for Exact {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.number");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.0.clone()));
        methods.add_method("float", |_, this, ()| Ok(this.float()));
        methods.add_meta_function(MetaMethod::Eq, |_, (a, b): (LuaValue, LuaValue)| {
            let text = |v: &LuaValue| match v {
                LuaValue::UserData(ud) => ud.borrow::<Exact>().ok().map(|e| e.0.clone()),
                _ => None,
            };
            Ok(match (text(&a), text(&b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            })
        });
        methods.add_meta_function(MetaMethod::Lt, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? < float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Le, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? <= float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Add, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? + float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Sub, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? - float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Mul, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? * float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Div, |_, (a, b): (LuaValue, LuaValue)| {
            Ok(float_of(&a)? / float_of(&b)?)
        });
        methods.add_meta_function(MetaMethod::Unm, |_, a: LuaValue| Ok(-float_of(&a)?));
    }
}

/// Bytes in a tree, apart from text: `tostring` gives them as a Lua
/// string, `#` their length.
#[derive(Debug, Clone)]
pub struct Bytes(pub Vec<u8>);

impl UserData for Bytes {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.bytes");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            lua.create_string(&this.0)
        });
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| Ok(this.0.len()));
        methods.add_meta_function(MetaMethod::Eq, |_, (a, b): (LuaValue, LuaValue)| {
            let bytes = |v: &LuaValue| match v {
                LuaValue::UserData(ud) => ud.borrow::<Bytes>().ok().map(|b| b.0.clone()),
                _ => None,
            };
            Ok(matches!((bytes(&a), bytes(&b)), (Some(a), Some(b)) if a == b))
        });
    }
}

/// What a state keeps to tell shapes apart: the list and map metatables,
/// each map's key order (weak keys), and the absent sentinel.
#[derive(Clone)]
pub(crate) struct Shapes {
    pub(crate) list: LuaTable,
    pub(crate) map: LuaTable,
    order: LuaTable,
    pub(crate) absent: LuaAnyUserData,
}

/// The state's shapes, made once.
pub(crate) fn shapes(lua: &Lua) -> LuaResult<Shapes> {
    if let Some(shapes) = lua.app_data_ref::<Shapes>() {
        return Ok(shapes.clone());
    }
    let list = lua.create_table()?;
    list.raw_set("__name", "guatiao.list")?;
    let map = lua.create_table()?;
    map.raw_set("__name", "guatiao.map")?;
    let order = lua.create_table()?;
    let weak = lua.create_table()?;
    weak.raw_set("__mode", "k")?;
    order.set_metatable(Some(weak))?;
    let shapes = Shapes {
        list,
        map,
        order,
        absent: lua.create_userdata(Absent)?,
    };
    lua.set_app_data(shapes.clone());
    Ok(shapes)
}

/// Marks `table` a list.
pub(crate) fn mark_list(lua: &Lua, table: &LuaTable) -> LuaResult<()> {
    table.set_metatable(Some(shapes(lua)?.list))
}

/// Marks `table` a map.
pub(crate) fn mark_map(lua: &Lua, table: &LuaTable) -> LuaResult<()> {
    table.set_metatable(Some(shapes(lua)?.map))
}

fn refused(why: impl Into<String>) -> LuaError {
    LuaError::runtime(why.into())
}

/// An integer Lua holds exactly. Lua 5.3 on has a 64-bit integer; Luau
/// and LuaJIT hold integers in a double, exact to 2^53.
#[cfg(not(any(feature = "luau", feature = "luajit")))]
fn integer(i: i64) -> Option<LuaValue> {
    mlua::Integer::try_from(i).ok().map(LuaValue::Integer)
}

#[cfg(any(feature = "luau", feature = "luajit"))]
fn integer(i: i64) -> Option<LuaValue> {
    (i.unsigned_abs() <= 1 << 53).then_some(LuaValue::Number(i as f64))
}

/// A Lua integer widened: `i32` on 32-bit targets.
#[allow(clippy::useless_conversion)]
fn wide(i: mlua::Integer) -> i64 {
    i64::from(i)
}

/// A number's text as Lua's: an integer when its text is one Lua holds,
/// a float when the float's shortest text is its text, else exact.
pub(crate) fn number_to_lua(lua: &Lua, text: &str) -> LuaResult<LuaValue> {
    if let Ok(i) = text.parse::<i64>()
        && i.to_string() == text
        && let Some(v) = integer(i)
    {
        return Ok(v);
    }
    if let Ok(f) = text.parse::<f64>()
        && f.is_finite()
        && f.to_string() == text
    {
        return Ok(LuaValue::Number(f));
    }
    Ok(LuaValue::UserData(
        lua.create_userdata(Exact(text.to_string()))?,
    ))
}

/// `v` as Lua's; `absent` is `nil` at the top and the sentinel in a table.
pub(crate) fn to_lua(lua: &Lua, v: &Value) -> LuaResult<LuaValue> {
    to_lua_at(lua, v, 0)
}

fn to_lua_at(lua: &Lua, v: &Value, depth: usize) -> LuaResult<LuaValue> {
    if depth > DEPTH {
        return Err(refused(format!("a tree deeper than {DEPTH} levels")));
    }
    let tag = v
        .tag()
        .map_err(|e| refused(format!("a value guatiao cannot read: {e}")))?;
    Ok(match tag {
        Tag::GUATIAO_ABSENT if depth == 0 => LuaValue::Nil,
        Tag::GUATIAO_ABSENT => LuaValue::UserData(shapes(lua)?.absent),
        Tag::GUATIAO_NULL => LuaValue::NULL,
        Tag::GUATIAO_BOOL => {
            LuaValue::Boolean(TryAsRef::<bool>::try_as_ref(v).copied().unwrap_or(false))
        }
        Tag::GUATIAO_NUMBER => {
            let n: &Number =
                TryAsRef::try_as_ref(v).ok_or_else(|| refused("a malformed number"))?;
            number_to_lua(lua, n)?
        }
        Tag::GUATIAO_STRING => {
            let s: &str =
                TryAsRef::try_as_ref(v).ok_or_else(|| refused("text that is not UTF-8"))?;
            LuaValue::String(lua.create_string(s)?)
        }
        Tag::GUATIAO_BYTES => {
            let b: &[u8] = TryAsRef::try_as_ref(v).unwrap_or(&[]);
            LuaValue::UserData(lua.create_userdata(Bytes(b.to_vec()))?)
        }
        Tag::GUATIAO_LIST => {
            let list: &List = TryAsRef::try_as_ref(v).ok_or_else(|| refused("a malformed list"))?;
            LuaValue::Table(list_to_lua(lua, list, depth)?)
        }
        Tag::GUATIAO_MAP => {
            let map: &Map = TryAsRef::try_as_ref(v).ok_or_else(|| refused("a malformed map"))?;
            LuaValue::Table(map_to_lua(lua, map, depth)?)
        }
        _ => return Err(refused("a value of a kind this binding does not know")),
    })
}

/// A list as a table marked a list.
pub(crate) fn list_to_lua(lua: &Lua, list: &List, depth: usize) -> LuaResult<LuaTable> {
    let table = lua.create_table_with_capacity(list.len(), 0)?;
    for (i, item) in list.iter().enumerate() {
        table.raw_set(i + 1, to_lua_at(lua, item, depth + 1)?)?;
    }
    mark_list(lua, &table)?;
    Ok(table)
}

/// A map as a table marked a map, its key order kept.
pub(crate) fn map_to_lua(lua: &Lua, map: &Map, depth: usize) -> LuaResult<LuaTable> {
    let table = lua.create_table_with_capacity(0, map.len())?;
    let keys = lua.create_table_with_capacity(map.len(), 0)?;
    for (i, entry) in map.iter().enumerate() {
        table.raw_set(entry.key(), to_lua_at(lua, entry.value(), depth + 1)?)?;
        keys.raw_set(i + 1, entry.key())?;
    }
    let shapes = shapes(lua)?;
    table.set_metatable(Some(shapes.map))?;
    shapes.order.raw_set(&table, keys)?;
    Ok(table)
}

/// Which tree an empty plain table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Empty {
    Map,
    List,
}

/// A Lua value as guatiao's: `nil` is absent.
pub(crate) fn to_value(lua: &Lua, v: &LuaValue) -> LuaResult<Value> {
    to_value_as(lua, v, Empty::Map)
}

/// As [`to_value`], an empty plain table taken as `empty`.
pub(crate) fn to_value_as(lua: &Lua, v: &LuaValue, empty: Empty) -> LuaResult<Value> {
    let mut seen = BTreeSet::new();
    to_value_at(lua, v, empty, 0, &mut seen)
}

fn number_value(text: String) -> LuaResult<Value> {
    Ok(Value::from(Number::new(&text).map_err(|_| {
        refused(format!("`{text}` is not a number"))
    })?))
}

/// A Lua float's value: its shortest text; NaN and the infinities have
/// none.
pub(crate) fn float_value(f: f64) -> LuaResult<Value> {
    Value::try_from(f).map_err(|_| refused("a number that is not finite has no value to cross as"))
}

fn to_value_at(
    lua: &Lua,
    v: &LuaValue,
    empty: Empty,
    depth: usize,
    seen: &mut BTreeSet<usize>,
) -> LuaResult<Value> {
    if depth > DEPTH {
        return Err(refused(format!(
            "a table nested deeper than {DEPTH} levels"
        )));
    }
    Ok(match v {
        LuaValue::Nil => Value::absent(),
        LuaValue::Boolean(b) => Value::from(*b),
        LuaValue::LightUserData(p) if p.0.is_null() => Value::null(),
        LuaValue::Integer(i) => Value::from(wide(*i)),
        LuaValue::Number(f) => float_value(*f)?,
        LuaValue::String(s) => match s.to_str() {
            Ok(text) => Value::from(&*text),
            Err(_) => Value::from(Buffer::from(&*s.as_bytes())),
        },
        LuaValue::UserData(ud) if ud.is::<Absent>() => Value::absent(),
        LuaValue::UserData(ud) if ud.is::<Exact>() => {
            number_value(ud.borrow::<Exact>()?.0.clone())?
        }
        LuaValue::UserData(ud) if ud.is::<Bytes>() => {
            Value::from(Buffer::from(&ud.borrow::<Bytes>()?.0[..]))
        }
        LuaValue::Table(t) => {
            let at = t.to_pointer() as usize;
            if !seen.insert(at) {
                return Err(refused(
                    "a table that contains itself has no value to cross as",
                ));
            }
            let value = table_to_value(lua, t, empty, depth, seen)?;
            seen.remove(&at);
            value
        }
        other => {
            return Err(refused(format!(
                "a {} has no value to cross as",
                other.type_name()
            )));
        }
    })
}

fn table_to_value(
    lua: &Lua,
    t: &LuaTable,
    empty: Empty,
    depth: usize,
    seen: &mut BTreeSet<usize>,
) -> LuaResult<Value> {
    let shapes = shapes(lua)?;
    let marked = t.metatable();
    let is = |mt: &LuaTable| marked.as_ref().is_some_and(|m| m == mt);
    if is(&shapes.list) {
        return list_from(lua, t, depth, seen);
    }
    if is(&shapes.map) {
        return map_from(lua, t, &shapes, depth, seen);
    }
    // A plain table: what its keys say.
    let mut count = 0usize;
    let mut all_index = true;
    let mut all_text = true;
    let mut largest = 0i64;
    let mut odd_key = None;
    for pair in t.pairs::<LuaValue, LuaValue>() {
        let (k, _) = pair?;
        count += 1;
        match &k {
            LuaValue::Integer(i) if *i >= 1 => {
                all_text = false;
                largest = largest.max(wide(*i));
            }
            LuaValue::String(_) => all_index = false,
            other => {
                all_index = false;
                all_text = false;
                odd_key.get_or_insert_with(|| other.type_name().to_string());
            }
        }
    }
    if count == 0 {
        return Ok(match empty {
            Empty::Map => Value::from(Map::new()),
            Empty::List => Value::from(List::new()),
        });
    }
    if all_index && usize::try_from(largest).ok() == Some(count) {
        return list_from(lua, t, depth, seen);
    }
    if all_text {
        return map_from(lua, t, &shapes, depth, seen);
    }
    Err(refused(match odd_key {
        Some(kind) => format!("a table keyed by a {kind} has no value to cross as"),
        None => "a table with both text keys and indices, or indices with holes, is neither a map nor a list".to_string(),
    }))
}

fn list_from(
    lua: &Lua,
    t: &LuaTable,
    depth: usize,
    seen: &mut BTreeSet<usize>,
) -> LuaResult<Value> {
    let n = t.raw_len();
    let mut list = List::new();
    for i in 1..=n {
        let item: LuaValue = t.raw_get(i)?;
        if item.is_nil() {
            return Err(refused(format!(
                "a list with a hole at {i}: use guatiao.absent or guatiao.null"
            )));
        }
        list.push(to_value_at(lua, &item, Empty::Map, depth + 1, seen)?);
    }
    Ok(Value::from(list))
}

fn map_from(
    lua: &Lua,
    t: &LuaTable,
    shapes: &Shapes,
    depth: usize,
    seen: &mut BTreeSet<usize>,
) -> LuaResult<Value> {
    let mut keys: Vec<String> = Vec::new();
    // The order it came with, then keys added since, in byte order.
    if let Ok(LuaValue::Table(order)) = shapes.order.raw_get::<LuaValue>(t) {
        for key in order.sequence_values::<String>() {
            let key = key?;
            if !t.raw_get::<LuaValue>(key.as_str())?.is_nil() {
                keys.push(key);
            }
        }
    }
    let mut added = Vec::new();
    for pair in t.pairs::<LuaValue, LuaValue>() {
        let (k, _) = pair?;
        let LuaValue::String(k) = k else {
            return Err(refused(format!(
                "a map keyed by a {} has no value to cross as",
                k.type_name()
            )));
        };
        let k = k
            .to_str()
            .map_err(|_| refused("a map key that is not UTF-8"))?
            .to_string();
        if !keys.contains(&k) {
            added.push(k);
        }
    }
    added.sort();
    keys.extend(added);
    let mut map = Map::new();
    for key in keys {
        let item: LuaValue = t.raw_get(key.as_str())?;
        map.set(&key, to_value_at(lua, &item, Empty::Map, depth + 1, seen)?);
    }
    Ok(Value::from(map))
}

/// Writes `v` over the contents of `table`, keeping the table: how a lent
/// tree a callee changed comes back to the Lua code that lent it.
pub(crate) fn refill(lua: &Lua, table: &LuaTable, v: &Value) -> LuaResult<()> {
    let fresh = match to_lua(lua, v)? {
        LuaValue::Table(t) => t,
        _ => return Err(refused("a lent tree came back as something else")),
    };
    table.clear()?;
    for pair in fresh.pairs::<LuaValue, LuaValue>() {
        let (k, item) = pair?;
        table.raw_set(k, item)?;
    }
    let shapes = shapes(lua)?;
    table.set_metatable(fresh.metatable())?;
    let order: LuaValue = shapes.order.raw_get(&fresh)?;
    shapes.order.raw_set(table, order)?;
    Ok(())
}

/// guatiao's values as a Lua state's: the conversion the shared vectors
/// check.
#[derive(Debug, Clone)]
pub struct LuaValues {
    lua: Lua,
}

impl LuaValues {
    /// Conversions in `lua`.
    pub fn new(lua: &Lua) -> LuaValues {
        LuaValues { lua: lua.clone() }
    }
}

impl Convert for LuaValues {
    type Native<'a> = LuaValue;
    type Error = LuaError;

    fn to_native(&self, value: &Value) -> Result<LuaValue, LuaError> {
        to_lua(&self.lua, value)
    }

    fn to_value(&self, native: LuaValue) -> Result<Value, LuaError> {
        to_value(&self.lua, &native)
    }
}
