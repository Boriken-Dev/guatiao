% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [v, problem] = parse(text, varargin)
% Parses a document into a guatiao.Value; a number keeps its exact text.
%
%   v = guatiao.serde.parse('{"a": 1}')
%   v = guatiao.serde.parse(text, 'toml')
%   [v, problem] = guatiao.serde.parse(text, 'yaml')   % no error: v is [] and
%                                                      % problem says where
% The format is 'json' (default), 'toml' or 'yaml' (TOML and YAML need a
% guatiao_serde built with them). Options: 'ReadDataUris' true to read
% data: URIs back as bytes; 'Alloc' an allocator. A document that does not
% parse raises guatiao:BadValue:parse, its message naming the line and
% column (from 1; the column in characters) and never quoting the text;
% with two outputs it answers problem = struct(format, line, column,
% reason) instead.
  [format, rest] = guatiao.internal.format_arg(varargin);
  o = guatiao.internal.options(rest, struct('ReadDataUris', false, 'Alloc', []));
  how = 0;
  if o.ReadDataUris
    how = 256;
  end
  soft = nargout >= 2;
  [id, problem] = guatiao_mex('serde_parse', format, text, how, guatiao.internal.alloc_id(o.Alloc), soft);
  if isempty(id)
    v = [];
  else
    v = guatiao.Value.adopt(id);
    problem = [];
  end
end
