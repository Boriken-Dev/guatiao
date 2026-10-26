% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_serde()
  cases = {
    'JSON keeps key order and exact numbers', @json
    'TOML and YAML round-trip', @toml_yaml
    'a malformed document names its format and position', @malformed
    'MATLAB data emits too, and bytes as asked', @plain
    'a document reads straight into the data parse then data() answers', @data_reads
    'data reads every shared vector as the value it decodes to', @data_vectors
    'MATLAB data writes as wire bytes what it builds as a tree, refusals included', @data_writes
    'the writer judges text as UTF-8 as the library does, after any run of ASCII', @text_judged
  };
end

function same = deep_equal(a, b)
% Equal data, a containers.Map compared by its keys and values.
  same = strcmp(class(a), class(b));
  if ~same
    return
  end
  if isa(a, 'containers.Map')
    same = isequal(keys(a), keys(b));
    k = keys(a);
    for i = 1:numel(k)
      same = same && deep_equal(a(k{i}), b(k{i}));
    end
  elseif iscell(a)
    same = isequal(size(a), size(b));
    for i = 1:numel(a)
      same = same && deep_equal(a{i}, b{i});
    end
  elseif isstruct(a)
    same = isequal(size(a), size(b)) && isequal(fieldnames(a), fieldnames(b));
    f = fieldnames(a);
    for i = 1:numel(a)
      for j = 1:numel(f)
        same = same && deep_equal(a(i).(f{j}), b(i).(f{j}));
      end
    end
  elseif isa(a, 'guatiao.Number')
    same = strcmp(a.Text, b.Text);
  else
    same = isequal(a, b) && isequal(size(a), size(b));
  end
end

function expect_same(got, want, label)
  same = deep_equal(got, want);
  if ~same
    label = sprintf('%s: got %s, want %s', label, shown(got), shown(want));
  end
  expect_true(same, label);
end

function t = shown(x)
  try
    t = guatiao.serde.emit(x, 'Pretty', true);
  catch
    t = class(x);
  end
end

function data_reads()
  docs = {
    ['{"z":1,"a":[1,2.50,"x"],"big":123456789012345678901234567890.5,"n":null,"t":true,"e":{},"l":[],' ...
     '"s":"ü€😀","nested":{"a b":1,"c":[[1,2],[3,4]]},"neg":-0,' ...
     '"i":9007199254740993,"u":18446744073709551615,"m":-9223372036854775808,"f":1e400}']
    '[1,{"x":[]},null,[true,false]]'
    '"just text"'
    '3'
    'null'
    '{"b":"data:;base64,AQID"}'
  };
  options = {{}, {'Numbers', 'text'}, {'Numbers', 'double'}, {'Maps', 'map'}, {'ReadDataUris', true}};
  for i = 1:numel(docs)
    for k = 1:numel(options)
      parsing = {};
      read = options{k};
      if ~isempty(read) && strcmp(read{1}, 'ReadDataUris')
        parsing = read;
        read = {};
      end
      v = guatiao.serde.parse(docs{i}, parsing{:});
      want = v.data(read{:});
      v.close();
      label = sprintf('document %d, options %d', i, k);
      expect_same(guatiao.serde.data(docs{i}, options{k}{:}), want, label);
      [got, problem] = guatiao.serde.data(docs{i}, 'json', options{k}{:});
      expect_same(got, want, [label ', two outputs']);
      expect_equal(problem, [], [label ', no problem']);
    end
  end
  if guatiao.has_export('guatiao_toml_to_wire', 'guatiao_serde')
    expect_equal(guatiao.serde.data(sprintf('name = "ana"\nn = 3\n'), 'toml'), struct('name', 'ana', 'n', 3));
  else
    skip_case('guatiao_serde built without toml');
  end
  if guatiao.has_export('guatiao_yaml_to_wire', 'guatiao_serde')
    expect_equal(guatiao.serde.data(sprintf('a: [1, 2]\n'), 'Format', 'yaml'), struct('a', {{1, 2}}));
  else
    skip_case('guatiao_serde built without yaml');
  end
  e = expect_error(@() guatiao.serde.data(sprintf('{\n  "a": }')), 'guatiao:BadValue:parse');
  expect_true(~isempty(strfind(e.message, 'JSON does not parse at line 2, column')), e.message);
  [x, problem] = guatiao.serde.data(sprintf('{\n  "a": }'));
  expect_equal(x, []);
  expect_equal(problem.format, 'json');
  expect_equal(problem.line, 2);
  expect_error(@() guatiao.serde.data('{}', 'xml'), 'guatiao:BadValue');
  expect_error(@() guatiao.serde.data('{}', 'Numbers', 'nope'), 'guatiao:BadValue');
end

