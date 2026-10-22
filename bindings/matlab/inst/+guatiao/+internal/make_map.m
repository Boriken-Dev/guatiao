% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function m = make_map(keys, values)
% A containers.Map of text keys, for a map whose keys are not field names.
  m = containers.Map('KeyType', 'char', 'ValueType', 'any');
  for i = 1:numel(keys)
    m(keys{i}) = values{i};
  end
end
