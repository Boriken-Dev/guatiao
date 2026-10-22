% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Kind < handle
% The base of every kind class. A kind is declared once, from its C header,
% as a classdef in the kind author's package:
%
%   classdef Greeter < guatiao.Kind
%     properties (Constant, Hidden)
%       GreeterKind = guatiao.Kind.describe('greeter', 2695856491, ...
%         guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true), ...
%         guatiao.slot('shout', {'name', 'string'}, 'Returns', 'string'))
%     end
%     methods
%       function out = greet(self, name), out = self.call('greet', name); end
%       function out = shout(self, name), out = self.call('shout', name); end
%     end
%   end
%
% The description is a Constant property named for the class (<Class>Kind);
% each slot has a method calling self.call. An instance bound to a table
% (Registry.bind, Instance.bind, an object a slot answers) is a remote: its
% methods call the table. A provider is a class inheriting its kinds and
% overriding their methods (see guatiao.provider).

  properties (Hidden, SetAccess = private)
    Ref = []       % a guatiao.internal.RemoteRef: the table this object calls
    Holder = []    % what the table belongs to: a guatiao.Registry or guatiao.Instance
  end

  properties (Hidden, Dependent)
    RemoteId       % the gateway's id of that table; 0 when this is no remote
  end

  methods (Static)
    function d = describe(name, floor_hash, varargin)
    % A kind's description: its C name, the floor hash its header defines,
    % then a guatiao.slot per slot in the header's order (required ones
    % first). Option 'Object', true for an object kind, whose destroy slot
    % is the remote's close().
      object = false;
      slots = {};
      i = 1;
      while i <= numel(varargin)
        a = varargin{i};
        if ischar(a) && strcmpi(a, 'Object')
          object = logical(varargin{i + 1});
          i = i + 2;
        elseif isstruct(a) && isfield(a, 'args')
          slots{end + 1} = a; %#ok<AGROW>
          i = i + 1;
        else
          error('guatiao:BadValue', 'a kind is described by guatiao.slot(...) entries and ''Object''');
        end
      end
      names = cellfun(@(s) s.name, slots, 'UniformOutput', false);
      if numel(unique(names)) ~= numel(names)
        error('guatiao:BadValue', 'two slots of %s share a name', name);
      end
      if object && any(strcmp(names, 'close'))
        error('guatiao:BadValue', 'close is an object kind''s destroy slot');
      end
      required = cellfun(@(s) s.required, slots);
      if any(diff(double(~required)) < 0)
        error('guatiao:BadValue', 'required slots come first: they are the kind''s original ones');
      end
      d = struct('name', name, 'floor_hash', double(floor_hash), 'object', object, 'slots', {slots});
    end
  end

  methods
    function tf = has_slot(self, name)
    % Whether this remote's table carries the slot (a slot appended after a
    % library was built is missing from its older table).
      self.check_bound();
      [~, i] = guatiao.internal.kinds('slot', class(self), name);
      tf = guatiao_mex('remote_has', self.RemoteId, i);
    end

    function tf = is_bound(self)
    % Whether this object calls a table (a remote), not methods of its own.
      tf = self.RemoteId ~= 0;
    end

    function close(self)
    % Lets go of the table; an object of an object kind this side owns is
    % destroyed (its destroy slot), once. A provider class may define its
    % own close(), which runs when a host destroys what it was handed.
      if ~isempty(self.Ref)
        self.Ref.release();
      end
    end

    function id = get.RemoteId(self)
      if isempty(self.Ref)
        id = 0;
      else
        id = self.Ref.Id;
      end
    end
  end

  methods (Hidden)
    function attach(self, remote_id, holder)
      % The remote is let go when the last reference to this object goes,
      % through the RemoteRef, so a kind class has no delete of its own for
      % a provider serving two kinds to inherit along two paths.
      self.Ref = guatiao.internal.RemoteRef(remote_id);
      self.Holder = holder;
    end

    function id = detach(self)
    % The remote's id, which this object no longer closes.
      id = self.RemoteId;
      if ~isempty(self.Ref)
        self.Ref.Id = 0;
      end
    end
  end

  methods (Access = protected)
    function out = call(self, slot, varargin)
    % Calls the slot named `slot` of this remote's table with the slot's
    % arguments, in order. What a kind's methods call.
      if self.RemoteId == 0
        error('guatiao:Unsupported', '%s is not a remote, and it has no %s of its own', class(self), slot);
      end
      out = guatiao.internal.call_slot(self, slot, varargin);
    end

    function check_bound(self)
      if self.RemoteId == 0
        error('guatiao:Gone:closed', 'the remote is closed');
      end
    end
  end
end
