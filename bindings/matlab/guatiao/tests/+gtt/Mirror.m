% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Mirror < guatiao.Kind
% A test kind answering a map.
  properties (Constant, Hidden)
    MirrorKind = guatiao.Kind.describe('mirror', 8, guatiao.slot('show_me', {}, 'Returns', 'map', 'Required', true))
  end
  methods
    function out = show_me(self)
      out = self.call('show_me');
    end
  end
end