function vecs = vectors()
  vecs = guatiao.serde.parse(read_text_file(fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'vectors', 'wire.json')));
end

function b = unhex(t)
  if isempty(t)
    b = zeros(1, 0, 'uint8');
  else
    b = uint8(hex2dec(reshape(t, 2, [])'))';
  end
end

function data_vectors()
  values = vectors().at('values');
  expect_true(values.length() > 0, 'there are vectors');
  for i = 1:values.length()
    c = values.at(i);
    name = c.at('name').data();
    bytes = unhex(c.at('hex').data());
    v = guatiao.Value.from_wire(bytes);
    for numbers = 0:2
      for maps = 0:1
        names = {'auto', 'text', 'double'};
        want = v.data('Numbers', names{numbers + 1}, 'Maps', ifelse(maps, 'map', 'auto'));
        expect_same(guatiao_mex('wire_read', bytes, numbers, maps), want, sprintf('%s, read %d %d', name, numbers, maps));
      end
    end
    v.close();
  end
  % A reader of what a library wrote still stops at the end of the bytes.
  whole = guatiao_mex('wire_of', struct('a', {{1, 'two', uint8(3)}}));
  for n = 0:numel(whole) - 1
    expect_error(@() guatiao_mex('wire_read', whole(1:n), 0, 0), 'guatiao:BadValue', sprintf('cut at %d', n));
  end
  expect_error(@() guatiao_mex('wire_read', [whole uint8(1)], 0, 0), 'guatiao:BadValue', 'bytes after the value');
  expect_error(@() guatiao_mex('wire_read', uint8([6 255 255 255 15]), 0, 0), 'guatiao:BadValue', 'a count past the bytes');
end

function y = ifelse(c, a, b)
  if c
    y = a;
  else
    y = b;
  end
end

function data_writes()
  kept = guatiao.Value(struct('k', {{1, 2}}));
  inner = kept.at('k');
  m = containers.Map({'a b', 'c'}, {1, 'x'});
  data = struct('s', native2unicode(uint8([99 97 102 195 169 240 159 166 128]), 'UTF-8'), 'n', guatiao.Number('1.10'), 'i', int8(-3), 'u', uint64(18446744073709551615), ...
                'f', 0.1, 'g', single(0.1), 'sub', 5e-324, 'b', true, 'bs', [true false], 'none', [], 'bytes', uint8(1:2), ...
                'v', [1.5 2], 'mat', [1 2; 3 4], 'cells', {{1, 'a'; [], {}}}, 'absent', guatiao.Absent(), ...
                'ref', kept, 'view', inner, 'map', m, 'people', struct('name', {'ana', 'bo'}), 'e', struct(), ...
                'deep', {{{{1}}}}, 'text', '');
  built = guatiao.Value(data);
  expect_equal(guatiao_mex('wire_of', data), built.to_wire(), 'the same bytes');
  expect_equal(guatiao.serde.emit(data), guatiao.serde.emit(built), 'the same JSON');
  expect_equal(guatiao.serde.emit(data, 'Pretty', true), guatiao.serde.emit(built, 'Pretty', true), 'the same pretty JSON');
  if guatiao.has_export('guatiao_yaml_from_wire', 'guatiao_serde')
    expect_equal(guatiao.serde.emit(data, 'yaml'), guatiao.serde.emit(built, 'yaml'), 'the same YAML');
  end
  for how = {'data_uri', 'base64', 'array'}
    expect_equal(guatiao.serde.emit(data, 'Bytes', how{1}), guatiao.serde.emit(built, 'Bytes', how{1}), how{1});
  end
  built.close();
  expect_equal(guatiao_mex('wire_of', guatiao.Absent()), uint8(0), 'absent alone');
  expect_equal(guatiao_mex('wire_of', {guatiao.Absent()}), uint8([6 1 0]), 'absent in a list');
  expect_equal(guatiao_mex('wire_of', struct('a', guatiao.Absent())), uint8([7 0]), 'absent in a map is no entry');
  many = struct();
  for i = 1:200
    many.(sprintf('k%d', i)) = guatiao.Absent();
  end
  many.last = 1;
  b = guatiao.Value(many);
  expect_equal(guatiao_mex('wire_of', many), b.to_wire(), 'a count rewritten shorter');
  b.close();
  deep = {1};
  for i = 1:200
    deep = {deep};
  end
  bad = {NaN, Inf, @sin, ones(2, 2, 2), sparse(1), 1 + 2i, deep, ['ab'; 'cd'], struct('a', {1, 2; 3, 4}), ...
         containers.Map({1}, {2}), struct('k', {{single(NaN)}})};
  for i = 1:numel(bad)
    tree = message_of(@() guatiao.Value(bad{i}));
    tape = message_of(@() guatiao_mex('wire_of', bad{i}));
    expect_true(~isempty(tree), sprintf('refusal %d is refused', i));
    expect_equal(tape, tree, sprintf('refusal %d', i));
  end
  kept.close();
end

% Octave's chars are the text's bytes, so each shape reaches both writers
% as it is; MATLAB's chars are UTF-16 and always reach them as UTF-8.
function text_judged()
  shapes = {[195 169], [226 130 172], [240 144 128 128], [244 143 191 191], [192 128], [193 191], ...
            [224 159 191], [237 160 128], [240 143 191 191], [244 144 128 128], [245 128 128 128], 128, 195, ...
            [226 130], [240 159 152]};
  for run = [0 1 7 8 9 16 23]
    for s = 1:numel(shapes)
      x = {char([repmat(120, 1, run), shapes{s}, repmat(121, 1, mod(run, 3))])};
      tree = wire_or_message(@() value_wire(x));
      tape = wire_or_message(@() guatiao_mex('wire_of', x));
      expect_equal(tape, tree, sprintf('%d x then %s', run, sprintf('%02x ', shapes{s})));
    end
  end
end

function w = value_wire(x)
  v = guatiao.Value(x);
  w = v.to_wire();
  v.close();
end

function r = wire_or_message(f)
  try
    r = f();
  catch e
    r = [e.identifier ' ' e.message];
  end
end

function m = message_of(f)
  m = '';
  try
    f();
  catch e
    m = [e.identifier ' ' e.message];
  end
end

function json()
  text = '{"z":1,"a":[1,2.50,"x"],"big":123456789012345678901234567890.5}';
  v = guatiao.serde.parse(text);
  expect_equal(v.keys(), {'z', 'a', 'big'});
  expect_equal(guatiao.serde.emit(v), text);
  expect_true(numel(guatiao.serde.emit(v, 'Pretty', true)) > numel(text));
  expect_equal(guatiao.serde.emit(v.at('a'), 'json'), '[1,2.50,"x"]');
  v.close();
end

function toml_yaml()
  if guatiao.has_export('guatiao_toml_emit', 'guatiao_serde')
    v = guatiao.serde.parse(sprintf('name = "ana"\nn = 3\n'), 'toml');
    expect_equal(v.data(), struct('name', 'ana', 'n', 3));
    expect_true(~isempty(strfind(guatiao.serde.emit(v, 'toml'), 'name = "ana"')));
    expect_error(@() guatiao.serde.emit({1}, 'toml'), 'guatiao:');
    v.close();
  else
    skip_case('guatiao_serde built without toml');
  end
  if guatiao.has_export('guatiao_yaml_emit', 'guatiao_serde')
    v = guatiao.serde.parse(sprintf('a: [1, 2]\n'), 'Format', 'yaml');
    expect_equal(v.data(), struct('a', {{1, 2}}));
    back = guatiao.serde.parse(guatiao.serde.emit(v, 'yaml'), 'yaml');
    expect_equal(back.data(), v.data());
    v.close();
  else
    skip_case('guatiao_serde built without yaml');
  end
end

function malformed()
  e = expect_error(@() guatiao.serde.parse(sprintf('{\n  "a": }')), 'guatiao:BadValue:parse');
  expect_true(~isempty(strfind(e.message, 'JSON does not parse at line 2, column')), e.message);
  [v, problem] = guatiao.serde.parse(sprintf('{\n  "a": }'));
  expect_equal(v, []);
  expect_equal(problem.format, 'json');
  expect_equal(problem.line, 2);
  expect_true(problem.column > 0);
  expect_true(~isempty(problem.reason));
  [v, problem] = guatiao.serde.parse('{"a": 1}');
  expect_equal(problem, []);
  expect_equal(v.at('a').data(), 1);
  expect_error(@() guatiao.serde.parse('{}', 'xml'), 'guatiao:BadValue');
end

function plain()
  expect_equal(guatiao.serde.emit(struct('a', {{1, []}})), '{"a":[1,null]}');
  expect_equal(guatiao.serde.emit(struct('b', uint8(1:3))), '{"b":"data:;base64,AQID"}');
  expect_equal(guatiao.serde.emit(struct('b', uint8(1:3)), 'Bytes', 'array'), '{"b":[1,2,3]}');
  expect_equal(guatiao.serde.emit(struct('b', uint8(1:3)), 'Bytes', 'base64'), '{"b":"AQID"}');
  expect_error(@() guatiao.serde.emit(uint8(1), 'Bytes', 'refuse'), 'guatiao:');
  back = guatiao.serde.parse('{"b":"data:;base64,AQID"}', 'ReadDataUris', true);
  expect_equal(back.at('b').data(), uint8(1:3));
  back.close();
  expect_error(@() guatiao.serde.emit(1, 'Nonesuch', 1), 'guatiao:BadValue');
end
