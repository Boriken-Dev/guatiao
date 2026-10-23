% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_wire()
% The wire encoding and channels against the shared vectors every
% implementation reads: each value both ways, each malformed input
% refused, the frames the builders write, and a conversation with every
% refusal.
  cases = {
    'every value decodes to its tree and encodes to its bytes', @every_value
    'every malformed input is refused', @every_malformed
    'a view encodes alone', @a_view
    'the frame builders write the documented bytes', @every_frame
    'the conversation runs through one receiver', @conversation
    'a receiver frees what it built', @frees
    'a closed receiver refuses', @closed
  };
end

function vecs = vectors()
  path = fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'vectors', 'wire.json');
  % guatiao's own parser keeps a NUL in a key and every number's text.
  vecs = guatiao.serde.parse(read_text_file(path));
end

function b = unhex(t)
  if isempty(t)
    b = zeros(1, 0, 'uint8');
  else
    b = uint8(hex2dec(reshape(t, 2, [])'))';
  end
end

function t = hex(b)
  b = uint8(b(:)');
  if isempty(b)
    t = '';
  else
    t = lower(reshape(dec2hex(b, 2)', 1, []));
  end
end

function t = text_hex(s)
  t = hex(unicode2native(s, 'UTF-8'));
end

function t = joined(parts)
  if isempty(parts)
    t = '';
  else
    t = strjoin(parts, ',');
  end
end

function t = describe(n)
% A node of a value as one line of text: the vectors' trees read alike.
  tag = n.tag();
  switch tag
    case {'absent', 'null'}
      t = tag;
    case 'bool'
      t = sprintf('bool(%d)', n.data());
    case 'number'
      x = n.data('Numbers', 'text');
      t = ['number(' x.Text ')'];
    case 'string'
      t = ['string(' text_hex(n.data()) ')'];
    case 'bytes'
      t = ['bytes(' hex(n.data()) ')'];
    case 'list'
      parts = cell(1, n.length());
      for i = 1:n.length()
        parts{i} = describe(n.at(i));
      end
      t = ['list[' joined(parts) ']'];
    otherwise
      k = n.keys();
      parts = cell(1, numel(k));
      for i = 1:numel(k)
        parts{i} = [text_hex(k{i}) '=' describe(n.at(i))];
      end
      t = ['map[' joined(parts) ']'];
  end
end

function t = tree_text(j)
% A vector tree ([tag] or [tag, x]) as describe writes a value.
  head = j.at(1);
  tag = head.data();
  if j.length() > 1
    arg = j.at(2);
  end
  switch tag
    case {'absent', 'null'}
      t = tag;
    case 'bool'
      t = sprintf('bool(%d)', arg.data());
    case 'number'
      t = ['number(' arg.data() ')'];
    case 'string'
      t = ['string(' text_hex(arg.data()) ')'];
    case 'bytes'
      t = ['bytes(' arg.data() ')'];
    case 'list'
      parts = cell(1, arg.length());
      for i = 1:arg.length()
        parts{i} = tree_text(arg.at(i));
      end
      t = ['list[' joined(parts) ']'];
    otherwise
      parts = cell(1, arg.length());
      for i = 1:arg.length()
        pair = arg.at(i);
        key = pair.at(1);
        parts{i} = [text_hex(key.data()) '=' tree_text(pair.at(2))];
      end
      t = ['map[' joined(parts) ']'];
  end
end

function v = build(j)
% The value a vector tree names, built from MATLAB data.
  head = j.at(1);
  tag = head.data();
  if j.length() > 1
    arg = j.at(2);
  end
  switch tag
    case 'absent'
      v = guatiao.Value();
    case 'null'
      v = guatiao.Value([]);
    case 'bool'
      v = guatiao.Value(logical(arg.data()));
    case 'number'
      v = guatiao.Value(guatiao.Number(arg.data()));
    case 'string'
      v = guatiao.Value(arg.data());
    case 'bytes'
      v = guatiao.Value(unhex(arg.data()));
    case 'list'
      v = guatiao.Value({});
      for i = 1:arg.length()
        child = build(arg.at(i));
        v.push(child);
        child.close();
      end
    otherwise
      v = guatiao.Value(struct());
      for i = 1:arg.length()
        pair = arg.at(i);
        key = pair.at(1);
        child = build(pair.at(2));
        v.set(key.data(), child);
        child.close();
      end
  end
end

function s = str_of(n, key, fallback)
  if n.has(key)
    x = n.at(key);
    s = x.data('Numbers', 'text');
    if isa(s, 'guatiao.Number')
      s = s.Text;
    end
  else
    s = fallback;
  end
end

function every_value()
  vecs = vectors();
  values = vecs.at('values');
  expect_true(values.length() > 0, 'there are vectors');
  for i = 1:values.length()
    c = values.at(i);
    name = str_of(c, 'name', '');
    bytes = unhex(str_of(c, 'hex', ''));
    want = tree_text(c.at('tree'));
    v = guatiao.Value.from_wire(bytes);
    expect_equal(describe(v), want, [name ': decoded']);
    expect_equal(v.to_wire(), bytes, [name ': encoded again']);
    v.close();
    b = build(c.at('tree'));
    expect_equal(describe(b), want, [name ': built']);
    expect_equal(b.to_wire(), bytes, [name ': built and encoded']);
    b.close();
  end
end

function every_malformed()
  vecs = vectors();
  cases = vecs.at('malformed');
  for i = 1:cases.length()
    c = cases.at(i);
    bytes = unhex(str_of(c, 'hex', ''));
    expect_error(@() guatiao.Value.from_wire(bytes), 'guatiao:BadValue:malformed', str_of(c, 'name', ''));
  end
end

function a_view()
  v = guatiao.Value(struct('outer', struct('inner', {{1, 'two'}})));
  outer = v.at('outer');
  back = guatiao.Value.from_wire(outer.to_wire());
  expect_equal(back.keys(), {'inner'}, 'the view alone');
  inner = back.at('inner');
  expect_equal(inner.data(), {1, 'two'});
  back.close();
  v.close();
end

function n = channel_of(frame)
% The channel a frame names, from its LEB128 bytes: a 64-bit channel does
% not survive a double.
  n = uint64(0);
  shift = 0;
  for i = 2:numel(frame)
    n = bitor(n, bitshift(uint64(bitand(frame(i), uint8(127))), shift));
    if bitand(frame(i), uint8(128)) == 0
      break
    end
    shift = shift + 7;
  end
end

function every_frame()
  vecs = vectors();
  frames = vecs.at('frames');
  for i = 1:frames.length()
    f = frames.at(i);
    want = unhex(str_of(f, 'hex', ''));
    channel = channel_of(want);
    build_kind = str_of(f, 'build', '');
    if strcmp(build_kind, 'close')
      expect_equal(guatiao.wire.close(channel), want, 'close');
      continue
    end
    tree = build(f.at('tree'));
    if strcmp(build_kind, 'announce')
      got = guatiao.wire.announce(channel, tree);
    else
      got = guatiao.wire.send(channel, tree);
    end
    tree.close();
    expect_equal(got, want, build_kind);
  end
  second = frames.at(2);
  expect_equal(guatiao.wire.send(7, struct('greeting', 'hello')), unhex(str_of(second, 'hex', '')), ...
               'MATLAB data, built for the call');
end

function run_conversation(r, vecs)
  ids = struct('malformed', 'guatiao:BadValue:malformed', 'no_schema', 'guatiao:NotFound:noSchema', ...
               'not_a_schema', 'guatiao:WrongKind:notASchema', 'invalid', 'guatiao:BadValue:invalid');
  names = struct('malformed', 'malformed', 'no_schema', 'noSchema', 'not_a_schema', 'notASchema', 'invalid', 'invalid');
  steps = vecs.at('conversation');
  for i = 1:steps.length()
    s = steps.at(i);
    frame = unhex(str_of(s, 'frame', ''));
    channel = uint64(str2double(str_of(s, 'channel', '0')));
    label = sprintf('step %d', i);
    if s.has('event')
      e = r.accept(frame);
      expect_equal(e.kind, str_of(s, 'event', ''), [label ' kind']);
      expect_equal(e.channel, channel, [label ' channel']);
      if s.has('tree')
        expect_equal(describe(e.value), tree_text(s.at('tree')), [label ' value']);
        e.value.close();
      else
        expect_equal(e.value, [], [label ' no value']);
      end
      continue
    end
    refusal = str_of(s, 'refusal', '');
    err = expect_error(@() r.accept(frame), ids.(refusal), label);
    expect_true(~isempty(err.message), [label ' words']);
    expect_true(isempty(strfind(err.message, 'hunter2')), [label ' quotes nothing']);
    [e, p] = r.accept(frame);
    expect_equal(e, [], [label ' soft']);
    expect_equal(p.refusal, names.(refusal), [label ' refusal']);
    expect_equal(p.channel, channel, [label ' channel']);
    expect_true(~isempty(p.message) && ~isempty(strfind(err.message, p.message)), [label ' message']);
    if s.has('kind')
      expect_equal(p.kind, str2double(str_of(s, 'kind', '')), [label ' kind']);
    end
    if s.has('at')
      expect_equal(p.at, str2double(str_of(s, 'at', '')), [label ' at']);
    end
    if s.has('key')
      expect_equal(p.key, str_of(s, 'key', ''), [label ' key']);
    end
  end
end

function conversation()
  vecs = vectors();
  r = guatiao.wire.Receiver();
  expect_true(r.is_open(), 'open');
  run_conversation(r, vecs);
  r.close();
  r.close();
  expect_true(~r.is_open(), 'closed');
end

function frees()
  vecs = vectors();
  c = guatiao.Allocator.counting();
  r = guatiao.wire.Receiver('Alloc', c);
  run_conversation(r, vecs);
  frame = guatiao.wire.announce(4, struct('type', 'object'));
  e = r.accept(frame);
  expect_true(c.records(e.value), 'the schema is built in the receiver''s allocator');
  e.value.close();
  expect_true(c.live() > 0, 'the receiver keeps a schema');
  r.close();
  expect_equal(c.live(), 0, 'all back');
  v = guatiao.Value.from_wire(uint8([4 1 120]), 'Alloc', c);
  v.close();
  expect_error(@() guatiao.Value.from_wire(uint8([1 2]), 'Alloc', c), 'guatiao:BadValue:malformed');
  expect_equal(c.live(), 0, 'a refused decode frees what it built');
end

function closed()
  r = guatiao.wire.Receiver();
  r.close();
  expect_error(@() r.accept(guatiao.wire.close(1)), 'guatiao:Gone:closed');
  expect_error(@() guatiao.wire.close(-1), 'guatiao:BadValue');
  expect_error(@() guatiao.wire.send(1.5, 1), 'guatiao:BadValue');
end
