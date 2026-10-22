% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function v = redact(schema, value, placeholder, varargin)
% A new guatiao.Value: `value` with every field the schema marks
% (x-sensitive or writeOnly) replaced by `placeholder` (default
% '[redacted]') at any depth, and whatever the schema does not describe
% hidden too. Local $refs are followed. Option 'Alloc': an allocator.
  if nargin < 3 || isempty(placeholder)
    placeholder = '[redacted]';
  end
  alloc = guatiao.internal.option(varargin, 'Alloc', []);
  v = guatiao.Value.adopt(guatiao_mex('redact', schema, value, placeholder, guatiao.internal.alloc_id(alloc)));
end
