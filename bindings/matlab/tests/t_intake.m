% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_intake()
  cases = {
    'checking a form and laying it out', @check_layout
    'a path names one place inside a value', @paths
    'a form made out of a schema', @forms
    'the flat projection', @flat
    'redaction', @redaction
    'validation against a schema', @validation
  };
end

function s = name_schema()
  s = ['{"type": "object", "properties": {"name": {"type": "string", "title": "Name"}},' ...
       ' "required": ["name"], "additionalProperties": false}'];
end

function s = flat_schema()
  s = ['{"type": "object", "properties": {' ...
       '"auth": {"type": "object", "x-variant-tag": "auth", "oneOf": [' ...
       '{"type": "object", "properties": {"auth": {"const": "sso"}}, "required": ["auth"]},' ...
       '{"type": "object", "properties": {"auth": {"const": "userpass"}, "username": {"type": "string"},' ...
       '"password": {"type": "string", "x-sensitive": true}}, "required": ["auth"]}]},' ...
       '"agent": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},' ...
       '"env": {"type": "object", "additionalProperties": {"type": "string"}},' ...
       '"host": {"type": "string", "title": "Host"}}}'];
end

function check_layout()
  schema = guatiao.serde.parse(name_schema());
  expect_equal(guatiao.intake.check(schema, struct()), []);
  err = guatiao.intake.check(schema, struct('sections', {{}}, 'fields', struct('nope', struct())));
  expect_equal(err.kind, 'unknown_field');
  sections = guatiao.intake.layout(schema, struct());
  expect_equal(sections{1}.fields, {'name'});
  expect_true(guatiao.intake.is_visible(schema, struct(), 'name', struct('name', 'ana')));
  expect_error(@() guatiao.intake.is_visible(schema, struct(), 'nope', struct()), 'guatiao:NotFound');
  schema.close();
end

function paths()
  env = containers.Map({'PATH', 'a.b', '1'}, {'/bin', 'dotted', 'keyed'});
  v = guatiao.Value(struct('agent', {{struct('name', 'one'), struct('name', 'two')}}, 'env', env));
  % Octave cannot index the result of a package function in place: name it.
  want = {'agent[0].name', 'one'; 'agent[1].name', 'two'; 'env[PATH]', '/bin'; 'env[a.b]', 'dotted'; ...
          'env["1"]', 'keyed'};
  for k = 1:size(want, 1)
    n = guatiao.intake.at(v, want{k, 1});
    expect_equal(n.data(), want{k, 2}, want{k, 1});
  end
  expect_equal(guatiao.intake.at(v, 'agent[9].name'), []);
  expect_equal(guatiao.intake.at(v, 'nonesuch'), []);
  expect_equal(guatiao.intake.at(v, 'agent['), []);
  view = guatiao.intake.at(v, 'agent[0]');
  expect_equal(class(view), 'guatiao.Node');
  expect_equal(view.path(), 'agent[0]');
  agents = v.at('agent');
  inner = guatiao.intake.at(agents, '[1]');
  if ~isempty(inner)
    expect_equal(inner.path(), 'agent[1]', 'a view''s own path, then the lookup''s');
  end
  copy = guatiao.intake.at(struct('a', struct('b', 1)), 'a.b');
  expect_equal(class(copy), 'guatiao.Value');
  expect_equal(copy.data(), 1);
  v.close();
  expect_error(@() view.data(), 'guatiao:Gone:closed');
end

function forms()
  props = struct('host', struct('type', 'string', 'x_section', 'net'));
  schema = guatiao.Value(struct('type', 'object', 'additionalProperties', false));
  p = guatiao.Value(struct());
  p.set('host', containers.Map({'type', 'x-section'}, {'string', 'net'}));
  p.set('tls', struct('type', 'object', 'additionalProperties', false, 'properties', ...
        struct('verify', containers.Map({'type', 'x-section'}, {'boolean', 'trust'}))));
  p.set('label', struct('type', 'string'));
  schema.set('properties', p);
  clear('props');
  made = guatiao.intake.for_schema(schema);
  doc = made.data();
  expect_equal(doc.sections{1}.id, 'net');
  expect_equal(doc.fields.tls.form.sections, {struct('id', 'trust')});
  expect_equal(guatiao.intake.check(schema, made), []);
  made.close();
  implied = guatiao.intake.form_for(struct(), schema, 'tls');
  expect_equal(class(implied), 'guatiao.Value');
  implied.close();
  expect_equal(guatiao.intake.form_for(struct(), schema, 'label'), []);
  expect_equal(guatiao.intake.form_for(struct(), schema, 'nonesuch'), []);
  schema.close();
