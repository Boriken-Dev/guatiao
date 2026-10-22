% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function text = emit(x, varargin)
% Writes a guatiao value, a view or MATLAB data as text.
%
%   guatiao.serde.emit(v)                       % JSON
%   guatiao.serde.emit(v, 'yaml')
%   guatiao.serde.emit(v, 'json', 'Pretty', true, 'Bytes', 'base64')
%
% The format is 'json' (default), 'toml' or 'yaml'. Options: 'Pretty'
% (JSON only); 'Bytes', how bytes are written: 'data_uri' (default),
% 'base64', 'array' or 'refuse'. TOML writes only a map, and no null.
  [format, rest] = guatiao.internal.format_arg(varargin);
  o = guatiao.internal.options(rest, struct('Pretty', false, 'Bytes', 'data_uri'));
  how = find(strcmpi(o.Bytes, {'data_uri', 'base64', 'array', 'refuse'})) - 1;
  if isempty(how)
    error('guatiao:BadValue', 'Bytes is ''data_uri'', ''base64'', ''array'' or ''refuse''');
  end
  text = guatiao_mex('serde_emit', format, x, how, logical(o.Pretty));
end
