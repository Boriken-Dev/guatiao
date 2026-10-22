% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Shouter < greeterkind.Greeter
% Built from a configuration: one instance per configuration, its schema
% read from the field list below.

  properties (Constant, Hidden)
    Provider = guatiao.provider('Config', guatiao.Config( ...
      'prefix', guatiao.field('string', 'Doc', 'What every greeting starts with.', 'minLength', 1)))
  end

  properties
    prefix = ''
  end

  methods
    function self = Shouter(config)
      if nargin
        self.prefix = config.prefix;
      end
    end

    function out = greet(self, name)
      out = struct('greeting', [self.prefix ' ' upper(name)]);
    end
  end
end
