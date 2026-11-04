% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Adds < gtt.Summer
% Adds a list it borrows; what an attempt to change it came to is kept in
% Adds.refused.
  methods
    function n = total(self, items)
      try
        items.push(0);
        gtt.Adds.refused('changed');
      catch e
        gtt.Adds.refused(e.message);
      end
      d = items.data();
      if iscell(d)
        d = cellfun(@double, d);
      end
      n = int64(sum(double(d)));
    end
  end
  methods (Static)
    function out = refused(why)
      persistent seen
      if isempty(seen)
        seen = {};
      end
      if nargin
        seen{end + 1} = why;
      end
      out = seen;
    end
  end
end
