% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_examples()
% The example host runs as its README says.
  cases = {
    'the M host example runs', @host_runs
  };
end

function host_runs()
  addpath(fullfile(repo_root(), 'examples', 'matlab_host'));
  args = {gt_env('library')};
  c_dir = gt_env('c_greeter');
  if ~isempty(c_dir)
    args{end + 1} = c_dir;
  end
  go = gt_env('go');
  if ~isempty(go)
    args{end + 1} = go;
  end
  py = gt_env('bridge');
  if ~isempty(py)
    cfg = gt_env('bridge_config');
    args = [args, {'--python-source', py, '--python-dir', cfg.dirs{1}, ...
                   '--python-path', cfg.paths{1}, '--python-path', cfg.paths{2}}];
  end
  text = evalc('host(args{:})');
  has = @(s) expect_true(~isempty(strfind(text, s)), s);
  has('registered mlgreeter 1.0.0 from M');
  has(sprintf('mlgreeter_hello (mlgreeter 1.0.0)\n  greet: hello, ana'));
  has('derived_greeter_hello');
  has('an empty configuration is refused');
  has('not this greeter kind');   % hello_library's table is another kind's
  has('a conversation of 2 turns: "hi\nagain"');
  has('closed it; the listener heard hi, again and was let go');
  if ~isempty(c_dir)
    has(sprintf('c_greeter (c_greeter_library 1.0)\n  greet: hello from C, ana'));
  end
  if ~isempty(go)
    has('go_greeter_hello');
  end
  if ~isempty(py)
    has('served py_greeter from Python');
  end
  has('retired mlgreeter, still mapped: unload said WrongKind');
  expect_true(~isempty(regexp(text, 'objects a library can still reach: 0\s*$', 'once')), 'nothing left');
end
