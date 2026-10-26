% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_greeters()
% One behaviour, many languages: a MATLAB or Octave host calls the
% greeters written in Rust, C, Python, Go and M through one description
% of the kind, and a Rust caller calls each of them through the kind's
% Rust proxy.
  cases = {
    'the kinds copy their floor hashes and sizes from the header', @header
    'one refresh loads every greeter its sources serve', @refresh_all
    'a host calls the Rust greeter', @() exercise('rust')
    'a host calls the C greeter', @() exercise('c')
    'a host calls the Python greeter', @() exercise('python')
    'a host calls the Go greeter', @() exercise('go')
    'a host calls the M greeter', @() exercise('m')
    'an answer built in the library''s allocator outlives the unload', @careless_answer
    'what a slot grew a lent map with outlives the unload', @lent_grown
    'a refused move of a lent tree is the call''s error', @refused_settle
    'a closed value passed to a slot is refused', @closed_argument
    'the Rust proxy calls every greeter', @rust_proxy
    'a table that is not the kind''s is refused before it runs', @floor_refused
    'nothing an M greeter served is left once its holders are done', @nothing_left
  };
end

function g = greeters()
  g = {'rust', 'derived_greeter_hello', 'hello, ana'
       'c', 'c_greeter', 'hello from C, ana'
       'python', 'py_greeter_hello', 'hello, ana'
       'go', 'go_greeter_hello', 'hello, ana'
       'm', 'mlgreeter_hello', 'hello, ana'};
end

function [reg, present] = host(set_reg, set_present)
% The registry every case shares, and which greeters it serves.
  persistent the_reg the_present
  if nargin
    the_reg = set_reg;
    the_present = set_present;
  end
  if isempty(the_reg)
    error('gt_test:fail', 'the greeter host was not made: run the refresh case first');
  end
  reg = the_reg;
  present = the_present;
end

function header()
  gt_env('examples');
  text = read_text_file(fullfile(repo_root(), 'examples', 'greeter_kind', 'include', 'greeter_kind.h'));
  pairs = {'greeter_vtable', 'greeterkind.Greeter'; 'counter_vtable', 'greeterkind.Counter'; ...
           'conversation_vtable', 'greeterkind.Conversation'; 'listener_vtable', 'greeterkind.Listener'; ...
           'wide_vtable', 'greeterkind.Wide'};
  for i = 1:size(pairs, 1)
    tok = regexp(text, ['#define ' pairs{i, 1} '_FLOOR_HASH ([0-9]+)'], 'tokens', 'once');
    e = guatiao.internal.kinds('of', pairs{i, 2});
    expect_equal(str2double(tok{1}), e.desc.floor_hash, [pairs{i, 1} ' floor hash']);
    body = regexp(text, ['typedef struct ' pairs{i, 1} ' \{(.*?)\} ' pairs{i, 1} ';'], 'tokens', 'once');
    slots = numel(regexp(body{1}, '\(\*\w+\)'));
    [~, most] = computer();
    expect_equal(e.size, 8 + (4 + 4 * (most > 2^32)) * slots, [pairs{i, 1} ' size']);
  end
end

function refresh_all()
  gt_env('examples');
  reg = guatiao.Registry('m-greeters', '0.1');
  % Every greeter written in another language comes through a source the
  % registry keeps: the workspace's folder and the C and Go greeters' as
  % watched folders, the Python greeters through the guatiao-source-py bridge.
  reg.watch_dir(gt_env('dir'), 'kind=greeter');
  present = {'rust'};
  missing = {};
  c_dir = gt_env('c_greeter');
  if isempty(c_dir)
    missing{end + 1} = 'the C greeter: no C compiler built it (set GUATIAO_CC or GUATIAO_C_GREETER)';
  else
    reg.watch_dir(c_dir, 'kind=greeter');
    present{end + 1} = 'c';
  end
  go = gt_env('go');
  if isempty(go)
    missing{end + 1} = 'the Go greeter: GUATIAO_GO_LIBRARY names no folder holding it built with -buildmode=c-shared';
  else
    reg.watch_dir(go, 'kind=greeter');
    present{end + 1} = 'go';
  end
  py = gt_env('bridge');
  if isempty(py)
    missing{end + 1} = 'the Python greeter: no guatiao-source-py build (set GUATIAO_PY_BRIDGE)';
  else
    reg.add_source_file(py, gt_env('bridge_config'));
    present{end + 1} = 'python';
  end
  refreshed = reg.refresh();
  expect_equal(numel(refreshed.refused), 0, 'nothing refused');
  loaded = cellfun(@(s) s.key, refreshed.loaded, 'UniformOutput', false);
  expect_true(any(strcmp(loaded, 'derived_greeter')), 'the Rust greeter');
  if any(strcmp(present, 'c')), expect_true(any(strcmp(loaded, 'c_greeter_library')), 'the C greeter'); end
  if any(strcmp(present, 'python')), expect_true(any(strcmp(loaded, 'py_greeter')), 'the Python greeter'); end
  if any(strcmp(present, 'go')), expect_true(any(~cellfun(@isempty, regexp(loaded, 'go_greeter$'))), 'the Go greeter'); end
  reg.register_library(guatiao.library_of('mlgreeter'));
  present{end + 1} = 'm';
  host(reg, present);
  if ~isempty(missing)
    skip_case(strjoin(missing, '; '));
  end
