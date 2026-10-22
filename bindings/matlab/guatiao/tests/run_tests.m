% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function ok = run_tests(varargin)
% Runs every t_*.m in this folder, under MATLAB or Octave:
%
%   cd bindings/matlab/guatiao/tests; run_tests                 % all files
%   run_tests('t_values')                               % some
%   run_tests('Exit', true)                             % exit 1 on failure (CI)
%
% GUATIAO_LIBRARY names the library (default: the checkout's
% target/debug). The gateway is built first when it is not on the path.
% GUATIAO_REQUIRE_ALL=1 makes a skipped case a failure.
  here = fileparts(mfilename('fullpath'));
  exit_after = false;
  only = {};
  i = 1;
  while i <= numel(varargin)
    if strcmpi(varargin{i}, 'Exit')
      exit_after = logical(varargin{i + 1});
      i = i + 2;
    else
      only{end + 1} = varargin{i}; %#ok<AGROW>
      i = i + 1;
    end
  end
  addpath(fullfile(fileparts(here), 'inst'));
  addpath(here);
  if isempty(getenv('GUATIAO_LIBRARY'))
    setenv('GUATIAO_LIBRARY', fullfile(repo_root(), 'target', 'debug'));
  end
  if exist('guatiao_mex', 'file') ~= 3
    guatiao.build('Strict', true);
  end

  global GT_EXPECTATIONS
  GT_EXPECTATIONS = 0;
  files = dir(fullfile(here, 't_*.m'));
  names = sort({files.name});
  passed = 0;
  failed = 0;
  skipped = 0;
  failures = {};
  require_all = strcmp(getenv('GUATIAO_REQUIRE_ALL'), '1');
  t0 = tic;
  for f = 1:numel(names)
    file = names{f}(1:end - 2);
    if ~isempty(only) && ~any(strcmp(only, file))
      continue
    end
    cases = feval(file);
    for k = 1:size(cases, 1)
      label = sprintf('%s: %s', file, cases{k, 1});
      try
        cases{k, 2}();
        passed = passed + 1;
      catch e
        if strcmp(e.identifier, 'gt_test:skip')
          skipped = skipped + 1;
          fprintf('SKIP %s (%s)\n', label, e.message);
          if require_all
            failed = failed + 1;
            failures{end + 1} = sprintf('%s: skipped with GUATIAO_REQUIRE_ALL=1: %s', label, e.message); %#ok<AGROW>
          end
        else
          failed = failed + 1;
          where = '';
          if ~isempty(e.stack)
            where = sprintf(' (%s line %d)', e.stack(1).name, e.stack(1).line);
          end
          failures{end + 1} = sprintf('%s: [%s] %s%s', label, e.identifier, e.message, where); %#ok<AGROW>
          fprintf('FAIL %s\n', failures{end});
        end
      end
    end
  end
  fprintf('\n%d passed, %d failed, %d skipped (%d expectations) in %.1f s on %s\n', passed, failed, skipped, ...
          GT_EXPECTATIONS, toc(t0), host_name());
  for k = 1:numel(failures)
    fprintf('  %s\n', failures{k});
  end
  ok = failed == 0;
  if exit_after
    exit(double(~ok));
  end
end

function n = host_name()
  if exist('OCTAVE_VERSION', 'builtin')
    n = ['Octave ' OCTAVE_VERSION];
  else
    n = ['MATLAB ' version];
  end
end
