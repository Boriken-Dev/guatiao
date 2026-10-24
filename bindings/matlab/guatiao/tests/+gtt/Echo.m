% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Echo < greeterkind.Wide
% Answers the 64-bit integer it is given, and refuses 7.
  methods
    function out = echo_i64(self, x)
      if x == 7
        error('guatiao:BadValue', 'seven is no number');
      end
      out = x;
    end

    function out = echo_u64(self, x)
      if x == 7
        error('guatiao:BadValue', 'seven is no number');
      end
      out = x;
    end
  end
end