end

function exercise(which)
  [reg, present] = host();
  if ~any(strcmp(present, which))
    skip_case(sprintf('the %s greeter is not loaded here', which));
  end
  rows = greeters();
  row = rows(strcmp(rows(:, 1), which), :);
  key = row{2};
  want = row{3};
  g = reg.bind(key, 'greeterkind.Greeter');
  expect_true(isa(g, 'greeterkind.Greeter'), 'a remote of the kind');
  answer = g.greet('ana');
  expect_equal(answer.data(), struct('greeting', want), [which ' greet']);
  answer.close();
  e = expect_error(@() g.greet(''), 'guatiao:BadValue', [which ' refuses nobody']);
  expect_true(~isempty(strfind(e.message, 'nobody to greet')), [which ' says why']);
  if strcmp(which, 'c')
    a = g.greet('panic');   % the greeter written in C greets anyone
    a.close();
  else
    expect_error(@() g.greet('panic'), 'guatiao:Internal', [which ' fails']);
  end
  config = guatiao.Value(struct('name', 'bo'));
  expect_equal(g.check(config), true, [which ' check']);
  expect_equal(config.at('name').data(), 'bo');
  expect_equal(config.length(), 2, [which ' check filled the lent map']);
  config.close();
  handed = guatiao.Value(struct('name', 'ana'));
  answer = g.greet_from(handed);
  expect_equal(answer.data(), struct('greeting', want), [which ' greet_from']);
  expect_true(handed.isabsent(), [which ' the map was handed over']);
  answer.close();
  handed.close();
  e1 = Ear();
  g.greet_into('ana', e1);
  expect_equal(e1.Heard, {want}, [which ' greet_into']);
  if g.has_slot('shout')
    expect_equal(g.shout('ana'), upper(want), [which ' shout']);
  else
    expect_error(@() g.shout('ana'), 'guatiao:Unsupported');
  end
  if g.has_slot('start')
    listener = Ear();
    chat = g.start(listener);
    expect_true(isa(chat, 'greeterkind.Conversation'), [which ' a conversation']);
    chat.say('hi');
    chat.say('again');
    expect_error(@() chat.say(''), 'guatiao:BadValue', [which ' says nothing']);
    buf = guatiao.Buffer(64);
    n = chat.read(buf);
    expect_equal(native2unicode(buf.Data(1:n), 'UTF-8'), sprintf('hi\nagain'), [which ' read']);
    expect_equal(chat.turns(), 2, [which ' turns']);
    expect_equal(listener.Heard, {'hi', 'again'}, [which ' the listener heard']);
    chat.close();
    guatiao.internal.drain();
    expect_true(listener.Closed, [which ' the listener was let go']);
  else
    expect_equal(which, 'c', 'only the C greeter starts no conversation');
  end
end

% Each of these unloads a library of its own: no other registry keeps it mapped.
function careless_answer()
  gt_env('examples');
  dir = gt_env('c_greeter_own', 'GREETER_IGNORES_CARRIER');
  if isempty(dir)
    skip_case('the C greeter: no C compiler built it (set GUATIAO_CC)');
  end
  own = guatiao.Registry('m-careless', '0.1');
  scan = own.scan_dir(dir, 'kind=greeter');
  expect_equal(numel(scan.failed), 0, 'the careless greeter loads');
  g = own.bind('c_greeter', 'greeterkind.Greeter');
  answer = g.greet('hal');
  % The C greeter has no `unload` function: nothing it handed out is
  % alive, which the caller states.
  own.unload('c_greeter_library', true);
  expect_equal(numel(own.libraries()), 0, 'unmapped');
  expect_equal(answer.data(), struct('greeting', 'hello from C, hal'), 'the answer outlives it');
  answer.close();
  own.close();
end

function lent_grown()
  gt_env('examples');
  % This greeter's check rebuilds the lent map in its own allocator.
  dir = gt_env('c_greeter_own', 'GREETER_IGNORES_CARRIER');
  if isempty(dir)
    skip_case('the C greeter: no C compiler built it (set GUATIAO_CC)');
  end
  own = guatiao.Registry('m-lent', '0.1');
  scan = own.scan_dir(dir, 'kind=greeter');
  expect_equal(numel(scan.failed), 0, 'the greeter loads');
  g = own.bind('c_greeter', 'greeterkind.Greeter');
  config = guatiao.Value(struct('name', 'ivy'));
  expect_equal(g.check(config), true, 'check');
  own.unload('c_greeter_library', true);
  expect_equal(config.data(), struct('name', 'ivy', 'checked_by', 'C'), 'the lent map outlives it');
  config.close();
  own.close();
