% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Sizes < gtt.Pair
% Answers how many entries two handed-over maps hold, and closes them.
  methods
    function n = both(self, a, b)
      n = a.length() + b.length();
      a.close();
      b.close();
    end
  end
end
