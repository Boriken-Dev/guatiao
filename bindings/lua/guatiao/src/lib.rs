// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `require "guatiao"`: guatiao's values, kinds and a registry for a Lua
//! interpreter. Built against the Lua that loads it; the guatiao library
//! is inside, so nothing else needs to be found.

use mlua::prelude::*;

/// `luaopen_guatiao`: the table `guatiao_lua::module` answers.
#[mlua::lua_module(name = "guatiao")]
fn open(lua: &Lua) -> LuaResult<LuaTable> {
    guatiao_lua::module(lua)
}
