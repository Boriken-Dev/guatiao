% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = options(args, defaults)
% Name-value options in `args` read against a struct of defaults (names
% matched without case); a name the struct does not have raises.
  out = defaults;
  names = fieldnames(defaults);
  if mod(numel(args), 2) ~= 0
    error('guatiao:BadValue', 'options come in name-value pairs');
  end
  for i = 1:2:numel(args)
    if ~ischar(args{i})
      error('guatiao:BadValue', 'an option name is text');
    end
    hit = find(strcmpi(names, args{i}), 1);
    if isempty(hit)
      error('guatiao:BadValue', 'no option ''%s'' (%s)', args{i}, strjoin(names', ', '));
    end
    out.(names{hit}) = args{i + 1};
  end
end
