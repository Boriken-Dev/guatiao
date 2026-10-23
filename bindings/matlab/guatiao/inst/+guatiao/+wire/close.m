% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function frame = close(channel)
% The frame that ends `channel`, a uint8 row: a receiver forgets the
% channel's schema.
  frame = guatiao_mex('wire_frame', 2, channel);
end
