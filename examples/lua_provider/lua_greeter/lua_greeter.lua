-- This Source Code Form is subject to the terms of the Mozilla Public
-- License, v. 2.0. If a copy of the MPL was not distributed with this
-- file, You can obtain one at https://mozilla.org/MPL/2.0/.

-- Greeters written in Lua: the same kinds the Rust, C and Python greeters
-- serve, offered to any host through the guatiao-source-lua source.
--
-- The module is the library: its id is its name, its version `_VERSION`,
-- its providers the tables under `providers`, each a table of methods
-- named for the kinds' slots. `lua_greeter.guatiao-lua.toml` beside it is
-- what the source reads before running anything.

local guatiao = require "guatiao"

local M = { _VERSION = "1.0.0" }

-- Changed by the examples to show a refresh serving new code.
local GREETING = "hello"

local function greeting(name)
  if name == "" then
    return nil, "nobody to greet"
  end
  return { greeting = GREETING .. ", " .. name }
end

-- One conversation `start` hands back: the listener it was handed, and
-- what was said.
local Chat = {}
Chat.__index = Chat

function Chat:say(what)
  if what == "" then
    return nil, "nothing to say"
  end
  self.said[#self.said + 1] = what
  self.listener:heard(what)
end

function Chat:read(dst)
  return dst:write(table.concat(self.said, " "))
end

function Chat:turns()
  return #self.said
end

-- One provider serving two kinds, keeping its count in itself.
local Hello = { name = "Hello, in Lua", kinds = { "greeter", "counter" }, asked = 0 }

function Hello:greet(name)
  if name == "panic" then
    error("a provider bug, on request")
  end
  return greeting(name)
end

function Hello:check(config)
  if config.name == nil then
    config.name = "ana"
  end
  config.checked_by = "Lua"
  return true
end

function Hello:greet_from(config)
  return greeting(config.name or "")
end

function Hello:greet_into(name, listener)
  local answer, why = greeting(name)
  if not answer then
    return nil, why
  end
  listener:heard(answer.greeting)
end

function Hello:start(listener)
  return setmetatable({ listener = listener, said = {} }, Chat)
end

function Hello:count()
  self.asked = self.asked + 1
  return self.asked
end

-- A class: its methods reached through a metatable, `shout` left to the
-- kind's default.
local Polite = { name = "Polite, in Lua", kinds = { "greeter" } }
Polite.__index = Polite

function Polite:greet(name)
  if name == "" then
    return nil, guatiao.error("bad_value", "nobody to greet, politely")
  end
  return { greeting = "good day, " .. name }
end

M.providers = {
  hello = Hello,
  polite = setmetatable({}, Polite),
}

return M
