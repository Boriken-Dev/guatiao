% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function id = alloc_id(alloc)
% The gateway's id of an allocator argument: [] for the binding's own.
  if isempty(alloc)
    id = [];
  elseif isa(alloc, 'guatiao.Allocator')
    id = alloc.Id;
  else
    error('guatiao:WrongKind', 'Alloc is a guatiao.Allocator');
  end
end
