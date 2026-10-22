% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function out = call_slot(remote, slot, args)
% Calls the slot named `slot` of a remote's table with `args`, the slot's
% arguments in order: the primitive every kind method calls through
% guatiao.Kind's call.
  guatiao.internal.drain();
  holder = remote.Holder;
  if ~isempty(holder) && ~holder.is_open()
    error('guatiao:Gone:closed', 'the registry or instance this remote came from is closed');
  end
  [e, i] = guatiao.internal.kinds('slot', class(remote), slot);
  c = e.codes{i};
  if numel(args) ~= numel(c.args)
    error('guatiao:BadValue', '%s takes %d arguments', slot, numel(c.args));
  end
  buffers = {};
  held = {};
  lent_held = {};
  for j = 1:numel(c.args)
    code = c.args(j);
    if code == 33
      if ~isa(args{j}, 'guatiao.Buffer')
        error('guatiao:BadValue', 'argument %d of %s is a guatiao.Buffer', j, slot);
      end
      buffers{end + 1} = {j, args{j}}; %#ok<AGROW>
      args{j} = args{j}.Data;
    elseif code == 34 || code == 44
      [args{j}, id] = guatiao.internal.object_parts(c.arg_kinds{j}, args{j});
      if ~isempty(id)
        if code == 44
          held{end + 1} = id; %#ok<AGROW>
        else
          lent_held{end + 1} = id; %#ok<AGROW>
        end
      end
    end
  end
  try
    if isempty(buffers)
      result = guatiao_mex('remote_call', remote.RemoteId, i, args);
    else
      [result, back] = guatiao_mex('remote_call', remote.RemoteId, i, args);
    end
  catch err
    release_all(lent_held);
    % A call that never reached the table leaves what it would have handed over here.
    if isempty(strfind(err.identifier, ':provider'))
      release_all(held);
    end
    rethrow(err);
  end
  release_all(lent_held);
  for k = 1:numel(buffers)
    b = buffers{k};
    b{2}.Data = back{b{1}};
  end
  switch c.ret
    case 50
      out = guatiao.internal.remote_of(c.ret_kind, result, []);
    case {20, 21, 22}
      out = guatiao.Value.adopt(result);
    case 0
      out = [];
    otherwise
      out = result;
  end
end

function release_all(ids)
  for k = 1:numel(ids)
    guatiao.internal.held('release', ids{k}, false);
  end
end
