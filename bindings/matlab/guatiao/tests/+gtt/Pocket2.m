% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Pocket2 < guatiao.Kind
% A test kind: a label, then two listeners handed over.
  properties (Constant, Hidden)
    Pocket2Kind = guatiao.Kind.describe('pocket2', 13, ...
      guatiao.slot('put', {'label', 'string', 'a', guatiao.taken('gtt.FussyListener'), ...
                          'b', guatiao.taken('gtt.FussyListener')}, ...
                   'Required', true, 'Fallible', true))
  end
  methods
    function put(self, label, a, b)
      self.call('put', label, a, b);
    end
  end
end
