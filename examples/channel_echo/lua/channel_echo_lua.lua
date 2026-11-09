-- This Source Code Form is subject to the terms of the Mozilla Public
-- License, v. 2.0. If a copy of the MPL was not distributed with this
-- file, You can obtain one at https://mozilla.org/MPL/2.0/.

-- An echo serving a channel from Lua: every message sent comes back
-- through the host's sink, in order, inside the `send` that took it. A
-- Lua object is called on its interpreter's thread only, so this echo has
-- no thread of its own to push from.

local guatiao = require "guatiao"

local M = { _VERSION = "1.0.0" }

local Channel = {}
Channel.__index = Channel

-- Ends the stream once: the sink hears `closed(status, reason)`, then nothing.
function Channel:finish(status, reason)
  if self.sink == nil then
    return
  end
  local sink = self.sink
  self.sink = nil
  sink:closed(status, reason)
  -- The handle is released when it is collected: let the sink go now.
  sink = nil
  collectgarbage()
end

function Channel:send(bytes)
  if self.sink == nil then
    return nil, guatiao.error("gone", "the channel is closed")
  end
  local _, err = self.sink:receive(bytes)
  if err ~= nil then
    -- Any refusal ends the stream; the message was taken all the same.
    self:finish(err.code, "")
    return
  end
  self.echoed = self.echoed + 1
  if self.close_after ~= nil and self.echoed >= self.close_after then
    self:finish(self.close_status, self.close_reason)
  end
end

function Channel:close(status, reason)
  self:finish(status, reason)
end

-- Dropping the channel is `close(0, "")`: its table is collected when the
-- host destroys it.
Channel.__gc = function(self)
  pcall(self.finish, self, 0, "")
end

local Echo = { name = "Echo, in Lua", kinds = { "channel_echo" } }

function Echo:start(config, sink)
  return setmetatable({
    sink = sink,
    echoed = 0,
    close_after = config.close_after,
    close_status = config.close_status or 0,
    close_reason = config.close_reason or "",
  }, Channel)
end

M.providers = { echo = Echo }

return M
