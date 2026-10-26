% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [out, extra] = kinds(op, varargin)
% The kinds this session knows, by class and by the gateway's id: each
% kind's description (its <Class>Kind constant), its shape codes and its C
% description, made once a session.
%
%   e = kinds('of', cls)              the entry of a kind class
%   [e, i] = kinds('slot', cls, name) the entry and a slot's position
%   e = kinds('by_id', id)            the entry of a C kind id
%   tf = kinds('is_kind', cls)        whether a class declares a kind
  persistent by_class by_id
  if isempty(by_class)
    by_class = containers.Map('KeyType', 'char', 'ValueType', 'any');
    by_id = containers.Map('KeyType', 'double', 'ValueType', 'any');
  end
  extra = [];
  switch op
    case 'is_kind'
      out = ~isempty(description_of(varargin{1}));
    case 'of'
      cls = varargin{1};
      if isKey(by_class, cls)
        out = by_class(cls);
        return
      end
      out = build(cls);
      by_class(cls) = out;
      by_id(out.cid) = out;
    case 'slot'
      out = guatiao.internal.kinds('of', varargin{1});
      extra = find(strcmp(out.names, varargin{2}), 1);
      if isempty(extra)
        error('guatiao:NotFound', 'the %s kind has no slot %s', out.desc.name, varargin{2});
      end
    case 'by_id'
      id = varargin{1};
      if ~isKey(by_id, id)
        error('guatiao:NotFound', 'no kind has the id %d in this session', id);
      end
      out = by_id(id);
    otherwise
      error('guatiao:Internal', 'kinds: no operation %s', op);
  end
end

function d = description_of(cls)
% The <Class>Kind constant the class itself defines, or [].
  d = [];
  mc = meta.class.fromName(cls);
  if isempty(mc)
    return
  end
  parts = strsplit(cls, '.');
  want = [parts{end} 'Kind'];
  props = mc.PropertyList;
  for i = 1:numel(props)
    p = guatiao.internal.item(props, i);
    if strcmp(p.Name, want) && p.Constant && strcmp(p.DefiningClass.Name, cls)
      d = eval([cls '.' want]);
      return
    end
  end
end

function e = build(cls)
  d = description_of(cls);
  if isempty(d)
    error('guatiao:WrongKind', '%s declares no kind: a kind class inherits guatiao.Kind and has a Constant %sKind', ...
          cls, regexprep(cls, '.*\.', ''));
  end
  n = numel(d.slots);
  names = cellfun(@(s) s.name, d.slots, 'UniformOutput', false);
  codes = cell(1, n);
  args = cell(1, n);
  arg_kinds = cell(1, n);
  rets = zeros(1, n);
  ret_kinds = zeros(1, n);
  for i = 1:n
    s = d.slots{i};
    na = numel(s.args) / 2;
    c = struct('args', zeros(1, na), 'arg_kinds', {cell(1, na)}, 'ret', 0, 'ret_kind', '', ...
               'arg_names', {s.args(1:2:end)});
    ids = zeros(1, na);
    for j = 1:na
      [c.args(j), c.arg_kinds{j}] = arg_code(s.args{2 * j}, sprintf('%s.%s(%s)', d.name, s.name, s.args{2 * j - 1}));
      if ~isempty(c.arg_kinds{j})
        k = guatiao.internal.kinds('of', c.arg_kinds{j});
        ids(j) = k.cid;
      end
    end
    [c.ret, c.ret_kind] = ret_code(s.returns, sprintf('%s.%s', d.name, s.name));
    if ~isempty(c.ret_kind)
      k = guatiao.internal.kinds('of', c.ret_kind);
      ret_kinds(i) = k.cid;
    end
    codes{i} = c;
    args{i} = int32(c.args);
    arg_kinds{i} = int32(ids);
    rets(i) = c.ret;
  end
  required = cellfun(@(s) s.required, d.slots);
  fallible = cellfun(@(s) s.fallible, d.slots);
  cid = guatiao_mex('kind_new', d.name, d.floor_hash, d.object, args, int32(rets), logical(fallible), ...
                    logical(required), arg_kinds, int32(ret_kinds));
  total = n + double(d.object);
  floor_slots = sum(required) + double(d.object);
  % A slot is a pointer: 8 bytes where an array may pass 2^32 elements, else 4.
  [~, most] = computer();
  word = 4 + 4 * (most > 2^32);
  e = struct('class', cls, 'desc', d, 'cid', cid, 'names', {names}, 'codes', {codes}, ...
             'size', 8 + word * total, 'floor_size', 8 + word * floor_slots);
end

function [code, kind] = arg_code(s, where)
  kind = '';
  plain = {'string', 1; 'bytes', 2; 'bool', 3; 'int8', 4; 'uint8', 5; 'int16', 6; 'uint16', 7; 'int32', 8; ...
           'uint32', 9; 'int64', 10; 'uint64', 11; 'float', 12; 'double', 13; 'size', 14; 'intptr', 15; ...
           'value', 20; 'map', 21; 'list', 22};
  lent = {'value', 30; 'map', 31; 'list', 32; 'bytes', 33};
  taken = {'value', 40; 'map', 41; 'list', 42};
  if isstruct(s) && isfield(s, 'wrap')
    table = lent;
    object = 34;
    if strcmp(s.wrap, 'taken')
      table = taken;
      object = 44;
    end
    hit = find(strcmp(table(:, 1), s.shape), 1);
    if ~isempty(hit)
      code = table{hit, 2};
      return
    end
    if any(strcmp(plain(:, 1), s.shape))
      error('guatiao:BadValue', '%s: %s(''%s'') is no shape', where, s.wrap, s.shape);
    end
    e = guatiao.internal.kinds('of', s.shape);
    if ~e.desc.object
      error('guatiao:BadValue', '%s: %s is not an object kind', where, s.shape);
    end
    code = object;
    kind = s.shape;
    return
  end
  hit = find(strcmp(plain(:, 1), s), 1);
  if isempty(hit)
    error('guatiao:BadValue', '%s: no such shape (an object kind is passed guatiao.lent(...) or guatiao.taken(...))', where);
  end
  code = plain{hit, 2};
end

function [code, kind] = ret_code(s, where)
  kind = '';
  code = 0;
  if isempty(s)
    return
  end
  plain = {'string', 1; 'bool', 3; 'int8', 4; 'uint8', 5; 'int16', 6; 'uint16', 7; 'int32', 8; 'uint32', 9; ...
           'int64', 10; 'uint64', 11; 'float', 12; 'double', 13; 'size', 14; 'intptr', 15; 'value', 20; ...
           'map', 21; 'list', 22};
  hit = find(strcmp(plain(:, 1), s), 1);
  if ~isempty(hit)
    code = plain{hit, 2};
    return
  end
  if strcmp(s, 'bytes')
    error('guatiao:BadValue', '%s: no slot answers bytes', where);
  end
  e = guatiao.internal.kinds('of', s);
  if ~e.desc.object
    error('guatiao:BadValue', '%s: %s is not an object kind', where, s);
  end
  code = 50;
  kind = s;
end
