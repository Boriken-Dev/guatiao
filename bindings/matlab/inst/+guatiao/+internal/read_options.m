% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [numbers, maps] = read_options(varargin)
% The gateway's codes for the 'Numbers' and 'Maps' reading options.
  o = guatiao.internal.options(varargin, struct('Numbers', 'auto', 'Maps', 'auto'));
  numbers = find(strcmpi(o.Numbers, {'auto', 'text', 'double'})) - 1;
  if isempty(numbers)
    error('guatiao:BadValue', 'Numbers is ''auto'', ''text'' or ''double''');
  end
  maps = find(strcmpi(o.Maps, {'auto', 'map'})) - 1;
  if isempty(maps)
    error('guatiao:BadValue', 'Maps is ''auto'' or ''map''');
  end
end
