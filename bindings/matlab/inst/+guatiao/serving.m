% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function varargout = serving(fn)
% Runs `fn` (a function handle taking no arguments) inside a call into
% guatiao, and answers what it answers. A slot an M object serves runs only
% while such a call is in progress on the interpreter's thread: C code of
% your own that calls guatiao tables directly (a MEX file, a library called
% through calllib) reaches M providers when it runs inside serving.
%
%   n = guatiao.serving(@() mymex('greet', table, ctx));
  box = guatiao.internal.Box();
  n = max(nargout, 0);
  guatiao_mex('serving', @() capture(fn, box, n));
  if ~isempty(box.Error)
    rethrow(box.Error);
  end
  varargout = box.Out;
end

function capture(fn, box, n)
  try
    if n == 0
      fn();
    else
      out = cell(1, n);
      [out{:}] = fn();
      box.Out = out;
    end
  catch err
    box.Error = err;
  end
end
