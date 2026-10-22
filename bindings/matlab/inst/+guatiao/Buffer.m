% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Buffer < handle
% Bytes a slot writes in place (a guatiao.lent('bytes') argument): the
% caller passes guatiao.Buffer(n) and reads Data after the call; a provider
% receives one and sets Data (its length stays what the caller lent).

  properties
    Data = zeros(1, 0, 'uint8')
  end

  methods
    function self = Buffer(n)
      if nargin == 0
        return
      end
      if isa(n, 'uint8')
        self.Data = reshape(n, 1, []);
      else
        self.Data = zeros(1, n, 'uint8');
      end
    end

    function set.Data(self, d)
      self.Data = reshape(uint8(d), 1, []);
    end

    function n = length(self)
      n = numel(self.Data);
    end
  end
end
