% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function tf = has_export(symbol, library)
% Whether the loaded build has an export: has_export('guatiao_toml_emit',
% 'guatiao_serde'). `library` is 'guatiao' (the default), 'guatiao_serde'
% or 'guatiao_intake'; the library is opened first.
  if nargin < 2
    library = 'guatiao';
  end
  lib = find(strcmp(library, {'guatiao', 'guatiao_serde', 'guatiao_intake'})) - 1;
  if isempty(lib)
    error('guatiao:BadValue', 'no such library: %s', library);
  end
  tf = guatiao_mex('has_export', lib, symbol);
end
