% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = build(varargin)
% Compiles the gateway, guatiao_mex, from the package's C sources: once,
% after installing, with a C compiler and libffi. Octave builds it with
% mkoctfile --mex, MATLAB with mex (-R2018a). Answers the file written.
%
%   guatiao.build()
%   guatiao.build('FfiDir', 'C:\vcpkg\installed\x64-windows-static-md')
%
% Options:
%   'Output'  the folder to write it to (default: beside the package)
%   'FfiDir'  a libffi prefix (include/, lib/) when the compiler does not
%             find it; default GUATIAO_FFI_DIR, else the system's
%   'Strict'  true to compile with warnings as errors (gcc and clang)
%   'Flags'   a cell of further arguments for mex or mkoctfile
%   'Verbose' true to print the commands
  here = fileparts(fileparts(mfilename('fullpath')));
  o = guatiao.internal.options(varargin, struct('Output', here, 'FfiDir', getenv('GUATIAO_FFI_DIR'), ...
                                                'Strict', false, 'Flags', {{}}, 'Verbose', false));
  src = guatiao.internal.source_dir();
  files = {'guatiao_mex.c', 'core.c', 'convert.c', 'values.c', 'forms.c', 'native.c', ...
           'kinds.c', 'provide.c', 'registry.c'};
  files = files(cellfun(@(f) exist(fullfile(src, f), 'file') == 2, files));
  paths = cellfun(@(f) fullfile(src, f), files, 'UniformOutput', false);

  octave = exist('OCTAVE_VERSION', 'builtin') ~= 0;
  args = {['-I' src]};
  if ~isempty(o.FfiDir)
    args = [args, {['-I' fullfile(o.FfiDir, 'include')], ['-L' fullfile(o.FfiDir, 'lib')]}];
    if ispc() && ~octave
      args = [args, {'-DFFI_STATIC_BUILD'}];
    end
  elseif ismac()
    [st, sdk] = system('xcrun --show-sdk-path');
    if st == 0
      args = [args, {['-I' strtrim(sdk) '/usr/include/ffi']}];
    end
  elseif isunix()
    [st, cflags] = system('pkg-config --cflags libffi 2>/dev/null');
    if st == 0 && ~isempty(strtrim(cflags))
      args = [args, strsplit(strtrim(cflags), ' ')];
    end
  end
  libs = {'-lffi'};
  if isunix() && ~ismac()
    libs = [libs, {'-ldl', '-lpthread'}];
  end

  if octave
    target = fullfile(o.Output, 'guatiao_mex.mex');
    warn = {'-Wall', '-Wextra'};
    if o.Strict
      warn = [warn, {'-Werror'}];
    end
    cmd = [{'--mex', '-o', target}, warn, args, o.Flags, paths, libs];
    if o.Verbose
      disp(strjoin([{'mkoctfile'}, cmd], ' '));
    end
    [msg, status] = mkoctfile(cmd{:});
    if status ~= 0
      error('guatiao:Internal', 'mkoctfile failed:\n%s', msg);
    end
  else
    target = fullfile(o.Output, ['guatiao_mex.' mexext()]);
    cc = mex.getCompilerConfigurations('C', 'Selected');
    extra = {};
    if ~isempty(cc) && ~contains(lower(cc.Name), 'microsoft')
      warn = '-Wall -Wextra';
      if o.Strict
        warn = [warn ' -Werror'];
      end
      extra = {['CFLAGS=$CFLAGS ' warn]};
    end
    if ispc() && ~isempty(cc) && contains(lower(cc.Name), 'microsoft')
      libs = {'-lffi'};
    end
    cmd = [{'-R2018a', '-outdir', o.Output, '-output', 'guatiao_mex'}, extra, args, o.Flags, paths, libs];
    if o.Verbose
      disp(strjoin([{'mex'}, cmd], ' '));
      cmd = [{'-v'}, cmd];
    else
      cmd = [{'-silent'}, cmd];
    end
    mex(cmd{:});
  end
  clear('guatiao_mex');
  if nargout
    out = target;
  end
end
