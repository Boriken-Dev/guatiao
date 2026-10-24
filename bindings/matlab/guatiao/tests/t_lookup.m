% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_lookup()
% Where serde's library is looked for. The gateway reads the environment
% once a process and keeps what it opened, so each case asks a new
% MATLAB or Octave. Serde opens the core first: the own-variable cases
% name a copy of the core with no serde beside it, which only the own
% variable can get past.
  cases = {
    'serde''s own variable names a file, whatever it is called', @own_file
    'serde''s own variable names a folder', @own_folder
    'GUATIAO_LIBRARY naming a file without serde points at its sibling', @sibling
    'GUATIAO_LIBRARY naming a file carrying serde serves it as it is', @carrying
  };
end

function own_file()
  serde = built('guatiao_serde');
  renamed = copy_of(serde, 'renamed');
  core = copy_of(built('guatiao'), 'core');
  served = serde_served_with('GUATIAO_SERDE_LIBRARY', renamed, 'GUATIAO_LIBRARY', core);
  delete(renamed);
  delete(core);
  expect_equal(served, slashes(renamed), 'the file named');
end

function own_folder()
  serde = built('guatiao_serde');
  core = copy_of(built('guatiao'), 'core');
  served = serde_served_with('GUATIAO_SERDE_LIBRARY', gt_env('dir'), 'GUATIAO_LIBRARY', core);
  delete(core);
  expect_equal(served, slashes(serde), 'serde in the folder named');
end

function sibling()
  serde = built('guatiao_serde');
  served = serde_served_with('GUATIAO_SERDE_LIBRARY', '', 'GUATIAO_LIBRARY', built('guatiao'));
  expect_equal(served, slashes(serde), 'the sibling');
end

function carrying()
  app = gt_env('workspace', 'myapp');
  if exist(app, 'file') ~= 2
    skip_case(['no application library (examples/app_library) built: ' app]);
  end
  served = serde_served_with('GUATIAO_SERDE_LIBRARY', '', 'GUATIAO_LIBRARY', app);
  expect_equal(served, slashes(app), 'the file named');
end

function path = built(base)
  path = gt_env('workspace', base);
  if exist(path, 'file') ~= 2
    skip_case(['not built: ' path]);
  end
end

function path = copy_of(file, what)
% Named with an extension no loader appends to: Windows adds .dll to a
% name with none.
  path = [tempname() '-guatiao-lookup-' what '.bin'];
  copy_bytes(file, path);
end

function served = serde_served_with(var, value, other, other_value)
% The file a new session opens serde from, with `var` and `other` set (an
% empty value leaves one unset) and the rest of the environment as it is.
  names = {var, other};
  values = {value, other_value};
  saved = cellfun(@getenv, names, 'UniformOutput', false);
  sets = '';
  if ispc()
    % Set for the child by cmd itself: a line that opens with a quote
    % loses its first and last under `cmd /c`.
    for k = 1:2
      sets = [sets sprintf('set "%s=%s" & ', names{k}, values{k})]; %#ok<AGROW>
    end
  else
    for k = 1:2
      setenv(names{k}, values{k});
    end
  end
  gateway = fileparts(which('guatiao_mex'));
  script = sprintf(['addpath(''%s''); guatiao_mex(''has_export'', 1, ''guatiao_json_parse_ex''); ' ...
                    'fprintf(''served: %%s\\n'', guatiao_mex(''library_path'', 1));'], gateway);
  if exist('OCTAVE_VERSION', 'builtin')
    command = sprintf('"%s" --norc --quiet --no-window-system --eval "%s"', ...
                      fullfile(OCTAVE_HOME(), 'bin', 'octave-cli'), script);
  else
    command = sprintf('"%s" -batch "%s"', fullfile(matlabroot(), 'bin', 'matlab'), script);
  end
  [~, out] = system([sets command]);
  for k = 1:2
    setenv(names{k}, saved{k});
  end
  at = strfind(out, 'served: ');
  if isempty(at)
    error('gt_test:fail', 'the session found no serde:\n%s', out);
  end
  served = slashes(strtrim(strtok(out(at(end) + 8:end), sprintf('\n'))));
end

function p = slashes(p)
  p = strrep(p, '\', '/');
end

function copy_bytes(from, to)
  in = fopen(from, 'r');
  bytes = fread(in, Inf, '*uint8');
  fclose(in);
  out = fopen(to, 'w');
  fwrite(out, bytes, 'uint8');
  fclose(out);
end
