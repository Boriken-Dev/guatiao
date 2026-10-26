% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [x, problem] = data(text, varargin)
% Reads a document straight into MATLAB data: what parse(text).data()
% answers, with no guatiao.Value built (the document crosses once, as
% wire bytes read in the gateway).
%
%   x = guatiao.serde.data('{"a": 1}')                    % struct('a', 1)
%   x = guatiao.serde.data(text, 'yaml', 'Maps', 'map')
%   [x, problem] = guatiao.serde.data(text)   % no error: x is [] and
%                                             % problem says where
% The format is 'json' (default), 'toml' or 'yaml', as for parse.
% Options: 'ReadDataUris' true to read data: URIs back as bytes; 'Numbers'
% and 'Maps' as data() reads them. A document that does not parse raises
% guatiao:BadValue:parse, as parse does; with two outputs problem is
% struct(format, line, column, reason) instead, and [] when it parsed.
  [format, rest] = guatiao.internal.format_arg(varargin);
  o = guatiao.internal.options(rest, struct('ReadDataUris', false, 'Numbers', 'auto', 'Maps', 'auto'));
  [numbers, maps] = guatiao.internal.read_options('Numbers', o.Numbers, 'Maps', o.Maps);
  how = 0;
  if o.ReadDataUris
    how = 256;
  end
  [x, problem] = guatiao_mex('serde_data', format, text, how, numbers, maps, nargout >= 2);
end
