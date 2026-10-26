% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Pockets < gtt.Pocket
% Lets go of what it is handed.
  methods
    function put(self, label, item)
      item.close();
    end
  end
end
