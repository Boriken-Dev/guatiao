% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Conversation < guatiao.Kind
% conversation_vtable of include/greeter_kind.h: one conversation a
% greeter started, an object kind (close() destroys it).

  properties (Constant, Hidden)
    ConversationKind = guatiao.Kind.describe('conversation', 2769624503, 'Object', true, ...
      guatiao.slot('say', {'what', 'string'}, 'Required', true, 'Fallible', true), ...
      guatiao.slot('read', {'dst', guatiao.lent('bytes')}, 'Returns', 'int64', 'Required', true), ...
      guatiao.slot('turns', {}, 'Returns', 'int64', 'Required', true))
  end

  methods
    function say(self, what)
      self.call('say', what);
    end

    function n = read(self, dst)
    % Writes the transcript into the guatiao.Buffer `dst`; the bytes written.
      n = self.call('read', dst);
    end

    function n = turns(self)
      n = self.call('turns');
    end
  end
end
