% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function t = read_text_file(path)
% A UTF-8 file's text as a character row (MATLAB: UTF-16, Octave: UTF-8).
  fid = fopen(path, 'r');
  if fid < 0
    error('gt_test:fail', 'cannot open %s', path);
  end
  bytes = fread(fid, Inf, '*uint8')';
  fclose(fid);
  t = native2unicode(bytes, 'UTF-8');
end
