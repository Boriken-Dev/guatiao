% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = gt_env(op, varargin)
% What several test files share: where the library and the checkout are,
% the example packages on the path, and C built for the tests that need
% it (each built once a session).
%
%   gt_env('dir')                     the folder of workspace libraries: GUATIAO_LIBRARY,
%                                     or the folder of the file it names
%   gt_env('core')                    the file GUATIAO_LIBRARY gives the core
%   gt_env('file', base)              a workspace library's file name
%   gt_env('workspace', base)         its path in that folder
%   gt_env('examples')                puts the example packages on the path
%   gt_env('c_greeter')               the folder holding the C greeter, or ''
%   gt_env('c_greeter_own', define)   a fresh build of the C greeter in a folder of its own, with
%                                     -D<define> when given (not kept: a test unloads it), or ''
%   gt_env('c_source', name)          the C shelf source built as `name`, or ''
%   gt_env('fixture')                 whether the gtfixture MEX file is built
%   gt_env('bridge')                  GUATIAO_PY_BRIDGE when it names a file, or ''
%   gt_env('bridge_config')           the bridge's configuration for the examples
%   gt_env('go')                      GUATIAO_GO_LIBRARY, or ''
  persistent built
  if isempty(built)
    built = struct();
  end
  switch op
    case 'dir'
      out = lib_dir();
    case 'core'
      % A file serves the core as it is; a folder holds it under its own name.
      out = getenv('GUATIAO_LIBRARY');
      if exist(out, 'dir') == 7
        out = fullfile(out, lib_file('guatiao'));
      end
    case 'file'
      out = lib_file(varargin{1});
    case 'workspace'
      out = fullfile(lib_dir(), lib_file(varargin{1}));
    case 'examples'
      addpath(fullfile(repo_root(), 'examples', 'greeter_kind', 'matlab'));
      addpath(fullfile(repo_root(), 'examples', 'tally_kind', 'matlab'));
      addpath(fullfile(repo_root(), 'examples', 'matlab_provider', 'mlgreeter'));
      out = true;
    case 'c_greeter'
      if ~isfield(built, 'c_greeter')
        named = getenv('GUATIAO_C_GREETER');
        if ~isempty(named)
          built.c_greeter = named;
        else
          dir = scratch('c-greeter');
          src = fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'c_consumer', 'greeter_in_c.c');
          if build_c(src, fullfile(dir, lib_file('c_greeter')), false)
            built.c_greeter = dir;
          else
            built.c_greeter = '';
          end
        end
      end
      out = built.c_greeter;
    case 'c_greeter_own'
      src = fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'c_consumer', 'greeter_in_c.c');
      dir = scratch('c-greeter-own');
      if build_c(src, fullfile(dir, lib_file('c_greeter')), false, varargin{:})
        out = dir;
      else
        out = '';
      end
    case 'c_source'
      name = varargin{1};
      key = ['src_' name];
      if ~isfield(built, key)
        src = fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'c_consumer', 'source_in_c.c');
        path = fullfile(scratch('c-shelf'), lib_file(name));
        if build_c(src, path, false)
          built.(key) = path;
        else
          built.(key) = '';
        end
      end
      out = built.(key);
    case 'fixture'
      if ~isfield(built, 'fixture')
        built.fixture = build_fixture();
      end
      out = built.fixture;
    case 'bridge'
      out = getenv('GUATIAO_PY_BRIDGE');
      if ~isempty(out) && exist(out, 'file') ~= 2
        out = '';
      end
    case 'bridge_config'
      out = struct('dirs', {{fullfile(repo_root(), 'examples', 'python_provider')}}, ...
                   'paths', {{fullfile(repo_root(), 'bindings', 'python', 'guatiao', 'src'), ...
                              fullfile(repo_root(), 'examples', 'greeter_kind', 'python')}});
    case 'go'
      out = getenv('GUATIAO_GO_LIBRARY');
    otherwise
      error('gt_test:fail', 'gt_env: no %s', op);
  end
end

function d = lib_dir()
% GUATIAO_LIBRARY may name one file carrying every surface; the example
% libraries are built beside it.
  d = getenv('GUATIAO_LIBRARY');
  if exist(d, 'dir') ~= 7
    d = fileparts(d);
  end
end

function f = lib_file(base)
  if ispc()
    f = [base '.dll'];
  elseif ismac()
    f = ['lib' base '.dylib'];
  else
    f = ['lib' base '.so'];
  end
end

function d = scratch(name)
  d = tempname();
  d = [d '-' name];
  mkdir(d);
end

function cc = compiler()
% GUATIAO_CC, else Octave's own compiler, else cc where there is one.
  cc = getenv('GUATIAO_CC');
  if ~isempty(cc)
    return
  end
  if exist('OCTAVE_VERSION', 'builtin')
    cc = strtrim(mkoctfile('-p', 'CC'));
  elseif isunix()
    cc = 'cc';
  end
end

function ok = build_c(source, out, strict, define)
% A shared library from one C file, with the guatiao and greeter headers
% and -D<define> when one is given.
  ok = false;
  cc = compiler();
  if isempty(cc)
    return
  end
  incs = sprintf('-I"%s" -I"%s"', fullfile(repo_root(), 'crates', 'guatiao', 'include'), ...
                 fullfile(repo_root(), 'examples', 'greeter_kind', 'include'));
  flags = '-shared -Wall -Wextra';
  if ~ispc()
    flags = [flags ' -fPIC'];
  end
  if strict
    flags = [flags ' -Werror'];
  end
  if nargin > 3
    flags = [flags ' -D' define];
  end
  libs = '';
  if isunix() && ~ismac()
    libs = '-lpthread';
  end
  [status, msg] = system(sprintf('"%s" %s %s -o "%s" "%s" %s', cc, flags, incs, out, source, libs));
  ok = status == 0 && exist(out, 'file') == 2;
  if ~ok
    fprintf('building %s failed:\n%s\n', source, msg);
  end
end

function ok = build_fixture()
% tests/fixtures/gtfixture.c as a MEX file beside the tests, with the
% warnings CI turns into errors.
  here = fileparts(mfilename('fullpath'));
  src = fullfile(here, 'fixtures', 'gtfixture.c');
  incs = {['-I' fullfile(repo_root(), 'crates', 'guatiao', 'include')], ...
          ['-I' fullfile(repo_root(), 'examples', 'greeter_kind', 'include')]};
  out = fullfile(tempdir(), sprintf('gtfixture-%d', round(rand() * 1e9)));
  mkdir(out);
  try
    if exist('OCTAVE_VERSION', 'builtin')
      args = [{'--mex', '-o', fullfile(out, 'gtfixture.mex'), '-Wall', '-Wextra', '-Werror'}, incs, {src}];
      if isunix() && ~ismac()
        args{end + 1} = '-lpthread';
      end
      [msg, status] = mkoctfile(args{:});
      if status ~= 0
        fprintf('building the fixture failed:\n%s\n', msg);
        ok = false;
        return
      end
    else
      cc = mex.getCompilerConfigurations('C', 'Selected');
      extra = {};
      if ~isempty(cc) && isempty(strfind(lower(cc.Name), 'microsoft'))
        extra = {'CFLAGS=$CFLAGS -Wall -Wextra -Werror'};
      end
      mex('-silent', '-R2018a', '-outdir', out, '-output', 'gtfixture', extra{:}, incs{:}, src);
    end
    addpath(out);
    ok = true;
  catch err
    fprintf('building the fixture failed: %s\n', err.message);
    ok = false;
  end
end
