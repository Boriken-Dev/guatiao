% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function tf = is_visible(schema, form, key, values)
% Whether the field `key` is shown, given the values entered so far.
  tf = guatiao_mex('intake_is_visible', schema, form, key, values);
end
