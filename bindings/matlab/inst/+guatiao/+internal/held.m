% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = held(op, varargin)
% M objects served to C, by id: what a table's ctx names (65536 + 16 * id,
% an id never reused, so a released one answers Gone).
%
%   id = held('hold', obj)
%   obj = held('get', id)          [] when released
%   held('release', id, close)     closes the object when `close` and its
%                                  class defines close()
%   n = held('count')
  persistent objects next
  if isempty(objects)
    objects = containers.Map('KeyType', 'double', 'ValueType', 'any');
    next = 0;
  end
  out = [];
  switch op
    case 'hold'
      next = next + 1;
      objects(next) = varargin{1};
      out = next;
    case 'get'
      id = varargin{1};
      if isKey(objects, id)
        out = objects(id);
      end
    case 'release'
      id = varargin{1};
      if ~isKey(objects, id)
        return
      end
      obj = objects(id);
      remove(objects, id);
      if numel(varargin) >= 2 && varargin{2} && has_close(obj)
        try
          close(obj);
        catch e
          warning('guatiao:close', 'guatiao: close failed: %s', e.message);
        end
      end
    case 'count'
      out = double(objects.Count);
    otherwise
      error('guatiao:Internal', 'held: no operation %s', op);
  end
end

function tf = has_close(obj)
% Whether the object's class defines its own close(), not guatiao.Kind's.
  tf = false;
  mc = metaclass(obj);
  ms = mc.MethodList;
  for i = 1:numel(ms)
    m = guatiao.internal.item(ms, i);
    if strcmp(m.Name, 'close') && ~strcmp(m.DefiningClass.Name, 'guatiao.Kind')
      tf = true;
      return
    end
  end
end
