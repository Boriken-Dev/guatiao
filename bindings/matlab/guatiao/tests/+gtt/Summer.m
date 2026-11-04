% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Summer < guatiao.Kind
% A test kind: a list borrowed, its integers added.
  properties (Constant, Hidden)
    SummerKind = guatiao.Kind.describe('summer', 10, ...
      guatiao.slot('total', {'items', 'list'}, 'Returns', 'int64', 'Required', true))
  end
  methods
    function n = total(self, items)
      n = self.call('total', items);
    end
  end
end
