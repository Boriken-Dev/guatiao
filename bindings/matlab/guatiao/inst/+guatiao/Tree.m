% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef (Abstract) Tree < handle
% A node of a guatiao tree: a guatiao.Value (which owns its tree) or a
% guatiao.Node (a view into one). Every method walks the node's path again,
% never a cached address: a write beside a node can move the array it
% sits in.
%
% Reading:  data, tag, length, keys, has, isabsent, isnull, path, copy, to_wire
% Writing:  set, push, insert, remove, clear, merge
% Views:    at(key) or at(position, from 1)

  properties (Hidden, SetAccess = protected)
    OwnerId = 0   % the gateway's id of the tree's owner
    Steps = {}    % keys (text) and positions (from 0) from the root
    Holder = []   % what keeps the owner alive: the guatiao.Value
  end

  methods
    function x = data(self, varargin)
    % The node as MATLAB data. Options: 'Numbers' 'auto' (default), 'text'
    % (every number a guatiao.Number) or 'double'; 'Maps' 'auto' (a struct
    % where every key is a field name, else a containers.Map) or 'map'.
      [numbers, maps] = guatiao.internal.read_options(varargin{:});
      x = guatiao_mex('node_read', self.OwnerId, self.Steps, numbers, maps);
    end

    function t = tag(self)
    % 'absent', 'null', 'bool', 'number', 'string', 'bytes', 'list' or 'map'.
      t = guatiao_mex('node_tag', self.OwnerId, self.Steps);
    end

    function n = length(self)
    % A map's entries, a list's elements, 1 for any other value, 0 for null
    % and absent.
      n = guatiao_mex('node_length', self.OwnerId, self.Steps);
    end

    function k = keys(self)
    % A map's keys in insertion order, as a cell row; {} for anything else.
      k = guatiao_mex('node_keys', self.OwnerId, self.Steps);
    end

    function tf = has(self, key)
    % Whether a map holds `key`.
      tf = strcmp(self.tag(), 'map') && ~isempty(guatiao_mex('node_child', self.OwnerId, self.Steps, key));
    end

    function tf = isabsent(self)
      tf = strcmp(self.tag(), 'absent');
    end

    function tf = isnull(self)
      tf = strcmp(self.tag(), 'null');
    end

    function n = at(self, sel)
    % A view of a map's entry (a key, or a position from 1) or a list's
    % element (a position from 1): a guatiao.Node. A missing key answers [];
    % a position past the end raises.
      step = guatiao_mex('node_child', self.OwnerId, self.Steps, sel);
      if isempty(step) && ~isnumeric(sel)
        n = [];
        return
      end
      n = guatiao.Node(self.holder(), self.OwnerId, [self.Steps, {step}]);
    end

    function self = set(self, sel, x)
    % Sets a map's entry (a new key goes last) or replaces a list's element,
    % appending at length + 1. What is added is built in the allocator the
    % container records. A failed write leaves the tree as it was.
      guatiao_mex('node_set', self.OwnerId, self.Steps, sel, x);
    end

    function self = push(self, varargin)
    % Appends each argument to a list.
      guatiao_mex('node_push', self.OwnerId, self.Steps, varargin);
    end

    function self = insert(self, at, x)
    % Inserts before position `at` (from 1; length + 1 appends).
      guatiao_mex('node_insert', self.OwnerId, self.Steps, at, x);
    end

    function self = remove(self, sel)
    % Removes a map's key or entry, or a list's element; nothing there is no
    % error.
      guatiao_mex('node_remove', self.OwnerId, self.Steps, sel);
    end

    function self = clear(self)
    % Empties a map or a list.
      guatiao_mex('node_clear', self.OwnerId, self.Steps);
    end

    function self = merge(self, src)
    % Copies a map's entries into this map, replacing collisions.
      guatiao_mex('node_merge', self.OwnerId, self.Steps, src);
    end

    function v = copy(self, varargin)
    % A deep copy, a new guatiao.Value. Option 'Alloc': an allocator.
      alloc = guatiao.internal.option(varargin, 'Alloc', []);
      v = guatiao.Value.adopt(guatiao_mex('node_copy', self.OwnerId, self.Steps, guatiao.internal.alloc_id(alloc)));
    end

    function b = to_wire(self)
    % The node in the wire encoding, a uint8 row: a tag byte per node,
    % LEB128 lengths, a number as its own text, a map in its order.
    % guatiao.Value.from_wire reads the same tree back, in any language.
      b = guatiao_mex('wire_encode', self.OwnerId, self.Steps);
    end

    function p = path(self)
    % The node's path in the intake grammar, positions from 0: 'agent[1].name'.
      p = '';
      for i = 1:numel(self.Steps)
        s = self.Steps{i};
        if ischar(s)
          if isempty(p)
            p = s;
          else
            p = [p '.' s]; %#ok<AGROW>
          end
        else
          p = sprintf('%s[%d]', p, s);
        end
      end
    end

    function disp(self)
      try
        fprintf('  guatiao %s (%s): %s\n', class_word(self), self.tag(), ...
                guatiao.serde.emit(self, 'json'));
      catch e
        fprintf('  guatiao %s: %s\n', class_word(self), e.message);
      end
    end
  end

  methods (Hidden)
    function id = owner_id(self)
      id = self.OwnerId;
    end

    function s = node_path(self)
      s = self.Steps;
    end

    function n = view_of(self, steps)
    % A view of the node at `steps` from this tree's root.
      n = guatiao.Node(self.holder(), self.OwnerId, steps);
    end
  end

  methods (Access = protected)
    function h = holder(self)
      h = self.Holder;
      if isempty(h)
        h = self;
      end
    end
  end
end

function w = class_word(x)
  if isa(x, 'guatiao.Value')
    w = 'value';
  else
    w = 'view';
  end
end
