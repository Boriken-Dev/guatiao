% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Overshoot < greeterkind.Wide
% Answers numbers its slots cannot hold.
  methods
    function out = echo_i64(self, x)
      out = 2^63;
    end

    function out = echo_u64(self, x)
      out = -1;
    end
  end
end
