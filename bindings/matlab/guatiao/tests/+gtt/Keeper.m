% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Keeper < greeterkind.Greeter
% Keeps every map handed to it, in the global GTT_KEPT.
  methods
    function out = greet(self, name)
      out = struct('greeting', name);
    end

    function out = greet_from(self, config)
      global GTT_KEPT
      GTT_KEPT{end + 1} = config;
      out = struct('greeting', 'kept');
    end
  end
end