end

function refused_settle()
  gt_env('examples');
  % This greeter's check rebuilds the lent map in its own allocator; moving
  % it back into the map's, which refuses, fails the call and leaves the
  % map whole where it stands.
  dir = gt_env('c_greeter_own', 'GREETER_IGNORES_CARRIER');
  if isempty(dir)
    skip_case('the C greeter: no C compiler built it (set GUATIAO_CC)');
  end
  own = guatiao.Registry('m-refused', '0.1');
  scan = own.scan_dir(dir, 'kind=greeter');
  expect_equal(numel(scan.failed), 0, 'the greeter loads');
  g = own.bind('c_greeter', 'greeterkind.Greeter');
  a = guatiao.Allocator.counting();
  config = guatiao.Value(struct('name', 'ivy'), 'Alloc', a);
  a.refuse(true);
  try
    e = expect_error(@() g.check(config), 'guatiao:AllocFailed', 'the refused settle');
  catch failure
    a.refuse(false);
    config.close();
    own.close();
    rethrow(failure);
  end
  a.refuse(false);
  expect_true(~isempty(strfind(e.message, 'lent tree')), 'the error names the lent tree');
  expect_equal(config.data(), struct('name', 'ivy', 'checked_by', 'C'), 'the map stands whole');
  config.close();
  expect_equal(a.live(), 0, 'all back');
  own.close();
end

function closed_argument()
  [reg, ~] = host();
  g = reg.bind('derived_greeter_hello', 'greeterkind.Greeter');
  config = guatiao.Value(struct('name', 'bo'));
  config.close();
  expect_error(@() g.check(config), 'guatiao:Gone', 'a closed value is not lent');
  expect_error(@() g.greet_from(config), 'guatiao:Gone', 'nor handed over');
end

function rust_proxy()
  [reg, present] = host();
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  rows = greeters();
  for r = 1:size(rows, 1)
    which = rows{r, 1};
    if ~any(strcmp(present, which))
      continue
    end
    out = guatiao.Value();
    st = guatiao.serving(@() gtfixture('greeter_caller', gt_env('workspace', 'greeter_caller'), reg.host(), ...
                                       rows{r, 2}, 'ana', guatiao_mex('node_pointer', out.owner_id(), {})));
    expect_equal(st, 0, [which ' greeter_caller_run']);
    report = out.data();
    out.close();
    expect_equal(report.greet, rows{r, 3}, [which ' greet']);
    expect_equal(report.refusal.status, 7, [which ' refusal']);
    if strcmp(which, 'c')
      expect_equal(report.failure, 0, 'the C greeter greets anyone');
    else
      expect_equal(report.failure, 100, [which ' failure']);
    end
    expect_equal(report.shout, upper(rows{r, 3}), [which ' shout']);
    expect_equal(report.checked, true, [which ' checked']);
    expect_equal(numel(fieldnames(report.filled)), 2, [which ' filled']);
    expect_equal(report.greet_from, rows{r, 3}, [which ' greet_from']);
    expect_equal(report.heard, {rows{r, 3}}, [which ' heard']);
    if strcmp(which, 'c')
      % No `start` in its table: the Rust proxy runs the kind's default.
      expect_equal(report.conversation, struct('status', 11, 'message', 'this greeter holds no conversations'));
    else
      expect_equal(report.conversation.turns, 2, [which ' turns']);
      expect_equal(report.conversation.transcript, sprintf('hi\nagain'), [which ' transcript']);
      expect_equal(report.conversation.heard, {'hi', 'again'}, [which ' conversation heard']);
      expect_equal(report.conversation.dropped, true, [which ' dropped']);
    end
  end
end

function floor_refused()
  reg = host();
  t = reg.provider_table('mlgreeter_hello', 'counter');
  expect_error(@() guatiao.internal.bind_table('greeterkind.Greeter', t, [], reg), 'guatiao:WrongKind:floor');
  expect_error(@() reg.bind('c_greeter', 'greeterkind.Counter'), 'guatiao:WrongKind');
end

function nothing_left()
  reg = host();
  before = guatiao.internal.held('count');
  g = reg.bind('mlgreeter_hello', 'greeterkind.Greeter');
  chat = g.start(Ear());
  expect_equal(guatiao.internal.held('count'), before + 2, 'the conversation, and the listener it holds');
  chat.close();
  guatiao.internal.drain();
  expect_equal(guatiao.internal.held('count'), before, 'both let go');
end
