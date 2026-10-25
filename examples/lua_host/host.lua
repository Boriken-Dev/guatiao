-- This Source Code Form is subject to the terms of the Mozilla Public
-- License, v. 2.0. If a copy of the MPL was not distributed with this
-- file, You can obtain one at https://mozilla.org/MPL/2.0/.

-- A host written in Lua: it loads greeter libraries and sources, whatever
-- language wrote them, and calls every greeter through the `greeter`
-- kind, as its description says.
--
--   lua host.lua KINDS [--library FILE]... [--source FILE KEY=A;B ...]...
--
-- KINDS is a kind description document (greeter_kind.kinds.json). A
-- source's configuration follows its file: each KEY=A;B is a list of
-- text under KEY. It raises an error on the first check that fails, and
-- prints "all checks passed" at the end.

local guatiao = require "guatiao"

local args = arg or {}
local kinds = guatiao.kinds(assert(args[1], "usage: host.lua KINDS [--library FILE]... [--source FILE KEY=A;B ...]..."))
local greeter = kinds.greeter

local registry = guatiao.registry("lua-host", "1.0")

local i = 2
while args[i] do
  local what = args[i]
  if what == "--library" then
    local loaded, why = registry:load_file(args[i + 1])
    if loaded then
      print(("loaded %s %s"):format(loaded.id, loaded.version))
    else
      print(("passed over %s: %s"):format(args[i + 1], why))
    end
    i = i + 2
  elseif what == "--source" then
    local file = args[i + 1]
    local config = {}
    i = i + 2
    while args[i] and args[i]:find("=", 1, true) and not args[i]:find("^%-%-") do
      local key, list = args[i]:match("^([^=]+)=(.*)$")
      local items = guatiao.list()
      for item in list:gmatch("[^;]+") do
        items[#items + 1] = item
      end
      config[key] = items
      i = i + 1
    end
    print(("added the source %s"):format(registry:add_source_file(file, config)))
  else
    error("unknown argument " .. what)
  end
end

local changed = registry:refresh()
for _, served in ipairs(changed.loaded) do
  print(("served %s from %s"):format(served.id, served.source))
end
for _, refused in ipairs(changed.refused) do
  error(("refused %s: %s"):format(tostring(refused.id), refused.reason))
end

local function check(ok, what)
  if not ok then
    error("check failed: " .. what, 2)
  end
end

-- A listener the greeters tell things to: a table of the kind's methods.
local function listener()
  local heard = {}
  return { heard = function(self, what) heard[#heard + 1] = what end }, heard
end

-- An optional slot a greeter leaves out answers `unsupported`.
local function offered(answer, err)
  if answer == nil and err and err.status == "unsupported" then
    return false
  end
  return true
end

-- A greeter that runs on a thread of its own (a Lua library, through the
-- guatiao-source-lua source) cannot call a Lua table back: the table is called
-- on this interpreter's thread only, which is waiting for the answer.
local function elsewhere(answer, err)
  return answer == nil and err ~= nil and err.status == "internal"
end

local called = 0
for _, p in ipairs(registry:providers("greeter")) do
  if p.configured then
    print(("%s is built from a configuration: not called here"):format(p.key))
  else
    local g = registry:bind(p.key, greeter)
    local answer = assert(g:greet("ana"))
    print(("%s greets: %s"):format(p.key, answer.greeting))
    check(answer.greeting:find("ana", 1, true), p.key .. " greets ana")

    local nothing, why = g:greet("")
    check(nothing == nil and why.status == "bad_value", p.key .. " refuses an empty name")
    print(("%s refuses an empty name: %s"):format(p.key, tostring(why)))

    local config = { name = "bo" }
    local checked, err = g:check(config)
    if offered(checked, err) then
      check(checked ~= nil, p.key .. " checks: " .. tostring(err))
      print(("%s checked a lent map, which now holds %d keys"):format(p.key, (function()
        local n = 0
        for _ in pairs(config) do n = n + 1 end
        return n
      end)()))
    end

    local from, ferr = g:greet_from({ name = "cy" })
    if offered(from, ferr) then
      check(from and from.greeting:find("cy", 1, true), p.key .. " greets from a map: " .. tostring(ferr))
    end

    local ear, heard = listener()
    local told, terr = g:greet_into("di", ear)
    if elsewhere(told, terr) then
      print(("%s cannot call a Lua table back from its thread"):format(p.key))
    elseif offered(told, terr) then
      check(told and heard[1] and heard[1]:find("di", 1, true), p.key .. " tells a lent listener: " .. tostring(terr))
      print(("%s told a lent listener: %s"):format(p.key, heard[1]))
    end

    local talker, said = listener()
    local talk, serr = g:start(talker)
    if offered(talk, serr) and talk then
      local first, ferr2 = talk:say("one")
      if not elsewhere(first, ferr2) then
        check(first, p.key .. " says: " .. tostring(ferr2))
        assert(talk:say("two"))
        check(talk:turns() == 2, p.key .. " counts turns")
        local buf = guatiao.buffer(64)
        local n = talk:read(buf)
        print(("%s held a conversation: %s"):format(p.key, buf:sub(1, n)))
        check(said[2] == "two", p.key .. " told the handed listener")
      end
    end
    called = called + 1
  end
end
check(called > 0, "some greeter was called")
registry:close()
print("all checks passed")
