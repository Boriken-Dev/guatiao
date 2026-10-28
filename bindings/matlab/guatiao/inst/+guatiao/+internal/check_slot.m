% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function check_slot(kind, s, object)
% What #[guatiao::kind] and the library's description reader refuse to
% declare, for a slot guatiao.Kind.describe is given: a slot named header,
% a borrowed list, bytes written in place on a kind that is not an object
% kind, and an object passed or answered on a slot that cannot fail (its
% table is checked where it arrives).
  at = sprintf('%s.%s', kind, s.name);
  if strcmp(s.name, 'header')
    error('guatiao:BadValue', '%s: the table''s first field is its header; name the slot otherwise', at);
  end
  trees = {'value', 'map', 'list', 'bytes'};
  for j = 2:2:numel(s.args)
    shape = s.args{j};
    where = sprintf('%s(%s)', at, s.args{j - 1});
    if ischar(shape) && strcmp(shape, 'list')
      error('guatiao:BadValue', '%s: a list is not borrowed: guatiao.lent(''list'') or guatiao.taken(''list'')', where);
    end
    if ~isstruct(shape) || ~isfield(shape, 'wrap')
      continue
    end
    if strcmp(shape.shape, 'bytes') && ~object
      error('guatiao:BadValue', '%s: bytes written in place cross on an object kind only', where);
    end
    if ~any(strcmp(trees, shape.shape)) && ~s.fallible
      error('guatiao:BadValue', '%s: an object crosses on a fallible slot only', where);
    end
  end
  plain = {'string', 'bytes', 'bool', 'int8', 'uint8', 'int16', 'uint16', 'int32', 'uint32', 'int64', 'uint64', ...
           'float', 'double', 'size', 'intptr', 'value', 'map', 'list'};
  if ischar(s.returns) && ~isempty(s.returns) && ~any(strcmp(plain, s.returns)) && ~s.fallible
    error('guatiao:BadValue', '%s answer: an object crosses on a fallible slot only', at);
  end
end
