% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function sections = layout(schema, form)
% How the schema is shown: a cell of sections, each a struct with its
% section and its fields.
  sections = guatiao_mex('intake_layout', schema, form);
end
