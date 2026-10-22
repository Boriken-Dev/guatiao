% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef HostEar < greeterkind.Listener
% A listener: what a conversation tells, kept; closed when its holder lets
% it go.
  properties
    Heard = {}
    Closed = false
  end

  methods
    function heard(self, what)
      self.Heard{end + 1} = what;
    end

    function close(self)
      self.Closed = true;
    end
  end
end
