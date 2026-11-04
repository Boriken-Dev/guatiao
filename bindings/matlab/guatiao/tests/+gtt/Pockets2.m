% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Pockets2 < gtt.Pocket2
% Lets go of what it is handed.
  methods
    function put(self, label, a, b)
      a.close();
      b.close();
    end
  end
end
