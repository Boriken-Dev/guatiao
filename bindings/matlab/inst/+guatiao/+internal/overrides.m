% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function has = overrides(cls, kind_cls, names)
% For each method name, whether `cls` defines it somewhere other than the
% kind class that declares it (an implementation, not the remote's call).
  has = false(1, numel(names));
  mc = meta.class.fromName(cls);
  if isempty(mc)
    error('guatiao:NotFound', 'no class %s', cls);
  end
  ms = mc.MethodList;
  for i = 1:numel(ms)
    m = guatiao.internal.item(ms, i);
    hit = strcmp(names, m.Name);
    if any(hit) && ~strcmp(m.DefiningClass.Name, kind_cls)
      has(hit) = true;
    end
  end
end
