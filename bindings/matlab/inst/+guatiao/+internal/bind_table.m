% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function obj = bind_table(cls, t, ctx, holder)
% A remote of the kind class `cls` over a table struct(table, size, ctx),
% refused when the table is smaller than the kind's required slots or
% carries another floor hash (guatiao:WrongKind:floor). `ctx` replaces the
% table's when given.
  e = guatiao.internal.kinds('of', cls);
  if isempty(t.table)
    error('guatiao:WrongKind', 'the provider does not serve %s through a table', e.desc.name);
  end
  hash = guatiao_mex('table_floor_hash', t.table);
  if t.size < e.floor_size || hash ~= e.desc.floor_hash
    error('guatiao:WrongKind:floor', ['the table is not the %s kind''s: size %d (at least %d), ' ...
          'floor hash %d (want %d)'], e.desc.name, t.size, e.floor_size, hash, e.desc.floor_hash);
  end
  if isempty(ctx)
    ctx = t.ctx;
  end
  obj = guatiao.internal.remote_of(cls, guatiao_mex('remote_new', e.cid, t.table, t.size, ctx, false), holder);
end
