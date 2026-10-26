% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Watching < tallykind.Watcher
% The host's watcher, lent to a tally for one call: what it was told, kept.
  properties
    Said = {}
  end

  methods
    function changed(self, name, total)
      self.Said{end + 1} = sprintf('%s=%d', name, total);
    end
  end
end
