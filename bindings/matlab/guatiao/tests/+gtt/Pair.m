% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Pair < guatiao.Kind
% A test kind: two maps handed over, their sizes answered.
  properties (Constant, Hidden)
    PairKind = guatiao.Kind.describe('pair', 7, ...
      guatiao.slot('both', {'a', guatiao.taken('map'), 'b', guatiao.taken('map')}, 'Returns', 'int64', 'Required', true))
  end
  methods
    function n = both(self, a, b)
      n = self.call('both', a, b);
    end
  end
end
