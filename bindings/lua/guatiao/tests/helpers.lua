-- This Source Code Form is subject to the terms of the Mozilla Public
-- License, v. 2.0. If a copy of the MPL was not distributed with this
-- file, You can obtain one at https://mozilla.org/MPL/2.0/.

-- The module's helpers, from a stock interpreter: `lua helpers.lua KINDS`.

local guatiao = require "guatiao"

assert(guatiao.null ~= nil and guatiao.absent ~= nil)
assert(tostring(guatiao.absent) == "guatiao.absent")
assert(getmetatable(guatiao.list({})).__name == "guatiao.list")
assert(getmetatable(guatiao.map()).__name == "guatiao.map")

local exact = guatiao.number("1.10")
assert(tostring(exact) == "1.10" and type(exact) == "userdata")
assert(math.abs((exact + 1) - 2.1) < 1e-12)
assert(guatiao.number("2") == 2)
assert(not pcall(guatiao.number, "1,5"))

local b = guatiao.bytes("\0\255")
assert(#b == 2 and tostring(b) == "\0\255")

local buf = guatiao.buffer(4)
assert(buf:write("hello") == 4 and buf:sub(1, 4) == "hell")

local e = guatiao.error("not_found", "no such greeting")
assert(e.status == "not_found" and e.code == 10 and tostring(e) == "no such greeting")
assert(guatiao.status.bad_value == 7)

local kinds = guatiao.kinds(assert(arg[1]))
assert(kinds.greeter.name == "greeter" and not kinds.greeter.object)
assert(kinds.listener.object)
assert(guatiao.kind("counter").slots[1] == "count")

local registry = guatiao.registry("helpers", "1.0")
local refreshed = registry:refresh()
assert(#refreshed.loaded == 0 and #refreshed.refused == 0)
local nothing, why = registry:load_file(arg[1])
assert(nothing == nil and type(why) == "string")
registry:close()
assert(not pcall(registry.refresh, registry))

print("helpers: all checks passed")
