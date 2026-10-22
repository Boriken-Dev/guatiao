% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Picky < greeterkind.Greeter
% Greets with the name alone; says why it cannot run when asked to.
  methods
    function out = greet(self, name)
      out = struct('greeting', name);
    end
  end
end
