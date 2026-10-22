% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function lib = library_of(where, varargin)
% A library written in M, read from the package conventions MATLAB and
% Octave have: `where` is a folder, an installed Octave package's name, or
% the name of a namespace (a +folder) on the path.
%
%   lib = guatiao.library_of('mlgreeter');
%   reg.register_library(lib);
%
% Its id, version and meta come from the folder's DESCRIPTION (Octave's
% package format; packinfo/DESCRIPTION once installed): Name, Version,
% Title as summary, Description, Author as authors, Maintainer, License,
% Url as urls.homepage. Without one, MATLAB's conventions: Contents.m's
% '% Version x.y.z' line, an installed toolbox's version. Its COPYING,
% LICENSE, LICENCE and NOTICE files (and their .md and .txt forms) become
% meta.notices. Its providers are the classes in its namespaces that carry
% a Provider constant, in name order.
%
% Options: 'Id', 'Version' (override or supply them), 'Meta' (a struct
% merged last), 'Providers' (a cell of classes, in order). Built once per
% folder and session.
  persistent built
  if isempty(built)
    built = containers.Map('KeyType', 'char', 'ValueType', 'any');
  end
  o = guatiao.internal.options(varargin, struct('Id', '', 'Version', '', 'Meta', [], 'Providers', {{}}));
  [folder, ns] = locate(where);
  key = [folder '|' o.Id];
  if isKey(built, key)
    lib = built(key);
    if ~lib.is_closed()
      return
    end
  end
  desc = read_description(folder);
  id = o.Id;
  version = o.Version;
  meta = struct();
  if ~isempty(desc)
    if isempty(id) && isfield(desc, 'name'), id = desc.name; end
    if isempty(version) && isfield(desc, 'version'), version = desc.version; end
    meta = meta_of(desc);
  end
  if isempty(version)
    version = contents_version(folder);
  end
  if isempty(version)
    version = toolbox_version(where);
  end
  if isempty(id)
    id = ns;
  end
  if isempty(version)
    error('guatiao:BadValue', ['%s has no version: give it a DESCRIPTION (Version:), a Contents.m ' ...
          '(%% Version x.y.z), or say ''Version'''], folder);
  end
  notices = notices_of(folder, id);
  if ~isempty(notices)
    meta.notices = notices;
  end
  if ~isempty(o.Meta)
    names = fieldnames(o.Meta);
    for i = 1:numel(names)
      meta.(names{i}) = o.Meta.(names{i});
    end
  end
  providers = o.Providers;
  if isempty(providers)
    providers = discover(folder);
  end
  if isempty(providers)
    error('guatiao:BadValue', '%s declares no provider (a class with a Constant Provider in one of its namespaces)', id);
  end
  if isempty(fieldnames(meta))
    meta = [];
  end
  lib = guatiao.Library(id, version, providers, 'Meta', meta);
  built(key) = lib;
end

function [folder, ns] = locate(where)
  ns = where;
  if exist(where, 'dir') == 7 && ~isempty(dir(where))
    folder = absolute(where);
    [~, ns] = fileparts(folder);
    return
  end
  if exist('OCTAVE_VERSION', 'builtin')
    try
      found = pkg('list', where);
      if ~isempty(found)
        folder = found{1}.dir;
        return
      end
    catch
      % pkg cannot answer here: look on the path.
    end
  end
  dirs = strsplit(path(), pathsep());
  for i = 1:numel(dirs)
    if ~isempty(dirs{i}) && exist(fullfile(dirs{i}, ['+' where]), 'dir') == 7
      folder = dirs{i};
      return
    end
  end
  error('guatiao:NotFound', 'no folder, Octave package or namespace on the path is called %s', where);
end

function p = absolute(p)
  here = pwd();
  cd(p);
  p = pwd();
  cd(here);
end

