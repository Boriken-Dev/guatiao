% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function use_library(path)
% Names the guatiao_intake library this session uses, before the first
% call that needs it: a file exporting its functions, whatever it is
% called (an application's own library carrying several surfaces, say),
% or a folder holding it under its platform file name. It wins over
% GUATIAO_INTAKE_LIBRARY and GUATIAO_LIBRARY. Naming another file once one
% is loaded raises guatiao:Busy.
  if ~ischar(path)
    error('guatiao:BadValue', 'the path is text');
  end
  guatiao_mex('use_library', path, 2);
end
