% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Timestamp
% An instant, to the nanosecond, written in UTC: '2026-10-21T13:45:00.5Z'.
% Read with any offset and converted to the instant it names.
%
%   t = guatiao.Timestamp('2026-10-21T19:15:00.5+05:30');   char(t)   % '...13:45:00.5Z'
%   t = guatiao.Timestamp(seconds, nanoseconds)              % since 1970, UTC
%   posix(t)                                                 % seconds as a double

  properties (SetAccess = private)
    Seconds = 0       % whole seconds since 1970-01-01T00:00:00Z
    Nanoseconds = 0   % 0 to 999999999
  end

  properties (Hidden, SetAccess = private)
    Text = '1970-01-01T00:00:00Z'
  end

  methods
    function self = Timestamp(varargin)
      if nargin == 0
        return
      end
      if nargin == 1 && ischar(varargin{1})
        f = guatiao_mex('time_parse', 'timestamp', varargin{1});
      elseif nargin == 1 || nargin == 2
        s = double(varargin{1});
        ns = 0;
        if nargin == 2
          ns = double(varargin{2});
        end
        whole = floor(s);
        ns = ns + round((s - whole) * 1e9);
        f = [whole + floor(ns / 1e9), mod(ns, 1e9)];
      else
        error('guatiao:BadValue', 'guatiao.Timestamp(text) or guatiao.Timestamp(seconds, nanoseconds)');
      end
      self.Text = guatiao_mex('time_format', 'timestamp', f);
      self.Seconds = f(1);
      self.Nanoseconds = f(2);
    end

    function t = char(self)
      t = self.Text;
    end

    function s = posix(self)
    % Seconds since 1970 as a double (to about a microsecond today).
      s = self.Seconds + self.Nanoseconds / 1e9;
    end

    function x = guatiao_value(self)
      x = self.Text;
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.Timestamp') && isa(b, 'guatiao.Timestamp') && strcmp(a.Text, b.Text);
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function disp(self)
      fprintf('  guatiao.Timestamp %s\n', self.Text);
    end
  end
end
