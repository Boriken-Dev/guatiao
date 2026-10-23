% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function frame = announce(channel, schema)
% The frame that announces `schema` (a guatiao.Value, a view or MATLAB
% data) on `channel` (a whole number; a uint64 past 2^53), a uint8 row. A
% later announcement on the channel replaces it.
  frame = guatiao_mex('wire_frame', 0, channel, schema);
end
