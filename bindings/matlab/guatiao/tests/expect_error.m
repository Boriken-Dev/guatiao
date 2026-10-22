% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function e = expect_error(fn, id, label)
% Raises unless calling `fn` raises an error whose identifier starts with
% `id` (any error when `id` is empty); answers the error.
  global GT_EXPECTATIONS
  GT_EXPECTATIONS = GT_EXPECTATIONS + 1;
  if nargin < 2
    id = '';
  end
  if nargin < 3
    label = '';
  end
  try
    fn();
  catch e
    if ~isempty(id) && ~strncmp(e.identifier, id, numel(id))
      error('gt_test:fail', '%s: raised [%s] %s, want %s', label, e.identifier, e.message, id);
    end
    return
  end
  error('gt_test:fail', '%s: raised nothing, want %s', label, id);
end
