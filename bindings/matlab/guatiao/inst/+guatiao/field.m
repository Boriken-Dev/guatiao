% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function f = field(type, varargin)
% One field of a guatiao.Config: its type, then options.
%
%   guatiao.field('string', 'Doc', 'What every greeting starts with.', 'minLength', 1)
%   guatiao.field('integer', 'Default', 1)
%   guatiao.field(guatiao.Config('host', guatiao.field('string')))   % nested
%
% Types: 'string', 'integer' (a whole number a double holds exactly),
% 'number', 'boolean', 'bytes', 'list', 'map', 'any', 'date', 'time',
% 'datetime', 'timestamp', 'span', 'duration', or a guatiao.Config.
% Options: 'Default' (the field is then optional, its default stated in the
% schema), 'Doc' (its description), 'Nullable' (it may be null, and is
% optional); any other name is a JSON Schema keyword added as it is.
  known = {'string', 'integer', 'number', 'boolean', 'bytes', 'list', 'map', 'any', ...
           'date', 'time', 'datetime', 'timestamp', 'span', 'duration'};
  if ~(isa(type, 'guatiao.Config') || (ischar(type) && any(strcmp(type, known))))
    error('guatiao:BadValue', 'a field''s type is one of %s, or a guatiao.Config', strjoin(known, ', '));
  end
  if mod(numel(varargin), 2) ~= 0
    error('guatiao:BadValue', 'options come in name-value pairs');
  end
  f = struct('type', {type}, 'has_default', false, 'default', {[]}, 'doc', '', 'nullable', false, ...
             'keywords', {cell(0, 2)});
  for i = 1:2:numel(varargin)
    name = varargin{i};
    v = varargin{i + 1};
    if ~ischar(name)
      error('guatiao:BadValue', 'an option name is text');
    end
    switch lower(name)
      case 'default'
        f.has_default = true;
        f.default = v;
      case 'doc'
        f.doc = v;
      case 'nullable'
        f.nullable = logical(v);
      otherwise
        f.keywords(end + 1, :) = {name, v};
    end
  end
end
