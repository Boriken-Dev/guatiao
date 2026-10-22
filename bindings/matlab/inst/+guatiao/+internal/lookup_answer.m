% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = lookup_answer(arg, r)
% What a lookup into `arg` answers, from the gateway's {kind, id, path}: a
% view of the argument's own node, a new value (a copy) when the argument
% was MATLAB data, or [] when nothing is there.
  if isempty(r)
    out = [];
  elseif strcmp(r{1}, 'view')
    out = arg.view_of(r{3});
  else
    out = guatiao.Value.adopt(r{2});
  end
end
