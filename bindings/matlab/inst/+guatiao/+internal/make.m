% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function obj = make(spec, config)
% A provider's instance: Make(config) or Make() when given, else its
% class's constructor with the configuration, or with nothing.
  configured = ~isempty(spec.Config) || ~isempty(spec.ConfigSchema);
  if ~isempty(spec.Make)
    if configured && ~isempty(config)
      obj = spec.Make(config);
    else
      obj = spec.Make();
    end
  elseif configured && ~isempty(config)
    obj = feval(spec.Class, config);
  else
    obj = feval(spec.Class);
  end
end
