% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Date
% A calendar date, years 0000 to 9999, written 'YYYY-MM-DD'.
%
%   d = guatiao.Date(2026, 10, 21);   d = guatiao.Date('2026-10-21');
%   char(d)                           % '2026-10-21'
%   d.add(guatiao.Span('P1M'))        % a month later, clamped to its end
%
% Every date is checked by the library's own function.

  properties (SetAccess = private)
    Year = 1970
    Month = 1
    Day = 1
  end

  properties (Hidden, SetAccess = private)
    Text = '1970-01-01'
  end

  methods
    function self = Date(varargin)
      if nargin == 0
        return
      end
      if nargin == 1 && ischar(varargin{1})
        f = guatiao_mex('time_parse', 'date', varargin{1});
      elseif nargin == 3
        f = double([varargin{:}]);
      else
        error('guatiao:BadValue', 'guatiao.Date(text) or guatiao.Date(year, month, day)');
      end
      self.Text = guatiao_mex('time_format', 'date', f);
      self.Year = f(1);
      self.Month = f(2);
      self.Day = f(3);
    end

    function t = char(self)
      t = self.Text;
    end

    function d = add(self, span)
    % The date `span` later (years and months, then weeks and days); a span
    % with any time unit is refused.
      f = guatiao_mex('date_add_span', [self.Year, self.Month, self.Day], span.fields());
      d = guatiao.Date(f(1), f(2), f(3));
    end

    function x = guatiao_value(self)
      x = self.Text;
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.Date') && isa(b, 'guatiao.Date') && strcmp(a.Text, b.Text);
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function disp(self)
      fprintf('  guatiao.Date %s\n', self.Text);
    end
  end
end
