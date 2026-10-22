% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function s = lent(shape)
% A shape the caller lends for the call: 'value', 'map', 'list' (a tree
% changed in place), 'bytes' (a guatiao.Buffer written in place) or an
% object kind's class (an object the callee uses during the call).
  s = struct('wrap', 'lent', 'shape', shape);
end
