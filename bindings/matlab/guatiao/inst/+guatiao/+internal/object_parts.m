% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [parts, id] = object_parts(kind_cls, x)
% What a C caller is given for an object of an object kind: a remote of the
% kind (as it is), or an M object served by a table of the kind, held
% under `id` until its holder destroys it.
  id = [];
  if isa(x, 'guatiao.Kind') && x.is_bound()
    parts = struct('remote', x.RemoteId);
    return
  end
  if ~isa(x, kind_cls)
    error('guatiao:WrongKind', 'an object of the %s kind is an object of that class or a remote of it', kind_cls);
  end
  e = guatiao.internal.kinds('of', kind_cls);
  table = guatiao.internal.served_table(kind_cls, class(x));
  id = guatiao.internal.held('hold', x);
  parts = struct('remote', [], 'table', table, 'size', e.size, 'ctx', 65536 + 16 * id);
end
