% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function r = dispatch(kind_id, i, ctx, args, answer)
% Called by a served slot on the interpreter's thread: finds the object
% its ctx names, calls the slot's method, writes the answer. Answers
% {status, message, buffers}; it raises nothing.
  e = guatiao.internal.kinds('by_id', kind_id);
  c = e.codes{i};
  bufs = cell(1, numel(c.args));
  try
    obj = [];
    if ctx >= 65536 && mod(ctx - 65536, 16) == 0
      obj = guatiao.internal.held('get', (ctx - 65536) / 16);
    end
    if isempty(obj)
      % What the call handed over is let go with it.
      for j = 1:numel(c.args)
        if any(c.args(j) == [40 41 42])
          guatiao_mex('value_close', args{j});
        elseif c.args(j) == 44
          guatiao_mex('remote_close', args{j});
        end
      end
      r = {12, 'the object was released', bufs};
      return
    end
    for j = 1:numel(c.args)
      switch c.args(j)
        case {20, 21, 22, 30, 31, 32}
          args{j} = guatiao.Value.adopt(args{j}, true);
        case {40, 41, 42}
          args{j} = guatiao.Value.adopt(args{j});
        case 33
          args{j} = guatiao.Buffer(args{j});
        case 34
          args{j} = guatiao.internal.remote_of(c.arg_kinds{j}, args{j}, []);
        case 44
          args{j} = guatiao.internal.remote_of(c.arg_kinds{j}, args{j}, []);
      end
    end
    if c.ret == 0
      feval(e.names{i}, obj, args{:});
    else
      result = feval(e.names{i}, obj, args{:});
      if c.ret == 50
        [result, ~] = guatiao.internal.object_parts(c.ret_kind, result);
      end
      guatiao_mex('answer', answer, result);
    end
    r = {0, '', bufs};
  catch err
    [code, message] = guatiao.internal.status_of(err);
    r = {code, message, bufs};
  end
  % What a method left in a lent buffer goes back to the caller's, and a
  % lent object's remote is let go without destroying it.
  for j = 1:numel(c.args)
    if c.args(j) == 33 && isa(args{j}, 'guatiao.Buffer')
      r{3}{j} = args{j}.Data;
    elseif c.args(j) == 34 && isa(args{j}, 'guatiao.Kind')
      args{j}.detach();
    end
  end
end
