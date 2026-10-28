% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function s = slot(name, args, varargin)
% One slot of a kind, for guatiao.Kind.describe: its name, its arguments
% as a cell of name-shape pairs in order, then options.
%
%   guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true)
%   guatiao.slot('read', {'dst', guatiao.lent('bytes')}, 'Returns', 'int64')
%   guatiao.slot('start', {'listener', guatiao.taken('greeterkind.Listener')}, 'Returns', 'greeterkind.Conversation', 'Fallible', true)
%
% Shapes: 'string', 'bytes', 'bool', 'int8' .. 'uint64', 'float', 'double',
% 'size', 'intptr', 'value', 'map'; guatiao.lent(...) of 'value',
% 'map', 'list', 'bytes' or an object kind's class; guatiao.taken(...) of
% 'value', 'map', 'list' or an object kind's class. Options: 'Returns' (a
% shape that is not 'bytes', or an object kind's class; default none),
% 'Required' (a slot every table fills, wherever it is), 'Fallible'
% (the slot ends in guatiao_provider_error *err).
  if nargin < 2
    args = {};
  end
  if ~ischar(name) || ~isvarname(name)
    error('guatiao:BadValue', 'a slot''s name is a MATLAB identifier');
  end
  if ~iscell(args) || mod(numel(args), 2) ~= 0
    error('guatiao:BadValue', 'a slot''s arguments are a cell of name-shape pairs');
  end
  o = guatiao.internal.options(varargin, struct('Returns', [], 'Required', false, 'Fallible', false));
  names = args(1:2:end);
  if ~all(cellfun(@(n) ischar(n) && isvarname(n), names))
    error('guatiao:BadValue', 'every argument of %s is named by an identifier', name);
  end
  s = struct('name', name, 'args', {args}, 'returns', {o.Returns}, 'required', logical(o.Required), ...
             'fallible', logical(o.Fallible));
end
