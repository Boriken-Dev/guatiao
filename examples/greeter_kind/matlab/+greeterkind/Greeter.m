% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Greeter < guatiao.Kind
% greeter_vtable of include/greeter_kind.h: greets by name. greet is the
% one slot every table has; the rest were appended, and an older table may
% lack them. The floor hash is the #define beside the table; a test
% compares it with the header.

  properties (Constant, Hidden)
    GreeterKind = guatiao.Kind.describe('greeter', 2695856491, ...
      guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true), ...
      guatiao.slot('shout', {'name', 'string'}, 'Returns', 'string'), ...
      guatiao.slot('start', {'listener', guatiao.taken('greeterkind.Listener')}, ...
                   'Returns', 'greeterkind.Conversation', 'Fallible', true), ...
      guatiao.slot('greet_into', {'name', 'string', 'listener', guatiao.lent('greeterkind.Listener')}, 'Fallible', true), ...
      guatiao.slot('greet_from', {'config', guatiao.taken('map')}, 'Returns', 'map', 'Fallible', true), ...
      guatiao.slot('check', {'config', guatiao.lent('map')}, 'Returns', 'bool', 'Fallible', true))
  end

  methods
    function out = greet(self, name)
    % {greeting: ...} for `name`; BadValue for nobody.
      out = self.call('greet', name);
    end

    function out = shout(self, name)
      out = self.call('shout', name);
    end

    function out = start(self, listener)
    % A greeterkind.Conversation that tells `listener` what is said.
      out = self.call('start', listener);
    end

    function greet_into(self, name, listener)
      self.call('greet_into', name, listener);
    end

    function out = greet_from(self, config)
    % Greets the name in `config`, which is handed over.
      out = self.call('greet_from', config);
    end

    function out = check(self, config)
    % Fills `config` in place; whether it names someone.
      out = self.call('check', config);
    end
  end
end
