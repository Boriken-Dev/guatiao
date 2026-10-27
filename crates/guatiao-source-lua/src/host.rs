// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `guatiao` table Lua code requires: the values' sentinels and
//! constructors, refusals, statuses and kinds, and, for a Lua host, a
//! registry.

use alloc::sync::Arc;
use std::path::Path;

use guatiao::Map;
use guatiao::library::kind::DescribedRemote;
use guatiao::library::{Refreshed, Registry};
use guatiao_bridge::Held;
use mlua::prelude::*;
use mlua::{UserData, UserDataMethods};

use crate::calls::{self, BufferView, Handle, KindView, Refusal, STATUSES};
use crate::values::{self, Bytes, Exact};

/// The helpers every Lua side has: what a library written in Lua
/// requires as `guatiao`.
///
/// | name | is |
/// | --- | --- |
/// | `null`, `absent` | the sentinels for guatiao's null, and absent inside a table |
/// | `list(t)`, `map(t)` | `t` (or a new table) marked a list or a map |
/// | `bytes(s)` | `s` as bytes rather than text |
/// | `number(x)` | an exact number from its text, or a Lua number |
/// | `buffer(n)` | `n` zeroed bytes for a slot that writes into a buffer |
/// | `error(status, words)` | a refusal: raise it, or return `nil, it` |
/// | `status` | every status by name: `status.bad_value` is 7 |
/// | `kinds(source)` | loads kind descriptions (a path or JSON text); answers every kind known, by name |
/// | `kind(name)` | one kind known |
pub fn helpers(lua: &Lua) -> LuaResult<LuaTable> {
    let shapes = values::shapes(lua)?;
    let t = lua.create_table()?;
    t.set("null", LuaValue::NULL)?;
    t.set("absent", shapes.absent.clone())?;
    t.set(
        "list",
        lua.create_function(|lua, t: Option<LuaTable>| {
            let t = match t {
                Some(t) => t,
                None => lua.create_table()?,
            };
            values::mark_list(lua, &t)?;
            Ok(t)
        })?,
    )?;
    t.set(
        "map",
        lua.create_function(|lua, t: Option<LuaTable>| {
            let t = match t {
                Some(t) => t,
                None => lua.create_table()?,
            };
            values::mark_map(lua, &t)?;
            Ok(t)
        })?,
    )?;
    t.set(
        "bytes",
        lua.create_function(|_, s: LuaString| Ok(Bytes(s.as_bytes().to_vec())))?,
    )?;
    t.set(
        "number",
        lua.create_function(|lua, x: LuaValue| match x {
            LuaValue::Integer(_) | LuaValue::Number(_) => Ok(x),
            LuaValue::String(s) => {
                let text = s.to_str()?.to_string();
                if !guatiao::value::types::Number::is_number(&text) {
                    return Err(LuaError::runtime(format!("`{text}` is not a number")));
                }
                values::number_to_lua(lua, &text)
            }
            LuaValue::UserData(ud) if ud.is::<Exact>() => Ok(LuaValue::UserData(ud)),
            other => Err(LuaError::runtime(format!(
                "a number is made from text or a number, not a {}",
                other.type_name()
            ))),
        })?,
    )?;
    t.set(
        "buffer",
        lua.create_function(|_, n: usize| Ok(BufferView(vec![0; n])))?,
    )?;
    t.set(
        "error",
        lua.create_function(|_, (status, message): (LuaValue, Option<String>)| {
            Ok(Refusal {
                status: calls::status_of(&status)?,
                message: message.unwrap_or_default(),
            })
        })?,
    )?;
    let statuses = lua.create_table()?;
    for (name, status) in STATUSES {
        statuses.set(name, u32::from(status))?;
    }
    t.set("status", statuses)?;
    t.set(
        "kinds",
        lua.create_function(|lua, source: String| {
            {
                let mut known = calls::known(lua)?;
                let loaded = if source.trim_start().starts_with('{') {
                    known.kinds.add_json(&source)
                } else {
                    known.kinds.load(Path::new(&source))
                };
                loaded.map_err(|e| LuaError::runtime(e.to_string()))?;
            }
            all_kinds(lua)
        })?,
    )?;
    t.set(
        "kind",
        lua.create_function(|lua, name: String| Ok(KindView(calls::kind(lua, &name)?)))?,
    )?;
    t.set("_VERSION", env!("CARGO_PKG_VERSION"))?;
    Ok(t)
}

/// Every kind the state knows, by name.
fn all_kinds(lua: &Lua) -> LuaResult<LuaTable> {
    let known = calls::known(lua)?;
    let out = lua.create_table()?;
    for name in known.kinds.names() {
        if let Some(k) = known.kinds.get(name) {
            out.set(name, KindView(Arc::clone(k)))?;
        }
    }
    Ok(out)
}

/// The helpers and a registry: what a Lua host requires as `guatiao`.
///
/// `registry(id, version)` answers a registry whose methods are
/// `load_file(path)`, `add_source_file(path, config)`, `refresh()`,
/// `providers([kind])`, `bind(key, kind)`, `retire(key)`, `collect()`
/// and `close()`.
pub fn module(lua: &Lua) -> LuaResult<LuaTable> {
    let t = helpers(lua)?;
    t.set(
        "registry",
        lua.create_function(|_, (id, version): (String, String)| {
            Ok(RegistryView(Some(Registry::new(&id, &version))))
        })?,
    )?;
    Ok(t)
}

