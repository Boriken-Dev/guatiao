% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Library < handle
% A library written in M: its id, version, providers and meta. Usually
% made by guatiao.library_of; guatiao.Library(id, version, providers,
% 'Meta', meta) is the primitive, `providers` a cell of provider classes
% (their Provider constant is read) or guatiao.provider specs with Class
% set. Its descriptor is built on first use and kept for the session, its
% unload answering Busy while an instance lives.

  properties (SetAccess = private)
    Id = ''
    Version = ''
    Providers = {}    % the provider specs, Class set
    ProviderIds = {}
    Meta = []
  end

  properties (Hidden, SetAccess = private)
    Cid = 0           % the gateway's id of the descriptor
    Entry = []        % its entry function's address
    Defaults = []     % ids of the providers' own instances
    Closed = false
  end

  methods
    function self = Library(id, version, providers, varargin)
      o = guatiao.internal.options(varargin, struct('Meta', []));
      if ~ischar(id) || isempty(id) || ~ischar(version)
        error('guatiao:BadValue', 'a library has an id and a version (text)');
      end
      if ischar(providers)
        providers = {providers};
      end
      if isempty(providers)
        error('guatiao:BadValue', 'a library offers at least one provider');
      end
      specs = cellfun(@guatiao.internal.provider_spec, providers, 'UniformOutput', false);
      parts = strsplit(id, '.');
      short = parts{end};
      ids = cell(1, numel(specs));
      for i = 1:numel(specs)
        s = specs{i};
        if isempty(s.Id)
          ids{i} = [short '_' s.Name];
        else
          ids{i} = s.Id;
        end
      end
      if numel(unique(ids)) ~= numel(ids)
        error('guatiao:BadValue', 'two providers of %s share an id: %s', id, strjoin(ids, ', '));
      end
      for i = 1:numel(specs)
        s = specs{i};
        for k = 1:numel(s.Serves)
          guatiao.internal.served_table(s.Serves{k}, s.Class);   % refuses a missing required slot
        end
      end
      self.Id = id;
      self.Version = version;
      self.Providers = specs;
      self.ProviderIds = ids;
      self.Meta = o.Meta;
    end

    function e = entry(self)
    % The address of the library's entry function, building its descriptor
    % once.
      if ~isempty(self.Entry)
        e = self.Entry;
        return
      end
      if self.Closed
        error('guatiao:Gone', 'the library %s is closed', self.Id);
      end
      n = numel(self.Providers);
      descs = struct('id', cell(1, n), 'display_name', '', 'version', '', 'kinds', {{}}, 'tables', {{}}, ...
                     'sizes', [], 'schema', [], 'meta', [], 'available', false, 'configured', false, ...
                     'ctx', 0, 'index', 0);
      for i = 1:n
        s = self.Providers{i};
        kinds = s.Serves;
        tables = cell(1, numel(kinds));
        sizes = zeros(1, numel(kinds));
        names = cell(1, numel(kinds));
        for k = 1:numel(kinds)
          e = guatiao.internal.kinds('of', kinds{k});
          tables{k} = guatiao.internal.served_table(kinds{k}, s.Class);
          sizes(k) = e.size;
          names{k} = e.desc.name;
        end
        schema = [];
        if ~isempty(s.ConfigSchema)
          schema = s.ConfigSchema;
        elseif ~isempty(s.Config)
          schema = s.Config.schema();
        end
        index = guatiao.internal.providers('add', struct('spec', s, 'library', self));
        ctx = 0;
        if s.Default
          obj = guatiao.internal.make(s, []);
          id = guatiao.internal.held('hold', obj);
          self.Defaults(end + 1) = id;
          ctx = 65536 + 16 * id;
        end
        descs(i).id = self.ProviderIds{i};
        descs(i).display_name = s.DisplayName;
        descs(i).version = s.Version;
        descs(i).kinds = names;
        descs(i).tables = tables;
        descs(i).sizes = sizes;
        descs(i).schema = schema;
        descs(i).meta = s.Meta;
        descs(i).available = ~isempty(s.Available);
        descs(i).configured = ~isempty(s.Config) || ~isempty(s.ConfigSchema);
        descs(i).ctx = ctx;
        descs(i).index = index;
      end
      self.Cid = guatiao_mex('library_new', self.Id, self.Version, self.Meta, descs);
      self.Entry = guatiao_mex('library_entry', self.Cid);
      e = self.Entry;
    end

    function n = live_instances(self)
    % Instances hosts built and have not destroyed yet.
      guatiao.internal.drain();
      if self.Cid == 0
        n = 0;
      else
        n = guatiao_mex('library_live', self.Cid);
      end
    end

    function close(self)
    % Lets go of each provider's own instance; a host asking for the
    % descriptor afterwards is declined.
      if self.Closed
        return
      end
      self.Closed = true;
      if self.Cid ~= 0
        guatiao_mex('library_close', self.Cid);
      end
      for id = self.Defaults
        guatiao.internal.held('release', id, true);
      end
      self.Defaults = [];
    end

    function tf = is_closed(self)
      tf = self.Closed;
    end
  end

  methods (Hidden)
    function a = info(self)
    % The guatiao_library_info * its entry answers, for a test.
      self.entry();
      a = guatiao_mex('library_info', self.Cid);
    end
  end
end
