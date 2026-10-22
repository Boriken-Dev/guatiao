% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function release_id(id)
% A destroy slot reached the interpreter's thread: the object goes, closed.
  guatiao.internal.held('release', id, true);
end
