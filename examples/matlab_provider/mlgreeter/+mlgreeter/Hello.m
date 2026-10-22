% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Hello < greeterkind.Greeter & greeterkind.Counter
% One provider serving two kinds, with one instance of its own.

  properties (Constant, Hidden)
    Provider = guatiao.provider('DisplayName', 'Hello, in M')
  end

  properties
    asked = 0
  end

  methods
    function out = greet(self, name)
      if strcmp(name, 'panic')
        error('a provider bug, on request');
      end
      if isempty(name)
        error('guatiao:BadValue', 'nobody to greet');
      end
      out = struct('greeting', ['hello, ' name]);
    end

    function out = shout(self, name)
      g = self.greet(name);
      out = upper(g.greeting);
    end

    function out = start(self, listener)
      out = mlgreeter.Chat(listener);
    end

    function greet_into(self, name, listener)
      g = self.greet(name);
      listener.heard(g.greeting);
    end

    function out = greet_from(self, config)
      name = '';
      if config.has('name')
        name = config.at('name').data();
      end
      config.close();
      out = self.greet(name);
    end

    function ok = check(self, config)
      name = 'world';
      if config.has('name')
        name = config.at('name').data();
      end
      config.set('name', name);
      config.set('greeting', ['hello, ' name]);
      ok = ~isempty(name);
    end

    function n = count(self)
      self.asked = self.asked + 1;
      n = self.asked;
    end

    function delete(~)
      % Octave 11 warns when an object of a class with two handle
      % superclasses is deleted, unless the class defines delete itself.
    end
  end
end
