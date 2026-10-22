% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function keys = flat_keys(schema, key)
% The flat keys the field `key` is stored under, a cell row of text.
  keys = guatiao_mex('intake_flat_keys', schema, key);
end
