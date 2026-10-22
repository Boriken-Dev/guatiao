% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function r = create(index, owner)
% A provider's create slot, on the interpreter's thread: the instance
% built from the configuration lent as `owner` (the host checked it
% against the schema), held; {status, message, ctx}.
  rec = guatiao.internal.providers('get', index);
  s = rec.spec;
  try
    if rec.library.is_closed()
      error('guatiao:Gone', 'the library is closed');
    end
    config = guatiao.Value.adopt(owner, true);
    if ~isempty(s.Config)
      cfg = s.Config.read(config);
    else
      cfg = config.data();
    end
    obj = guatiao.internal.make(s, cfg);
    id = guatiao.internal.held('hold', obj);
    r = {0, '', 65536 + 16 * id};
  catch err
    [code, message] = guatiao.internal.status_of(err);
    r = {code, message, 0};
  end
end
