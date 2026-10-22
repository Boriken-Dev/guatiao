% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef DateTime
% A civil date and time, what a wall clock shows, with no zone:
% 'YYYY-MM-DDTHH:MM:SS[.fraction]'.
%
%   x = guatiao.DateTime('2026-10-21T13:45:00.5');
%   x = guatiao.DateTime(guatiao.Date(2026, 10, 21), guatiao.TimeOfDay(13, 45));
%   x.add(guatiao.Span('PT1H'))

  properties (SetAccess = private)
    Date = guatiao.Date()
    Time = guatiao.TimeOfDay()
  end

  properties (Hidden, SetAccess = private)
    Text = '1970-01-01T00:00:00'
  end

  methods
    function self = DateTime(varargin)
      if nargin == 0
        return
      end
      if nargin == 1 && ischar(varargin{1})
        f = guatiao_mex('time_parse', 'datetime', varargin{1});
      elseif nargin == 2 && isa(varargin{1}, 'guatiao.Date') && isa(varargin{2}, 'guatiao.TimeOfDay')
        d = varargin{1};
        f = [d.Year, d.Month, d.Day, varargin{2}.fields()];
      elseif nargin == 7
        f = double([varargin{:}]);
      else
        error('guatiao:BadValue', 'guatiao.DateTime(text), (date, time) or (y, mo, d, h, mi, s, ns)');
      end
      self.Text = guatiao_mex('time_format', 'datetime', f);
      self.Date = guatiao.Date(f(1), f(2), f(3));
      self.Time = guatiao.TimeOfDay(f(4), f(5), f(6), f(7));
    end

    function t = char(self)
      t = self.Text;
    end

    function f = fields(self)
      f = [self.Date.Year, self.Date.Month, self.Date.Day, self.Time.fields()];
    end

    function x = add(self, span)
    % The civil date and time `span` later, carried across midnight.
      f = guatiao_mex('datetime_add_span', self.fields(), span.fields());
      x = guatiao.DateTime(f(1), f(2), f(3), f(4), f(5), f(6), f(7));
    end

    function x = guatiao_value(self)
      x = self.Text;
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.DateTime') && isa(b, 'guatiao.DateTime') && strcmp(a.Text, b.Text);
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function disp(self)
      fprintf('  guatiao.DateTime %s\n', self.Text);
    end
  end
end
