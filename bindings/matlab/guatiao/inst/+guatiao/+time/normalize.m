% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [t, problem] = normalize(form, text)
% Rewrites text of a form in its canonical spelling:
% normalize('span', '1h 30m') is 'PT1H30M'. Refusals as guatiao.time.parse.
  [t, problem] = guatiao_mex('time_normalize', form, text, nargout >= 2);
  if ~isempty(t) || ischar(t)
    problem = [];
  end
end
