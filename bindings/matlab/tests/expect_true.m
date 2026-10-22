% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function expect_true(x, label)
% Raises unless `x` is a true scalar.
  global GT_EXPECTATIONS
  GT_EXPECTATIONS = GT_EXPECTATIONS + 1;
  if nargin < 2
    label = '';
  end
  if ~(islogical(x) || isnumeric(x)) || ~isscalar(x) || ~x
    error('gt_test:fail', '%s: not true', label);
  end
end
