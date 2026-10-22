% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function d = source_dir()
% Where the gateway's C sources are: src/ beside inst/ in a checkout or an
% Octave package's source, or src/ inside the package folder once installed.
  pkg = fileparts(fileparts(fileparts(mfilename('fullpath'))));
  candidates = {fullfile(pkg, 'src'), fullfile(fileparts(pkg), 'src')};
  for i = 1:numel(candidates)
    if exist(fullfile(candidates{i}, 'guatiao_mex.c'), 'file') == 2
      d = candidates{i};
      return
    end
  end
  error('guatiao:NotFound', 'the gateway''s C sources are not beside the package (looked in %s)', ...
        strjoin(candidates, ', '));
end
