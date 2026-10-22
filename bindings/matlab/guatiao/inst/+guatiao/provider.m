% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function spec = provider(varargin)
% A provider's declaration, the value of its class's Constant Provider
% property:
%
%   classdef Hello < greeterkind.Greeter & greeterkind.Counter
%     properties (Constant, Hidden)
%       Provider = guatiao.provider('DisplayName', 'Hello, in M')
%     end
%     ...
%
% Options:
%   'Serves'       the kind classes it serves (default: the kinds it inherits)
%   'Name'         default: the class's name in snake case
%   'Id'           default: <library id>_<name>
%   'DisplayName', 'Version' (empty: the library's), 'Meta' (a struct)
%   'Config'       a guatiao.Config: one instance per configuration, built
%                  by the class's constructor from the checked struct
%   'ConfigSchema' a JSON Schema as MATLAB data: the constructor gets the
%                  configuration as plain data
%   'Make'         a function building the instance (from the configuration
%                  when there is one) instead of the constructor
%   'Default'      also offer the instance built with no configuration
%   'Available'    a function answering [] when the provider can run here,
%                  and why not otherwise; asked every time a host asks
  spec = guatiao.internal.options(varargin, struct('Serves', {{}}, 'Name', '', 'Id', '', 'DisplayName', '', ...
                                                   'Version', '', 'Meta', [], 'Config', [], 'ConfigSchema', [], ...
                                                   'Make', [], 'Default', [], 'Available', []));
  if ischar(spec.Serves)
    spec.Serves = {spec.Serves};
  end
  if ~isempty(spec.Config) && ~isa(spec.Config, 'guatiao.Config')
    error('guatiao:BadValue', 'Config is a guatiao.Config');
  end
  if ~isempty(spec.Available) && ~isa(spec.Available, 'function_handle')
    error('guatiao:BadValue', 'Available is a function');
  end
  if ~isempty(spec.Make) && ~isa(spec.Make, 'function_handle')
    error('guatiao:BadValue', 'Make is a function');
  end
  spec.Class = '';
end
