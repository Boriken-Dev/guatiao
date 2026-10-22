% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Counter < guatiao.Kind
% counter_vtable of include/greeter_kind.h: counts how often it was asked.

  properties (Constant, Hidden)
    CounterKind = guatiao.Kind.describe('counter', 1227528937, ...
      guatiao.slot('count', {}, 'Returns', 'int64', 'Required', true))
  end

  methods
    function n = count(self)
      n = self.call('count');
    end
  end
end
