% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function s = taken(shape)
% A shape handed over with the call: 'value', 'map', 'list' (a tree the
% callee owns afterwards) or an object kind's class (an object the callee
% holds until it destroys it).
  s = struct('wrap', 'taken', 'shape', shape);
end
