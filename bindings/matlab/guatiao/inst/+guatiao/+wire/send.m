% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function frame = send(channel, value)
% The frame that carries `value` (a guatiao.Value, a view or MATLAB data)
% on `channel`, a uint8 row.
  frame = guatiao_mex('wire_frame', 1, channel, value);
end
