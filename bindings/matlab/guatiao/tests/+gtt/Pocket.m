% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Pocket < guatiao.Kind
% A test kind: a label, then a listener handed over.
  properties (Constant, Hidden)
    PocketKind = guatiao.Kind.describe('pocket', 9, ...
      guatiao.slot('put', {'label', 'string', 'item', guatiao.taken('greeterkind.Listener')}, ...
                   'Required', true, 'Fallible', true))
  end
  methods
    function put(self, label, item)
      self.call('put', label, item);
    end
  end
end
