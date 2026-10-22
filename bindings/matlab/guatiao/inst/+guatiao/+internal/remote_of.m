% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function obj = remote_of(cls, remote_id, holder)
% An instance of the kind class `cls` bound to the gateway's remote.
  obj = feval(cls);
  if ~isa(obj, 'guatiao.Kind')
    guatiao_mex('remote_close', remote_id);
    error('guatiao:WrongKind', '%s is not a kind class (it does not inherit guatiao.Kind)', cls);
  end
  obj.attach(remote_id, holder);
end
