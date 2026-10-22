% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Span
% A length in calendar units, one sign for the whole: an ISO 8601 duration.
%
%   s = guatiao.Span('P1DT2H');   s = guatiao.Span('1h 30m');
%   s = guatiao.Span('Days', 1, 'Hours', 2);
%   char(-s)                      % '-P1DT2H'
%
% Spans are equal unit by unit: P1D is not PT24H. A unit past 2^53 is
% refused (a double holds whole numbers exactly only that far).

  properties (SetAccess = private)
    Years = 0
    Months = 0
    Weeks = 0
    Days = 0
    Hours = 0
    Minutes = 0
    Seconds = 0
    Nanoseconds = 0
  end

  properties (Hidden, SetAccess = private)
    Text = 'PT0S'
  end

  methods
    function self = Span(varargin)
      if nargin == 0
        return
      end
      if nargin == 1 && ischar(varargin{1})
        f = guatiao_mex('time_parse', 'span', varargin{1});
      elseif nargin == 1 && isnumeric(varargin{1}) && numel(varargin{1}) == 8
        f = double(varargin{1}(:)');
      else
        o = guatiao.internal.options(varargin, struct('Years', 0, 'Months', 0, 'Weeks', 0, 'Days', 0, ...
                                                       'Hours', 0, 'Minutes', 0, 'Seconds', 0, 'Nanoseconds', 0));
        f = double([o.Years, o.Months, o.Weeks, o.Days, o.Hours, o.Minutes, o.Seconds, o.Nanoseconds]);
      end
      self.Text = guatiao_mex('time_format', 'span', f);
      f = guatiao_mex('time_parse', 'span', self.Text);
      self.Years = f(1);
      self.Months = f(2);
      self.Weeks = f(3);
      self.Days = f(4);
      self.Hours = f(5);
      self.Minutes = f(6);
      self.Seconds = f(7);
      self.Nanoseconds = f(8);
    end

    function f = fields(self)
      f = [self.Years, self.Months, self.Weeks, self.Days, self.Hours, self.Minutes, self.Seconds, self.Nanoseconds];
    end

    function t = char(self)
      t = self.Text;
    end

    function s = uminus(self)
      s = guatiao.Span(-self.fields());
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.Span') && isa(b, 'guatiao.Span') && strcmp(a.Text, b.Text);
    end

    function tf = ne(a, b)
      tf = ~eq(a, b);
    end

    function tf = isequal(a, b)
      tf = eq(a, b);
    end

    function x = guatiao_value(self)
      x = self.Text;
    end

    function disp(self)
      fprintf('  guatiao.Span %s\n', self.Text);
    end
  end
end
