-- This Source Code Form is subject to the terms of the Mozilla Public
-- License, v. 2.0. If a copy of the MPL was not distributed with this
-- file, You can obtain one at https://mozilla.org/MPL/2.0/.

-- `luarocks make` in this directory builds the module against the Lua
-- LuaRocks runs for, with cargo; nothing prebuilt is fetched.
rockspec_format = "3.0"
package = "guatiao"
version = "dev-1"

source = {
   url = "git+https://github.com/Boriken-Dev/guatiao",
   dir = "guatiao/bindings/lua/guatiao",
}

description = {
   summary = "One contract for passing configuration and providers between languages",
   detailed = [[
guatiao's values, kinds and registry from Lua: load libraries written in
any language with a guatiao binding, and call their providers through a
kind's description.]],
   homepage = "https://boriken-dev.github.io/guatiao/",
   license = "MPL-2.0",
}

dependencies = {
   "lua >= 5.1",
   "luarocks-build-rust-mlua",
}

-- rust-mlua turns on the feature for the Lua LuaRocks runs for (lua54,
-- lua55, luajit) and copies `libguatiao_lua_module.so` (`.dylib`) in as
-- `guatiao`. On Windows it looks for `guatiao.dll` instead, which this
-- crate does not write: build with cargo there and copy the file.
build = {
   type = "rust-mlua",
   default_features = false,
   modules = {
      ["guatiao"] = "guatiao_lua_module",
   },
}
