% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function problem = validate(schema, value)
% [] when the JSON Schema accepts the value; otherwise the refusal as data
% (a struct with key, message, ...), which never quotes the value. Either
% argument is a guatiao value or view, or MATLAB data.
  problem = guatiao_mex('schema_validate', schema, value);
end
