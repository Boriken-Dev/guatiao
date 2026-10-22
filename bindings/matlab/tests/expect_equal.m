% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function expect_equal(got, want, label)
% Raises when `got` is not `want` (isequal, and the same class).
  global GT_EXPECTATIONS
  GT_EXPECTATIONS = GT_EXPECTATIONS + 1;
  if nargin < 3
    label = '';
  end
  same = isequal(got, want) && strcmp(class(got), class(want));
  if ~same
    error('gt_test:fail', '%s: got %s, want %s', label, show(got), show(want));
  end
end

function s = show(x)
  try
    s = sprintf('%s %s', class(x), guatiao.serde.emit(x));
  catch
    try
      s = sprintf('%s %s', class(x), mat2str(x));
    catch
      s = class(x);
    end
  end
end
