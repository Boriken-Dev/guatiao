% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Value < guatiao.Tree
% A guatiao value: owns the root of a tree, freed by close() or when the
% last reference to it goes.
%
%   v = guatiao.Value(struct('host', '10.0.0.1', 'port', 5900, 'tags', {{'a', 'b'}}));
%   v.set('port', 5901);
%   v.at('tags').push('c');
%   v.at('port').data()     % 5901
%   v.close();
%
% guatiao.Value() holds absent; guatiao.Value(x, 'Alloc', a) builds in
% allocator `a`. See guatiao.Tree for the methods.

  properties (Hidden, SetAccess = private)
    Borrowed = false   % a tree lent for one call: its lender frees it
  end

  methods
    function self = Value(varargin)
      if nargin >= 1 && ischar(varargin{1}) && strcmp(varargin{1}, '__guatiao_adopt__')
        self.OwnerId = varargin{2};
        self.Borrowed = nargin >= 3 && varargin{3};
        return
      end
      alloc = guatiao.internal.option(varargin(2:end), 'Alloc', []);
      if nargin == 0
        x = guatiao.Absent();
      else
        x = varargin{1};
      end
      self.OwnerId = guatiao_mex('value_new', x, guatiao.internal.alloc_id(alloc));
    end

    function close(self)
    % Frees the tree now; a second close does nothing. A view of a closed
    % value raises guatiao:Gone:closed.
      if self.OwnerId ~= 0 && ~self.Borrowed
        guatiao_mex('value_close', self.OwnerId);
      end
      self.OwnerId = 0;
    end

    function tf = is_open(self)
      tf = self.OwnerId ~= 0 && guatiao_mex('value_is_open', self.OwnerId);
    end

    function delete(self)
      try
        close(self);
      catch
        % The gateway is gone at the end of a session.
      end
    end
  end

  methods (Static, Hidden)
    function v = adopt(id, borrowed)
    % A Value over an owner the gateway already filed.
      if nargin < 2
        borrowed = false;
      end
      v = guatiao.Value('__guatiao_adopt__', id, borrowed);
    end
  end
end
