% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function v = option(args, name, default)
% The value of the name-value option `name` in `args` (matched without
% case), or `default`. Other names are not checked here.
  v = default;
  if mod(numel(args), 2) ~= 0
    error('guatiao:BadValue', 'options come in name-value pairs');
  end
  for i = 1:2:numel(args)
    if ~ischar(args{i})
      error('guatiao:BadValue', 'an option name is text');
    end
    if strcmpi(args{i}, name)
      v = args{i + 1};
    end
  end
end
