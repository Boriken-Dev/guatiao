% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef TimeOfDay
% A time of day to the nanosecond, with no zone: 'HH:MM:SS[.fraction]'.
%
%   t = guatiao.TimeOfDay(13, 45, 0, 500000000);   t = guatiao.TimeOfDay('13:45:00.5');

  properties (SetAccess = private)
    Hour = 0
    Minute = 0
    Second = 0
    Nanosecond = 0
  end

  properties (Hidden, SetAccess = private)
    Text = '00:00:00'
  end

  methods
    function self = TimeOfDay(varargin)
      if nargin == 0
        return
      end
      if nargin == 1 && ischar(varargin{1})
        f = guatiao_mex('time_parse', 'time', varargin{1});
      elseif nargin >= 2 && nargin <= 4
        f = [double([varargin{:}]), zeros(1, 4 - nargin)];
      else
        error('guatiao:BadValue', 'guatiao.TimeOfDay(text) or guatiao.TimeOfDay(hour, minute, second, nanosecond)');
      end
      self.Text = guatiao_mex('time_format', 'time', f);
      self.Hour = f(1);
      self.Minute = f(2);
      self.Second = f(3);
      self.Nanosecond = f(4);
    end

    function t = char(self)
      t = self.Text;
    end

    function f = fields(self)
      f = [self.Hour, self.Minute, self.Second, self.Nanosecond];
    end

    function x = guatiao_value(self)
      x = self.Text;
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.TimeOfDay') && isa(b, 'guatiao.TimeOfDay') && strcmp(a.Text, b.Text);
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function disp(self)
      fprintf('  guatiao.TimeOfDay %s\n', self.Text);
    end
  end
end
