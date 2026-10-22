% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function table = served_table(kind_cls, cls, refuse_missing)
% The table serving `kind_cls` for objects of `cls`: a slot for each of the
% kind's methods the class overrides, null for the rest; built once a
% session (a host may hold it). A required slot the class leaves alone is
% refused.
  persistent tables
  if isempty(tables)
    tables = containers.Map('KeyType', 'char', 'ValueType', 'any');
  end
  if nargin < 3
    refuse_missing = true;
  end
  key = [kind_cls '|' cls];
  if isKey(tables, key)
    table = tables(key);
    return
  end
  e = guatiao.internal.kinds('of', kind_cls);
  has = guatiao.internal.overrides(cls, kind_cls, e.names);
  required = cellfun(@(s) s.required, e.desc.slots);
  if refuse_missing && any(required & ~has)
    error('guatiao:BadValue', '%s does not implement %s, which the %s kind requires', cls, ...
          strjoin(e.names(required & ~has), ', '), e.desc.name);
  end
  table = guatiao_mex('served_table', e.cid, logical(has));
  tables(key) = table;
end
