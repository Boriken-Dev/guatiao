% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function r = repo_root()
% The checkout's root: GUATIAO_REPO, else three folders up from here.
  r = getenv('GUATIAO_REPO');
  if isempty(r)
    r = fileparts(fileparts(fileparts(fileparts(mfilename('fullpath')))));
  end
end
