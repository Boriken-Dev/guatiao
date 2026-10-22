% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function form = for_schema(schema)
% The form a schema implies, as a new guatiao.Value.
  form = guatiao.Value.adopt(guatiao_mex('intake_for_schema', schema));
end
