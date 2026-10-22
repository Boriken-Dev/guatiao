% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Instance < handle
% An instance a provider built from a configuration (Registry.create).
% close() reaches the provider that built it, even after a retire or a
% refresh loaded a new build under its key; the last reference going
% closes it too.

  properties (SetAccess = private)
    Key = ''
  end

  properties (Hidden, SetAccess = private)
    Registry = []
    Id = 0
  end

  methods
    function self = Instance(registry, key, id)
      self.Registry = registry;
      self.Key = key;
      self.Id = id;
    end

    function close(self)
      if self.Id ~= 0
        guatiao_mex('instance_destroy', self.Id);
        self.Id = 0;
      end
    end

    function delete(self)
      try
        close(self);
      catch
        % The gateway is gone at the end of a session.
      end
    end

    function tf = is_open(self)
      tf = self.Id ~= 0 && self.Registry.is_open();
    end

    function obj = bind(self, cls)
    % A remote of the kind class `cls` over this instance, valid until the
    % instance is closed.
      if self.Id == 0
        error('guatiao:Gone:closed', 'the instance is closed');
      end
      e = guatiao.internal.kinds('of', cls);
      t = self.Registry.provider_table(self.Key, e.desc.name);
      obj = guatiao.internal.bind_table(cls, t, guatiao_mex('instance_ctx', self.Id), self);
    end
  end
end
