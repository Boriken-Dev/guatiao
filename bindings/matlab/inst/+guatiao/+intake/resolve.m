% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function node = resolve(schema, key)
% The schema of the field `key` ('auth.password'): a view, a copy or [], as
% guatiao.intake.at answers.
  node = guatiao.internal.lookup_answer(schema, guatiao_mex('intake_resolve', schema, key));
end
