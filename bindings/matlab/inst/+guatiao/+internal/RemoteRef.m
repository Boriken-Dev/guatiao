% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef RemoteRef < handle
% The gateway's remote a kind object calls, let go once: by close(), or
% when the last reference to it goes.
  properties
    Id = 0
  end

  methods
    function self = RemoteRef(id)
      self.Id = id;
    end

    function release(self)
      if self.Id ~= 0
        id = self.Id;
        self.Id = 0;
        guatiao_mex('remote_close', id);
      end
    end

    function delete(self)
      if self.Id ~= 0
        try
          guatiao_mex('remote_close', self.Id);
        catch
          % The gateway is gone at the end of a session.
        end
        self.Id = 0;
      end
    end
  end
end
