% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function node = at(value, path)
% The node at a path in the intake grammar ('agent[1].name', positions
% from 0): a view of the argument's own node when it is a guatiao value or
% view, a copy (a new guatiao.Value) when it is MATLAB data, or [] when
% nothing is there.
  node = guatiao.internal.lookup_answer(value, guatiao_mex('intake_at', value, path));
end
