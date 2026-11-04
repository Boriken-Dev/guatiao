% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function form = provider_form(schema, meta)
% The form a provider's settings are shown with, as a new guatiao.Value: the
% one it declares in its `meta` under guatiao.intake.provider_form_key(), or
% else the one its configuration `schema` implies (for_schema).
%
% `schema` and `meta` are what the registry answers for the provider
% (provider_config and provider_meta, or the `config` and `meta` fields of an
% entry Registry.catalog lists): each a guatiao.Value, a view, MATLAB data or
% [] for none. What is not a map is skipped, as reading the key by hand skips
% it. [] when that leaves nothing to show. A declared form is not checked
% against the schema here; check does that.
  id = guatiao_mex('intake_provider_form', schema, meta);
  if isempty(id)
    form = [];
  else
    form = guatiao.Value.adopt(id);
  end
end
