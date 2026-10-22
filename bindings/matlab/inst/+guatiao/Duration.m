% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Duration
% An exact length of time, never negative: written [seconds, nanoseconds].
%
%   d = guatiao.Duration(90, 500000000);   d = guatiao.Duration(1.5);
%   seconds(d)                             % 90.5
%   guatiao.time.read_length(x)            % any of its three spellings
%
% Seconds past 2^53 are refused (a double holds them exactly only that far).

  properties (SetAccess = private)
    Seconds = 0
    Nanoseconds = 0
  end

  methods
    function self = Duration(s, ns)
      if nargin == 0
        return
      end
      if nargin < 2
        ns = 0;
      end
      s = double(s);
      ns = double(ns);
      if ~isscalar(s) || ~isscalar(ns) || ~isfinite(s) || ~isfinite(ns) || s < 0 || ns < 0
        error('guatiao:BadValue', 'an exact length is never negative');
      end
      whole = floor(s);
      ns = ns + round((s - whole) * 1e9);
      whole = whole + floor(ns / 1e9);
      ns = mod(ns, 1e9);
      if whole > 9007199254740992 || ns ~= floor(ns)
        error('guatiao:BadValue', 'an exact length is whole seconds (to 2^53) and nanoseconds');
      end
      self.Seconds = whole;
      self.Nanoseconds = ns;
    end

    function s = seconds(self)
      s = self.Seconds + self.Nanoseconds / 1e9;
    end

    function x = guatiao_value(self)
      x = {self.Seconds, self.Nanoseconds};
    end

    function t = char(self)
      t = sprintf('[%d, %d]', self.Seconds, self.Nanoseconds);
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.Duration') && isa(b, 'guatiao.Duration') && a.Seconds == b.Seconds && ...
           a.Nanoseconds == b.Nanoseconds;
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function disp(self)
      fprintf('  guatiao.Duration %s\n', char(self));
    end
  end
end
