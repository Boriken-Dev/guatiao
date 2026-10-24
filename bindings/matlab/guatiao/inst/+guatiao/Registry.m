% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Registry < handle
% A host's registry of plugin libraries. Every answer that is data is
% plain MATLAB data, already freed.
%
%   reg = guatiao.Registry('my-host', '1.0');
%   reg.scan_dir('/path/to/plugins', 'kind=greeter');
%   reg.providers('greeter')                  % a cell of structs, best first
%   g = reg.bind('acme_hello', 'greeterkind.Greeter');   % a remote
%   inst = reg.create('acme_shouter', struct('prefix', 'hey'));
%   s = inst.bind('greeterkind.Greeter');
%   inst.close(); reg.close();
%
% guatiao.Registry(id, version, 'Alloc', a): `a` is the allocator offered
% to libraries.

  properties (Hidden, SetAccess = private)
    Id = 0
    Kept = {}   % libraries written in M that this registry registered
  end

  methods
    function self = Registry(id, version, varargin)
      alloc = guatiao.internal.option(varargin, 'Alloc', []);
      self.Id = guatiao_mex('registry_new', id, version, guatiao.internal.alloc_id(alloc));
    end

    function close(self)
    % Releases the registry; the libraries it loaded stay mapped.
      if self.Id ~= 0
        guatiao_mex('registry_free', self.Id);
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
      tf = self.Id ~= 0 && guatiao_mex('registry_is_open', self.Id);
    end

    function r = load_file(self, path)
    % Loads one library file: a struct with loaded, skipped or failed.
      r = guatiao_mex('registry_load_file', self.id(), path);
    end

    function r = register_entry(self, name, entry)
    % Registers a library linked rather than loaded, from its entry
    % function's address (a uint64).
      r = guatiao_mex('registry_register_entry', self.id(), name, entry);
    end

    function r = register_library(self, lib)
    % Registers a library written in M (guatiao.library_of or
    % guatiao.Library) under its id. Linked, not mapped: unload answers
    % WrongKind and retire takes it out.
      if ischar(lib)
        lib = guatiao.library_of(lib);
      end
      if ~isa(lib, 'guatiao.Library')
        error('guatiao:WrongKind', 'register_library takes a guatiao.Library');
      end
      self.Kept{end + 1} = lib;
      r = self.register_entry(lib.Id, lib.entry());
    end

    function r = scan_dir(self, dir, rules, descending)
    % Scans a folder: struct(loaded, skipped, failed). `rules`: one
    % [!]KEY=VALUE per line, matched against what a library declares
    % before it is mapped.
      if nargin < 3, rules = ''; end
      if nargin < 4, descending = false; end
      r = guatiao_mex('registry_scan', self.id(), dir, rules, logical(descending), false);
    end

    function r = scan_path(self, spec, rules, descending)
    % Walks a search path of folders and files.
      if nargin < 3, rules = ''; end
      if nargin < 4, descending = false; end
      r = guatiao_mex('registry_scan', self.id(), spec, rules, logical(descending), true);
    end

    function r = libraries(self)
    % Every library loaded.
      r = guatiao_mex('registry_query', self.id(), 0, '');
    end

    function r = providers(self, kind)
    % Every provider serving `kind` (every one when omitted), best first.
      if nargin < 2, kind = ''; end
      r = guatiao_mex('registry_query', self.id(), 1, kind);
    end

    function r = available(self, kind)
    % Every provider serving `kind` that can run here now, best first.
      if nargin < 2, kind = ''; end
      r = guatiao_mex('registry_query', self.id(), 2, kind);
    end

    function r = why_not(self, kind)
    % Why nothing can serve `kind`: struct(available, why, providers).
      r = guatiao_mex('registry_query', self.id(), 3, kind);
    end

    function r = provider(self, key)
    % One provider, by the key the registry filed it under.
      r = guatiao_mex('registry_query', self.id(), 4, key);
    end

    function r = best(self, kind)
    % The best provider serving `kind` that can run here.
      r = guatiao_mex('registry_query', self.id(), 5, kind);
    end

    function r = provider_available(self, key)
    % struct(available, reason) for one provider, asked now.
      r = guatiao_mex('registry_provider_available', self.id(), key);
    end

    function keyed_by(self, template)
    % Names providers with `template` (%id, %name, %library, %version).
      guatiao_mex('registry_keyed_by', self.id(), template, false);
    end

    function libraries_keyed_by(self, template)
    % Names libraries with `template` (%id, %version).
      guatiao_mex('registry_keyed_by', self.id(), template, true);
    end

    function set_priority(self, id, priority)
    % Ranks every provider with this id.
      guatiao_mex('registry_set_priority', self.id(), id, priority);
    end

    function p = priority(self, id)
    % What the host ranked that id; 0 unless it said otherwise.
      p = guatiao_mex('registry_priority', self.id(), id);
    end

    function t = provider_table(self, key, kind)
    % struct(table, size, ctx): the table a provider speaks `kind` through
    % (table [] for none); without `kind`, its single table.
      if nargin < 3, kind = []; end
      t = guatiao_mex('registry_provider_table', self.id(), key, kind);
    end

    function v = provider_config(self, key)
    % A copy of the provider's configuration schema (a guatiao.Value), or [].
      id = guatiao_mex('registry_provider_config', self.id(), key);
      if isempty(id)
        v = [];
      else
        v = guatiao.Value.adopt(id);
      end
    end

    function v = provider_meta(self, key)
    % What the provider declared beyond its schema (its notices, or its form
    % under guatiao.intake.provider_form_key) as a guatiao.Value: a copy,
    % outliving the library, or [] when it declares nothing.
    % guatiao:NotFound for an unknown key.
      id = guatiao_mex('registry_provider_meta', self.id(), key);
      if isempty(id)
        v = [];
      else
        v = guatiao.Value.adopt(id);
      end
    end

    function h = host(self)
    % The registry's guatiao_host_info *, as a uint64.
      h = guatiao_mex('registry_host', self.id());
    end

    function inst = create(self, key, config)
    % Builds an instance of a provider from a configuration (MATLAB data or
    % a guatiao value), checked against its schema first.
      if nargin < 3 || (isstruct(config) && isempty(fieldnames(config)))
        config = struct();
      end
      inst = guatiao.Instance(self, key, guatiao_mex('registry_create', self.id(), key, config));
    end

    function obj = bind(self, key, cls)
    % A remote of the kind class `cls` over the provider filed under `key`,
    % valid while the registry is open and the library loaded.
      e = guatiao.internal.kinds('of', cls);
      t = self.provider_table(key, e.desc.name);
      obj = guatiao.internal.bind_table(cls, t, [], self);
    end

    function retire(self, key)
    % Takes a library out (by its library key) and leaves it mapped until
    % nothing uses it.
      guatiao_mex('registry_retire', self.id(), key);
    end

    function keys = collect(self)
    % Closes every retired library whose own unload agrees; answers their keys.
      keys = guatiao_mex('registry_collect', self.id());
    end

    function unload(self, key, unchecked)
    % Unmaps a library when it agrees: Busy while something it handed out
    % lives, Unsupported with no unload function, WrongKind for a library
    % linked rather than mapped, NotFound for none.
      if nargin < 3, unchecked = false; end
      guatiao_mex('registry_unload', self.id(), key, logical(unchecked));
    end

    function add_source_file(self, path, config)
    % Maps a file exporting guatiao_source (a bridge to another language)
    % and adds the source it answers.
      if nargin < 3, config = struct(); end
      guatiao_mex('registry_add_source_file', self.id(), path, config);
    end

    function watch_dir(self, dir, rules)
    % Watches a folder: each refresh loads what appeared and retires what
    % vanished.
      if nargin < 3, rules = ''; end
      guatiao_mex('registry_watch_dir', self.id(), dir, rules);
    end

    function r = refresh(self)
    % Makes the registry agree with every source: struct(loaded, retired,
    % skipped, refused). Nothing refreshes on its own.
      r = guatiao_mex('registry_refresh', self.id());
    end

    function disp(self)
      if self.is_open()
        fprintf('  guatiao.Registry\n');
      else
        fprintf('  guatiao.Registry (closed)\n');
      end
    end
  end

  methods (Hidden)
    function i = id(self)
      guatiao.internal.drain();
      if self.Id == 0
        error('guatiao:Gone:closed', 'the registry is closed');
      end
      i = self.Id;
    end
  end
end
