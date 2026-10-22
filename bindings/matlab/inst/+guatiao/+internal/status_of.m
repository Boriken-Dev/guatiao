% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [code, message] = status_of(err)
% The status an error answers across a slot: guatiao:<Status>[:...] its
% status, anything else Internal; and its words.
  s = guatiao.statuses();
  code = s.Internal;
  message = err.message;
  parts = strsplit(err.identifier, ':');
  if numel(parts) >= 2 && strcmp(parts{1}, 'guatiao') && isfield(s, parts{2})
    code = s.(parts{2});
  end
  % Octave puts the function's name before a MEX file's words.
  message = regexprep(message, '^guatiao_mex: ', '');
end
