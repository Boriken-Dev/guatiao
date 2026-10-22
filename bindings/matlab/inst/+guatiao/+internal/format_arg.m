% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [format, rest] = format_arg(args)
% A serde format given first ('json', 'toml', 'yaml') or as 'Format'.
  format = 'json';
  rest = args;
  if ~isempty(args) && ischar(args{1}) && any(strcmpi(args{1}, {'json', 'toml', 'yaml'}))
    format = lower(args{1});
    rest = args(2:end);
  end
  given = guatiao.internal.option(rest, 'Format', []);
  if ~isempty(given)
    format = lower(given);
    keep = true(1, numel(rest));
    for i = 1:2:numel(rest)
      if strcmpi(rest{i}, 'Format')
        keep(i:i + 1) = false;
      end
    end
    rest = rest(keep);
  end
end
