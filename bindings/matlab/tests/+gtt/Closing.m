% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Closing < greeterkind.Greeter
% Built from a configuration; counts its closes in GTT_CLOSED.
  properties
    label = ''
  end
  methods
    function self = Closing(config)
      if nargin
        self.label = config.label;
      end
    end
    function out = greet(self, name)
      out = struct('greeting', [self.label ' ' name]);
    end
    function close(self)
      global GTT_CLOSED
      GTT_CLOSED = GTT_CLOSED + 1;
    end
  end
end
