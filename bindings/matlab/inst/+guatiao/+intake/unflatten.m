% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function value = unflatten(schema, key, store)
% The field `key`'s value read back from flat text (a containers.Map or a
% struct of text), as a new guatiao.Value; [] for "nothing stored" or a key
% that does not apply.
  if isstruct(store)
    held = struct2cell(store);
  elseif isa(store, 'containers.Map')
    held = values(store);
  else
    error('guatiao:BadValue', 'a store is a containers.Map or a struct of text');
  end
  if ~all(cellfun(@ischar, held))
    error('guatiao:BadValue', 'a store holds text');
  end
  id = guatiao_mex('intake_unflatten', schema, key, store);
  if isempty(id)
    value = [];
  else
    value = guatiao.Value.adopt(id);
  end
end