end

function flat()
  schema = guatiao.serde.parse(flat_schema());
  host = guatiao.intake.resolve(schema, 'host');
  expect_equal(host.at('title').data(), 'Host');
  password = guatiao.intake.resolve(schema, 'auth.password');
  expect_equal(password.at('x-sensitive').data(), true);
  expect_equal(guatiao.intake.resolve(schema, 'nonesuch'), []);
  expect_equal(guatiao.intake.flat_keys(schema, 'auth'), {'auth', 'auth.username', 'auth.password'});
  expect_error(@() guatiao.intake.flat_keys(schema, 'nonesuch'), 'guatiao:');
  rows = {
    'auth', struct('auth', 'userpass', 'username', 'alice', 'password', 's3cret'), ...
      {'auth', 'userpass'; 'auth.username', 'alice'; 'auth.password', 's3cret'}
    'auth', struct('auth', 'sso'), {'auth', 'sso'}
    'agent', {struct('name', 'one'), struct('name', 'two')}, {'agent[0].name', 'one'; 'agent[1].name', 'two'}
    'env', containers.Map({'PATH', 'a.b'}, {'/bin', 'dotted'}), {'env[PATH]', '/bin'; 'env[a.b]', 'dotted'}
    'host', 'example.org', {'host', 'example.org'}
  };
  for r = 1:size(rows, 1)
    key = rows{r, 1};
    want = containers.Map(rows{r, 3}(:, 1), rows{r, 3}(:, 2));
    got = guatiao.intake.flatten(schema, key, rows{r, 2});
    expect_equal(sort(keys(got)), sort(keys(want)), ['flatten ' key]);
    expect_equal(values(got, sort(keys(got))), values(want, sort(keys(want))), ['flatten ' key ' values']);
    back = guatiao.intake.unflatten(schema, key, want);
    expect_equal(guatiao.serde.emit(back), guatiao.serde.emit(rows{r, 2}), ['unflatten ' key]);
    back.close();
  end
  expect_equal(guatiao.intake.unflatten(schema, 'auth', containers.Map({'auth'}, {'nonesuch'})), []);
  expect_equal(guatiao.intake.unflatten(schema, 'nonesuch', struct()), []);
  expect_equal(guatiao.intake.unflatten(schema, 'host', struct()), []);
  expect_error(@() guatiao.intake.unflatten(schema, 'host', struct('host', 1)), 'guatiao:');
  expect_error(@() guatiao.intake.flatten(schema, 'auth', {1}), 'guatiao:');
  schema.close();
  expect_error(@() host.data(), 'guatiao:Gone:closed');
end

function redaction()
  schema = ['{"type": "object", "properties": {"user": {"type": "string"},' ...
            '"password": {"type": "string", "writeOnly": true, "x-sensitive": true},' ...
            '"tls": {"type": "object", "properties": {"ca": {"type": "string"}, "key": {"$ref": "#/$defs/Key"}}},' ...
            '"recovery": {"type": "array", "items": {"type": "string", "x-sensitive": true}}},' ...
            '"$defs": {"Key": {"type": "string", "x-sensitive": true}}}'];
  s = guatiao.serde.parse(schema);
  login = struct('user', 'ana', 'password', 'hunter2', 'tls', struct('ca', '/ca', 'key', 'k'), ...
                 'recovery', {{'r1', 'r2'}}, 'extra', '?');
  shown = guatiao.redact(s, login);
  expect_equal(shown.data(), struct('user', 'ana', 'password', '[redacted]', ...
    'tls', struct('ca', '/ca', 'key', '[redacted]'), 'recovery', {{'[redacted]', '[redacted]'}}, ...
    'extra', '[redacted]'));
  shown.close();
  doc = guatiao.Value(struct('inner', login));
  r = guatiao.redact(s, doc.at('inner'), '***');
  expect_equal(r.at('password').data(), '***');
  doc.close();
  expect_error(@() guatiao.redact({1}, login), 'guatiao:WrongKind');
  s.close();
end

function validation()
  schema = struct('type', 'object', 'properties', struct('n', struct('type', 'integer', 'minimum', 1)), ...
                  'required', {{'n'}});
  expect_equal(guatiao.validate(schema, struct('n', 2)), []);
  problem = guatiao.validate(schema, struct('n', 0));
  expect_true(isstruct(problem) || isa(problem, 'containers.Map'), 'a refusal');
  problem = guatiao.validate(schema, struct());
  expect_true(~isempty(problem), 'missing');
end
