% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Allocator
% An allocator the gateway made. guatiao.Allocator.counting() counts the
% blocks it handed out and has not had back (live()); trees record their
% allocator, so one stays for the session.

  properties (SetAccess = private)
    Id = 0
  end

  methods (Static)
    function a = counting()
      a = guatiao.Allocator(guatiao_mex('alloc_counting'));
    end
  end

  methods
    function self = Allocator(id)
      if nargin
        self.Id = id;
      end
    end

    function n = live(self)
    % Blocks handed out and not yet given back.
      n = guatiao_mex('alloc_live', self.Id);
    end

    function tf = records(self, node)
    % Whether a node's container (or its text, number or bytes) records this
    % allocator.
      tf = guatiao_mex('node_alloc_is', node.owner_id(), node.node_path(), self.Id);
    end
  end
end
