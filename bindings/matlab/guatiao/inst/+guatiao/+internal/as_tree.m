% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function r = as_tree(x)
% What an object is as a value, for the gateway: a cell {kind, ...}.
%   {'node', owner, path}  a guatiao value or view (copied, or read in place)
%   {'number', text}       a number's exact text
%   {'absent'}             nothing at all
%   {'map', keys, values}  a map of text keys
%   {'data', y}            write y instead
%   {'refuse', why}        not something a value holds
  if isa(x, 'guatiao.Tree')
    [owner, path] = node_of(x);
    r = {'node', owner, path};
  elseif isa(x, 'guatiao.Number')
    r = {'number', x.Text};
  elseif isa(x, 'guatiao.Absent')
    r = {'absent'};
  elseif isa(x, 'containers.Map')
    if ~strcmp(x.KeyType, 'char')
      r = {'refuse', 'a containers.Map with numeric keys: a map''s keys are text'};
    else
      r = {'map', keys(x), values(x)};
    end
  elseif ~isscalar(x) && isobject(x)
    items = cell(1, numel(x));
    for i = 1:numel(x)
      items{i} = x(i);
    end
    r = {'data', items};
  elseif ismethod(x, 'guatiao_value')
    r = {'data', guatiao_value(x)};
  elseif isa(x, 'string')
    if ismissing(x)
      r = {'data', []};
    else
      r = {'data', char(x)};
    end
  elseif isa(x, 'datetime')
    r = {'data', datetime_text(x)};
  elseif isa(x, 'duration')
    s = seconds(x);
    if s < 0 || ~isfinite(s)
      r = {'refuse', 'an exact length is never negative'};
    else
      whole = floor(s);
      r = {'data', {whole, round((s - whole) * 1e9)}};
    end
  elseif isobject(x)
    names = properties(x);
    vals = cell(1, numel(names));
    for i = 1:numel(names)
      vals{i} = x.(names{i});
    end
    r = {'map', reshape(names, 1, []), vals};
  else
    r = {'refuse', sprintf('cannot write a %s', class(x))};
  end
end

function [owner, path] = node_of(x)
  owner = x.owner_id();
  path = x.node_path();
end

function t = datetime_text(x)
  if isempty(x.TimeZone)
    t = char(x, 'yyyy-MM-dd''T''HH:mm:ss.SSSSSSSSS');
    t = guatiao.time.normalize('datetime', t);
  else
    x.TimeZone = 'UTC';
    t = char(x, 'yyyy-MM-dd''T''HH:mm:ss.SSSSSSSSS''Z''');
    t = guatiao.time.normalize('timestamp', t);
  end
end
