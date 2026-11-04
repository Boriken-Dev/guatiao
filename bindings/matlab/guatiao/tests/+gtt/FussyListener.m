% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef FussyListener < guatiao.Kind
% The listener kind with a class that cannot be made: wrapping a listener
% handed to a provider fails, as it does for a class that is no kind.
  properties (Constant, Hidden)
    FussyListenerKind = guatiao.Kind.describe('listener', 1867921573, 'Object', true, ...
      guatiao.slot('heard', {'what', 'string'}, 'Required', true))
  end
  methods
    function self = FussyListener()
      error('guatiao:WrongKind', 'a listener of this class cannot be made');
    end
    function heard(self, what)
      self.call('heard', what);
    end
  end
end
