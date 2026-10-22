% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_typed()
% A configuration is a guatiao.Config of fields; its schema is read from
% them, and a value reads back as a struct of them.
  cases = {
    'a configuration''s schema', @schema
    'a value reads back as a configuration', @reading
  };
end

function c = agent()
  c = guatiao.Config('host', guatiao.field('string', 'Doc', 'Where the agent is.', 'minLength', 1), ...
                     'port', guatiao.field('integer', 'Default', 5900), ...
                     'labels', guatiao.field('list', 'Default', {}));
end

function c = plain()
  c = guatiao.Config('name', 'string', 'ratio', guatiao.field('number', 'Default', 0.5), 'on', 'boolean', ...
                     'note', guatiao.field('string', 'Nullable', true), 'agent', agent(), 'since', 'date', ...
                     'every', guatiao.field('span', 'Default', 'P1D'), 'wait', guatiao.field('duration', 'Nullable', true));
end

function schema()
  s = agent().schema();
  expect_equal(fieldnames(s.properties)', {'host', 'port', 'labels'});
  expect_equal(s.properties.host, struct('type', 'string', 'description', 'Where the agent is.', 'minLength', 1));
  expect_equal(s.properties.port.default, 5900);
  expect_equal(s.required, {'host'});
  expect_equal(s.additionalProperties, false);
  p = plain().schema();
  expect_equal(guatiao.serde.emit(p.properties.note), '{"anyOf":[{"type":"string"},{"type":"null"}]}');
  expect_equal(p.properties.ratio.default, 0.5);
  expect_equal(p.properties.since, struct('type', 'string', 'format', 'date'));
  expect_equal(p.properties.agent.type, 'object');
  expect_equal(p.required, {'name', 'on', 'agent', 'since'});
  expect_true(~isempty(strfind(guatiao.serde.emit(p.properties.wait), '18446744073709551615')));
  expect_equal(guatiao.validate(s, struct('host', 'h')), []);
  refusal = guatiao.validate(s, struct('port', 1));
  expect_equal(refusal.key, 'host');
  expect_true(~isempty(guatiao.validate(s, struct('host', 'h', 'nope', 1))));
  expect_true(~isempty(guatiao.validate(s, struct('host', ''))));
  expect_error(@() guatiao.Config('not a name', 'string'), 'guatiao:BadValue');
  expect_error(@() guatiao.field('nonesuch'), 'guatiao:BadValue');
end

function reading()
  a = agent().read(struct('host', 'h', 'ignored', true));
  expect_equal(a, struct('host', 'h', 'port', 5900, 'labels', {{}}));
  b = agent().read(struct('host', 'h', 'port', 22, 'labels', {{1}}));
  expect_equal(b.port, 22);
  expect_equal(b.labels, {1});
  e = expect_error(@() agent().read(struct('host', 'h', 'port', '22')), 'guatiao:BadValue');
  expect_equal(e.message, 'at port: want an integer, got string');
  expect_error(@() agent().read(struct('host', 'h', 'port', 1.5)), 'guatiao:BadValue');
  e = expect_error(@() agent().read(struct('port', 1)), 'guatiao:BadValue');
  expect_equal(e.message, 'at host: required');
  p = plain().read(struct('name', 'n', 'on', true, 'note', [], 'agent', struct('host', 'x'), 'since', '2026-10-21'));
  expect_equal(p.note, []);
  expect_equal(p.ratio, 0.5);
  expect_equal(p.agent.host, 'x');
  expect_true(p.since == guatiao.Date(2026, 10, 21));
  expect_equal(p.every, 'P1D');
  e = expect_error(@() plain().read(struct('name', 'n', 'on', true, 'agent', struct('host', 1), ...
                                           'since', '2026-10-21')), 'guatiao:BadValue');
  expect_equal(e.message, 'at agent.host: want text, got number');
  v = guatiao.Value(struct('host', 'h', 'wait', {{3, 0}}));
  w = guatiao.Config('host', 'string', 'wait', 'duration').read(v);
  expect_equal(w.wait, guatiao.Duration(3));
  round = agent().read(guatiao.Value(a));
  expect_equal(round.host, a.host);
end
