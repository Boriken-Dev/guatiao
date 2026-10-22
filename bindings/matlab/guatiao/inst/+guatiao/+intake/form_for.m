% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function sub = form_for(form, schema, path)
% The part of a form for the field at `path`, as a new guatiao.Value, or []
% when it has none.
  id = guatiao_mex('intake_form_for', form, schema, path);
  if isempty(id)
    sub = [];
  else
    sub = guatiao.Value.adopt(id);
  end
end
