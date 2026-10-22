% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_serde()
  cases = {
    'JSON keeps key order and exact numbers', @json
    'TOML and YAML round-trip', @toml_yaml
    'a malformed document names its format and position', @malformed
    'MATLAB data emits too, and bytes as asked', @plain
  };
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
