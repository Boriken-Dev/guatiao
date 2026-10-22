% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Node < guatiao.Tree
% A view of a node in a guatiao.Value, made by at(): the value and a path
% of keys and positions, walked again on every use. It keeps its value
% alive; a view of a closed value raises guatiao:Gone:closed, and one
% whose node is gone raises guatiao:NotFound. See guatiao.Tree.

  methods
    function self = Node(holder, owner, steps)
      self.Holder = holder;
      self.OwnerId = owner;
      self.Steps = steps;
    end
  end
end
