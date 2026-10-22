% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Number
% A number as its exact text (JSON's grammar), for what a double does not
% hold exactly: guatiao.Number('18446744073709551615.5'). Reading answers
% one where the 'auto' rule gives no double, int64 or uint64.
%
%   n = guatiao.Number('1.10');   n.Text   % '1.10'
%   double(n)                              % the nearest double

  properties (SetAccess = private)
    Text = '0'
  end

  methods
    function self = Number(text)
      if nargin == 0
        return
      end
      if ~ischar(text) || (size(text, 1) ~= 1 && ~isempty(text))
        error('guatiao:BadValue', 'a number is its text');
      end
      guatiao_mex('number_check', text);
      self.Text = text;
    end

    function d = double(self)
      d = str2double(self.Text);
    end

    function t = char(self)
      t = self.Text;
    end

    function tf = eq(a, b)
      tf = isa(a, 'guatiao.Number') && isa(b, 'guatiao.Number') && strcmp(a.Text, b.Text);
    end

    function tf = isequal(a, b, varargin)
      tf = eq(a, b);
      for i = 1:numel(varargin)
        tf = tf && eq(a, varargin{i});
      end
    end

    function disp(self)
      fprintf('  guatiao.Number %s\n', self.Text);
    end
  end
end
