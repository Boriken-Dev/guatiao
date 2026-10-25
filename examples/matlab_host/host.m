% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function host(varargin)
% A plugin host written in M, for MATLAB and GNU Octave.
%
% Scans the folders it is given for libraries declaring the greeter kind,
% whatever language wrote them, and calls each greeter through the kind's
% table: a map back, a map lent and changed in place, a map handed over,
% an object handed in and one handed back, an object lent for one call. A
% provider that builds instances from a configuration is given one its
% schema refuses, then one it accepts. The greeters written in M are
% registered beside them, and the greeters written in Python are served by
% the guatiao-source-py bridge when one is named. Everything it opened is closed
% before it returns, and it prints what it did.
%
%   host('target/debug', 'c-greeter')
%   host('target/debug', '--python-source', BRIDGE, '--python-dir', DIR, '--python-path', DIR)
%
% The guatiao package, the greeterkind kinds and the mlgreeter library are
% put on the path from this checkout when they are not on it already.
  here = fileparts(mfilename('fullpath'));
  repo = fileparts(fileparts(here));
  if exist('guatiao_mex') ~= 3 %#ok<EXIST>
    addpath(fullfile(repo, 'bindings', 'matlab', 'guatiao', 'inst'));
  end
  addpath(fullfile(repo, 'examples', 'greeter_kind', 'matlab'));
  addpath(fullfile(repo, 'examples', 'matlab_provider', 'mlgreeter'));

  dirs = {};
  python_source = '';
  python_dirs = {};
  python_paths = {};
  i = 1;
  while i <= numel(varargin)
    a = varargin{i};
    switch a
      case '--python-source'
        python_source = varargin{i + 1};
        i = i + 2;
      case '--python-dir'
        python_dirs{end + 1} = varargin{i + 1}; %#ok<AGROW>
        i = i + 2;
      case '--python-path'
        python_paths{end + 1} = varargin{i + 1}; %#ok<AGROW>
        i = i + 2;
      otherwise
        dirs{end + 1} = a; %#ok<AGROW>
        i = i + 1;
    end
  end

  held_before = guatiao.internal.held('count');
  reg = guatiao.Registry('m-host', '1.0');
  for k = 1:numel(dirs)
    report = reg.scan_dir(dirs{k}, 'kind=greeter');
    for f = 1:numel(report.loaded)
      entry = report.loaded{f};
      if isstruct(entry)
        entry = entry.path;
      end
      [~, name, ext] = fileparts(entry);
      fprintf('loaded %s%s\n', name, ext);
    end
  end
  lib = guatiao.library_of('mlgreeter');
  reg.register_library(lib);
  fprintf('registered %s %s from M\n', lib.Id, lib.Version);
  if ~isempty(python_source)
    reg.add_source_file(python_source, struct('dirs', {python_dirs}, 'paths', {python_paths}));
    changed = reg.refresh();
    for k = 1:numel(changed.loaded)
      fprintf('served %s from Python\n', changed.loaded{k}.id);
    end
    for k = 1:numel(changed.refused)
      fprintf('refused %s: %s\n', changed.refused{k}.id, changed.refused{k}.why);
    end
  end

  providers = reg.providers('greeter');
  for k = 1:numel(providers)
    p = providers{k};
    fprintf('\n%s (%s %s)\n', p.key, p.library, p.version);
    tell = @(varargin) fprintf('  %s\n', sprintf(varargin{:}));
    try
      g = reg.bind(p.key, 'greeterkind.Greeter');
    catch err
      if ~isempty(strfind(err.identifier, 'floor'))
        tell('not this greeter kind: %s', err.message);
        continue
      end
      rethrow(err);
    end
    if ~p.has_config
      exercise(g, tell);
      continue
    end
    % Built from a configuration: one the schema refuses, then one it accepts.
    try
      inst = reg.create(p.key, struct());
      inst.close();
    catch err
      tell('an empty configuration is refused: %s', regexprep(err.message, '^guatiao_mex: ', ''));
    end
    inst = reg.create(p.key, struct('prefix', 'hey'));
    tell('instance from prefix = hey');
    exercise(inst.bind('greeterkind.Greeter'), tell);
    inst.close();
  end

  % Unmapped where the library agrees; one with no say in it is retired,
  % and stays mapped.
  fprintf('\n');
  libraries = reg.libraries();
  for k = 1:numel(libraries)
    key = libraries{k}.key;
    try
      reg.unload(key);
      fprintf('unloaded %s\n', key);
    catch err
      reg.retire(key);
      parts = strsplit(err.identifier, ':');
      fprintf('retired %s, still mapped: unload said %s\n', key, parts{2});
    end
  end
  reg.close();
  % What this host built for itself, once no registry can call it.
  lib.close();
  guatiao.internal.drain();
  fprintf('objects a library can still reach: %d\n', guatiao.internal.held('count') - held_before);
end

function exercise(g, tell)
  answer = g.greet('ana');
  tell('greet: %s', answer.at('greeting').data());
  answer.close();
  try
    g.greet('');
  catch err
    parts = strsplit(err.identifier, ':');
    tell('greet('''') refused: %s: %s', parts{2}, regexprep(err.message, '^guatiao_mex: ', ''));
  end
  % The other slots were appended to the kind: an older table may lack them.
  slots = {'shout', 'check', 'greet_from', 'greet_into', 'start'};
  missing = slots(~cellfun(@(s) g.has_slot(s), slots));
  if ~isempty(missing)
    tell('an older table, with no %s', strjoin(missing, ', '));
  end
  if g.has_slot('shout')
    tell('shout: %s', g.shout('ana'));
  end
  if g.has_slot('check')
    config = guatiao.Value(struct('name', 'bo'));
    ok = g.check(config);
    word = 'refused';
    if ok
      word = 'accepted';
    end
    tell('check lent a map and it came back %s: %s', word, strjoin(sort(config.keys()), ', '));
    config.close();
  end
  if g.has_slot('greet_from')
    handed = guatiao.Value(struct('name', 'cy'));
    answer = g.greet_from(handed);
    tell('greet_from took a map: %s', answer.at('greeting').data());
    answer.close();
    handed.close();
  end
  if g.has_slot('greet_into')
    e = HostEar();
    g.greet_into('di', e);
    tell('greet_into told a listener: %s', strjoin(e.Heard, ', '));
  end
  if g.has_slot('start')
    listener = HostEar();
    try
      chat = g.start(listener);
    catch err
      tell('start refused: %s', err.message);
      return
    end
    chat.say('hi');
    chat.say('again');
    buf = guatiao.Buffer(64);
    n = chat.read(buf);
    text = native2unicode(buf.Data(1:n), 'UTF-8');
    tell('a conversation of %d turns: "%s"', chat.turns(), strrep(text, sprintf('\n'), '\n'));
    chat.close();
    guatiao.internal.drain();
    let_go = '';
    if listener.Closed
      let_go = ' and was let go';
    end
    tell('closed it; the listener heard %s%s', strjoin(listener.Heard, ', '), let_go);
  end
end
