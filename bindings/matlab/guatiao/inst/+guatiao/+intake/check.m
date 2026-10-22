% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function problem = check(schema, form)
% [] when the form fits its schema; otherwise the error as data. Every
% argument of the intake functions is a guatiao value, a view or MATLAB
% data.
  problem = guatiao_mex('intake_check', schema, form);
end