/// A registry, held by Lua code: closed by `close()` or when collected.
pub struct RegistryView(Option<Registry>);

impl RegistryView {
    fn get(&mut self) -> LuaResult<&mut Registry> {
        self.0
            .as_mut()
            .ok_or_else(|| LuaError::runtime("the registry is closed"))
    }
}

fn refreshed(lua: &Lua, r: &Refreshed) -> LuaResult<LuaTable> {
    let served = |list: &[guatiao::library::Served]| -> LuaResult<LuaTable> {
        let t = lua.create_table()?;
        for s in list {
            let one = lua.create_table()?;
            one.set("source", s.source.as_str())?;
            one.set("id", s.id.as_str())?;
            one.set("stamp", s.stamp.as_str())?;
            one.set("key", s.key.as_str())?;
            t.raw_push(one)?;
        }
        Ok(t)
    };
    let out = lua.create_table()?;
    out.set("loaded", served(&r.loaded)?)?;
    out.set("retired", served(&r.retired)?)?;
    let skipped = lua.create_table()?;
    for (source, id, why) in &r.skipped {
        let one = lua.create_table()?;
        one.set("source", source.as_str())?;
        one.set("id", id.as_str())?;
        one.set("why", why.to_string())?;
        skipped.raw_push(one)?;
    }
    out.set("skipped", skipped)?;
    let refused = lua.create_table()?;
    for r in &r.refused {
        let one = lua.create_table()?;
        one.set("source", r.source.as_str())?;
        one.set("id", r.id.as_deref())?;
        one.set("reason", r.reason.as_str())?;
        refused.raw_push(one)?;
    }
    out.set("refused", refused)?;
    Ok(out)
}

impl UserData for RegistryView {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("load_file", |lua, this, path: String| {
            let mut out = LuaMultiValue::new();
            match this.get()?.load_file(Path::new(&path)) {
                Ok(guatiao::library::Loading::Loaded(loaded)) => {
                    let t = lua.create_table()?;
                    t.set("id", loaded.id.as_str())?;
                    t.set("version", loaded.version.as_str())?;
                    t.set("key", &*loaded.key)?;
                    t.set("providers", loaded.providers)?;
                    out.push_back(LuaValue::Table(t));
                }
                Ok(guatiao::library::Loading::Skipped(why)) => {
                    out.push_back(LuaValue::Nil);
                    out.push_back(LuaValue::String(lua.create_string(why.to_string())?));
                }
                Err(e) => {
                    out.push_back(LuaValue::Nil);
                    out.push_back(LuaValue::String(lua.create_string(e.to_string())?));
                }
            }
            Ok(out)
        });
        methods.add_method_mut(
            "add_source_file",
            |lua, this, (path, config): (String, Option<LuaValue>)| {
                let config = match config {
                    Some(c) => Map::try_from(values::to_value(lua, &c)?)
                        .map_err(|_| LuaError::runtime("a source's configuration is a map"))?,
                    None => Map::new(),
                };
                this.get()?
                    .add_source_file(Path::new(&path), &config)
                    .map_err(|e| LuaError::runtime(e.to_string()))
            },
        );
        methods.add_method_mut("refresh", |lua, this, ()| {
            let r = this.get()?.refresh();
            refreshed(lua, &r)
        });
        methods.add_method_mut("providers", |lua, this, kind: Option<String>| {
            let registry = this.get()?;
            let out = lua.create_table()?;
            let all: Vec<&guatiao::library::Provider> = match &kind {
                Some(k) => registry.providers(k).collect(),
                None => registry.all().iter().collect(),
            };
            for p in all {
                let one = lua.create_table()?;
                one.set("key", p.key())?;
                one.set("id", p.id())?;
                one.set("name", p.display_name())?;
                one.set("library", p.library())?;
                one.set("version", p.version())?;
                one.set("configured", p.config_schema().is_some())?;
                one.set(
                    "kinds",
                    lua.create_sequence_from(p.kinds().iter().map(String::as_str))?,
                )?;
                out.raw_push(one)?;
            }
            Ok(out)
        });
        methods.add_method_mut("bind", |lua, this, (key, kind): (String, LuaValue)| {
            let described = calls::kind_named(lua, &kind)?;
            let caller = calls::caller(lua, described.name())?;
            let registry = this.get()?;
            let provider = registry
                .provider(&key)
                .ok_or_else(|| LuaError::runtime(format!("no provider `{key}`")))?;
            let remote =
                DescribedRemote::new(provider.view(), Arc::clone(&caller)).map_err(|e| {
                    LuaError::runtime(format!("`{key}` as `{}`: {e}", described.name()))
                })?;
            Ok(Handle {
                held: Held::Provider(remote),
                caller,
            })
        });
        methods.add_method_mut("retire", |_, this, key: String| {
            this.get()?
                .retire(&key)
                .map(|_| true)
                .map_err(|e| LuaError::runtime(e.to_string()))
        });
        methods.add_method_mut("collect", |lua, this, ()| {
            lua.create_sequence_from(this.get()?.collect())
        });
        methods.add_method_mut("close", |_, this, ()| {
            this.0 = None;
            Ok(())
        });
    }
}
