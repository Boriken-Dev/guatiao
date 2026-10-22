% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Built < gtt.Mirror
% Answers a guatiao.Value it built itself, in the binding's allocator.
  methods
    function out = show_me(self)
      out = guatiao.Value(struct('built', 'here'));
    end
  end
end
