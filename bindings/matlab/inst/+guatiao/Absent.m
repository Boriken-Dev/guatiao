% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Absent
% Nothing at all, as opposed to null ([]): guatiao.Absent(). A map entry
% holding it is written as no entry; guatiao.Value() holds it.
  methods
    function disp(~)
      fprintf('  guatiao.Absent\n');
    end

    function tf = isequal(a, b, varargin)
      tf = isa(a, 'guatiao.Absent') && isa(b, 'guatiao.Absent');
      for i = 1:numel(varargin)
        tf = tf && isa(varargin{i}, 'guatiao.Absent');
      end
    end
  end
end
