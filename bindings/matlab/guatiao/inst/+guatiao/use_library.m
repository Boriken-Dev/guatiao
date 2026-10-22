% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function use_library(path)
% Names the guatiao library this session uses, before the first call that
% needs it: a file exporting the C ABI, whatever it is called, or a folder
% holding guatiao.dll / libguatiao.so / libguatiao.dylib. It wins over
% GUATIAO_LIBRARY. guatiao.serde and guatiao.intake have their own
% use_library. Naming another file once one is loaded raises guatiao:Busy.
  if ~ischar(path)
    error('guatiao:BadValue', 'the path is text');
  end
  guatiao_mex('use_library', path, 0);
end
