% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function drain()
% Releases what a library's own thread destroyed, which waited for the
% interpreter's thread.
  ids = guatiao_mex('pending_destroys');
  for k = 1:numel(ids)
    guatiao.internal.held('release', ids(k), true);
  end
end
