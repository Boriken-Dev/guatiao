% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = providers(op, varargin)
% The providers whose descriptors this session built, by the index their
% slots carry: index = providers('add', rec); rec = providers('get', index).
  persistent recs
  if isempty(recs)
    recs = {};
  end
  switch op
    case 'add'
      recs{end + 1} = varargin{1};
      out = numel(recs);
    case 'get'
      out = recs{varargin{1}};
    otherwise
      error('guatiao:Internal', 'providers: no operation %s', op);
  end
end