function d = read_description(folder)
% Octave's DESCRIPTION format: "Field: value", continuation lines indented.
  d = [];
  candidates = {fullfile(folder, 'DESCRIPTION'), fullfile(folder, 'packinfo', 'DESCRIPTION')};
  for i = 1:numel(candidates)
    if exist(candidates{i}, 'file') == 2
      text = read_utf8(candidates{i});
      d = struct();
      last = '';
      lines = regexp(text, '\r?\n', 'split');
      for k = 1:numel(lines)
        line = lines{k};
        if isempty(strtrim(line)) || line(1) == '#'
          continue
        end
        if any(line(1) == [' ', sprintf('\t')]) && ~isempty(last)
          d.(last) = [d.(last) ' ' strtrim(line)];
          continue
        end
        colon = find(line == ':', 1);
        if isempty(colon)
          continue
        end
        last = lower(strtrim(line(1:colon - 1)));
        last = regexprep(last, '[^a-z0-9_]', '_');
        d.(last) = strtrim(line(colon + 1:end));
      end
      return
    end
  end
end

function m = meta_of(d)
  m = struct();
  pairs = {'title', 'summary'; 'description', 'description'; 'maintainer', 'maintainer'; 'license', 'license'};
  for i = 1:size(pairs, 1)
    if isfield(d, pairs{i, 1})
      m.(pairs{i, 2}) = regexprep(d.(pairs{i, 1}), '\s+', ' ');
    end
  end
  if isfield(d, 'author')
    m.authors = {regexprep(d.author, '\s+', ' ')};
  end
  if isfield(d, 'url')
    urls = regexp(strtrim(d.url), '[,\s]+', 'split');
    m.urls = struct('homepage', urls{1});
  end
end

function v = contents_version(folder)
  v = '';
  f = fullfile(folder, 'Contents.m');
  if exist(f, 'file') ~= 2
    return
  end
  tok = regexp(read_utf8(f), '%\s*Version\s+([0-9][^\s]*)', 'tokens', 'once');
  if ~isempty(tok)
    v = tok{1};
  end
end

function v = toolbox_version(name)
  v = '';
  if exist('OCTAVE_VERSION', 'builtin')
    return
  end
  try
    t = matlab.addons.toolbox.installedToolboxes();
    for i = 1:numel(t)
      if strcmp(t(i).Name, name)
        v = t(i).Version;
        return
      end
    end
  catch
    % No toolbox registry here.
  end
end

function notices = notices_of(folder, component)
  notices = {};
  names = {'COPYING', 'LICENSE', 'LICENCE', 'NOTICE'};
  exts = {'', '.md', '.txt'};
  dirs = {folder, fullfile(folder, 'packinfo')};
  for d = 1:numel(dirs)
    for i = 1:numel(names)
      for k = 1:numel(exts)
        f = fullfile(dirs{d}, [names{i} exts{k}]);
        if exist(f, 'file') == 2
          notices{end + 1} = struct('component', component, 'text', read_utf8(f), 'format', 'text'); %#ok<AGROW>
        end
      end
    end
    if ~isempty(notices)
      return
    end
  end
end

function t = read_utf8(f)
  fid = fopen(f, 'r');
  bytes = fread(fid, Inf, '*uint8')';
  fclose(fid);
  t = native2unicode(bytes, 'UTF-8');
end

function classes = discover(folder)
% Classes in the folder's namespaces that carry a Provider constant.
  classes = {};
  entries = dir(fullfile(folder, '+*'));
  for i = 1:numel(entries)
    if ~entries(i).isdir
      continue
    end
    ns = entries(i).name(2:end);
    files = dir(fullfile(folder, entries(i).name, '*.m'));
    names = sort({files.name});
    for k = 1:numel(names)
      cls = [ns '.' names{k}(1:end - 2)];
      mc = meta.class.fromName(cls);
      if isempty(mc)
        continue
      end
      props = mc.PropertyList;
      for p = 1:numel(props)
        pr = guatiao.internal.item(props, p);
        if strcmp(pr.Name, 'Provider') && pr.Constant
          classes{end + 1} = cls; %#ok<AGROW>
          break
        end
      end
    end
  end
end
