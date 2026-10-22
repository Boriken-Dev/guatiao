% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function x = item(list, i)
% Element i of a metadata list: Octave answers cells, MATLAB object arrays.
  if iscell(list)
    x = list{i};
  else
    x = list(i);
  end
end
