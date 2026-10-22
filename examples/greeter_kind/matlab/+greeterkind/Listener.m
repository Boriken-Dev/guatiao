% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Listener < guatiao.Kind
% listener_vtable of include/greeter_kind.h: what a conversation tells,
% implemented by the host; an object kind.

  properties (Constant, Hidden)
    ListenerKind = guatiao.Kind.describe('listener', 1867921573, 'Object', true, ...
      guatiao.slot('heard', {'what', 'string'}, 'Required', true))
  end

  methods
    function heard(self, what)
      self.call('heard', what);
    end
  end
end
