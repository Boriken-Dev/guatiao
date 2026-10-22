% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function why = available(index)
% A provider's availability, asked by its slot: [] when it can run here,
% else why not. An error is a reason too.
  rec = guatiao.internal.providers('get', index);
  try
    why = rec.spec.Available();
    if ~isempty(why) && ~ischar(why)
      why = 'the provider cannot run here';
    end
  catch err
    why = err.message;
  end
end
