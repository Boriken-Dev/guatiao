% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_time()
% The shared vectors every implementation reads, then the classes.
  cases = {
    'every vector of every form', @every_vector
    'every span applied', @every_span
    'every exact length', @every_length
    'the classes', @classes
    'a span is its own type, backed by the library', @spans
    'dates and lengths as values', @as_values
    'a refusal names the byte, never the text', @refusals
  };
end

function vecs = vectors()
  path = fullfile(repo_root(), 'crates', 'guatiao', 'tests', 'vectors', 'time.json');
  % Read with guatiao's own parser, numbers as exact text: some units are
  % past what a double holds.
  vecs = guatiao.serde.parse(read_text_file(path));
end

function t = text_of(n)
  if isa(n, 'guatiao.Number')
    t = n.Text;
  else
    t = sprintf('%d', n);
  end
end

function every_vector()
  vecs = vectors();
  forms = {'date', 'time', 'datetime', 'timestamp', 'span'};
  for f = 1:numel(forms)
    form = forms{f};
    cases = vecs.at(form).data('Numbers', 'text');
    expect_true(numel(cases) > 0, [form ' has vectors']);
    for k = 1:numel(cases)
      c = cases{k};
      if isfield(c, 'refused_at')
        at = str2double(text_of(c.refused_at));
        e = expect_error(@() guatiao_mex('time_parse', form, c.text, true), 'guatiao:BadValue:unparsed', c.text);
        expect_true(~isempty(strfind(e.message, sprintf('at byte %d,', at))), [form ' ' c.text ' at']);
        if numel(c.text) > 2
          expect_true(isempty(strfind(e.message, c.text)), 'the message never quotes the text');
        end
        [x, problem] = guatiao.time.normalize(form, c.text);
        expect_equal(x, [], [c.text ' soft']);
        expect_equal(problem.at, at, [c.text ' problem.at']);
        expect_equal(problem.form, form);
        expect_true(~isempty(problem.expected));
      else
        got = guatiao_mex('time_parse', form, c.text, true);
        want = cellfun(@text_of, c.fields, 'UniformOutput', false);
        expect_equal(got, want, [form ' ' c.text]);
        expect_equal(guatiao.time.normalize(form, c.text), c.canonical, [form ' ' c.text ' canonical']);
      end
    end
  end
end

function every_span()
  vecs = vectors();
  forms = {'date', 'datetime'};
  for f = 1:numel(forms)
    form = forms{f};
    rows = vecs.at([form '_add_span']).data('Numbers', 'text');
    for k = 1:numel(rows)
      c = rows{k};
      start = guatiao_mex('time_parse', form, c.start);
      s = guatiao_mex('time_parse', 'span', c.span, true);   % exact: units past 2^53
      label = [c.start ' + ' c.span];
      if ~isfield(c, 'result') || isempty(c.result)
        expect_error(@() guatiao_mex([form '_add_span'], start, s), 'guatiao:', label);
      else
        got = guatiao_mex('time_format', form, guatiao_mex([form '_add_span'], start, s));
        expect_equal(got, c.result, label);
      end
    end
  end
  d = guatiao.time.add_span(guatiao.time.parse('date', '2026-01-31'), 'P1M');
  expect_equal(char(d), '2026-02-28');
end

function every_length()
  vecs = vectors();
  rows = vecs.at('duration').data('Numbers', 'text');
  for k = 1:numel(rows)
    c = rows{k};
    value = c.value;
    if ~isfield(c, 'duration') || isempty(c.duration)
      expect_error(@() guatiao_mex('duration_read', value), 'guatiao:', 'refused');
    else
      got = guatiao_mex('duration_read', value);
      expect_equal(got{1}, text_of(c.duration{1}));
      expect_equal(got{2}, str2double(text_of(c.duration{2})));
      if str2double(got{1}) <= 2^53
        expect_equal(guatiao.time.read_length(value), guatiao.Duration(str2double(got{1}), got{2}));
      else
        expect_error(@() guatiao.time.read_length(value), 'guatiao:BadValue');
      end
    end
  end
end

function classes()
  expect_equal(char(guatiao.time.parse('date', '0000-02-29')), '0000-02-29');
  d = guatiao.Date(2026, 10, 21);
  expect_equal(char(d), '2026-10-21');
  expect_equal([d.Year d.Month d.Day], [2026 10 21]);
  expect_true(d == guatiao.Date('2026-10-21'));
  t = guatiao.time.parse('time', '13:45:00.000000001');
  expect_equal(t.Nanosecond, 1);
  expect_equal(char(t), '13:45:00.000000001');
  dt = guatiao.time.parse('datetime', '2026-10-21T13:45:00.0019');
  expect_equal(char(dt), '2026-10-21T13:45:00.0019');
  expect_true(dt.Date == d);
  ts = guatiao.time.parse('timestamp', '2026-10-21T19:15:00.5+05:30');
  expect_equal([ts.Seconds ts.Nanoseconds], [1792590300 5e8]);
  expect_equal(char(ts), '2026-10-21T13:45:00.5Z');
  expect_equal(ts.posix(), 1792590300.5);
  expect_equal(char(guatiao.Timestamp(-1, 999000000)), '1969-12-31T23:59:59.999Z');
  expect_equal(char(guatiao.Timestamp(-0.001)), '1969-12-31T23:59:59.999Z');
  expect_error(@() guatiao.time.parse('date', '2026-10-21Z'), 'guatiao:BadValue:unparsed');
  expect_equal(guatiao.time.normalize('timestamp', '2026-10-21T13:45:00.000000001+00:00'), ...
               '2026-10-21T13:45:00.000000001Z');
  expect_error(@() guatiao.TimeOfDay(24, 0), 'guatiao:BadValue');
  expect_error(@() guatiao.time.parse('week', 'x'), 'guatiao:BadValue');
  expect_error(@() guatiao.Date(2026, 2, 30), 'guatiao:BadValue');
end

function spans()
  s = guatiao.Span('Days', 1, 'Hours', 2);
  expect_equal(char(s), 'P1DT2H');
  expect_equal(char(-s), '-P1DT2H');
  expect_true(guatiao.Span('Days', 1) ~= guatiao.Span('Hours', 24));
  expect_true(guatiao.Span('Days', 1) == guatiao.time.parse('span', 'P1D'));
  expect_error(@() guatiao.Span('Days', 1, 'Hours', -1), 'guatiao:BadValue');
  expect_error(@() guatiao.Span('Nanoseconds', 1e9), 'guatiao:BadValue');
  expect_true(guatiao.Span('1h 30m') == guatiao.Span('Hours', 1, 'Minutes', 30));
  expect_equal(char(guatiao.Date(2026, 1, 31).add(guatiao.Span('Months', 1))), '2026-02-28');
  expect_equal(char(guatiao.Date(2026, 1, 31).add(guatiao.Span('Months', 1, 'Days', 1))), '2026-03-01');
  expect_equal(char(guatiao.time.add_span(guatiao.DateTime('2026-10-21T23:30:00'), 'PT1H')), '2026-10-22T00:30:00');
  expect_error(@() guatiao.Date(2026, 10, 21).add(guatiao.Span('Hours', 1)), 'guatiao:');
  expect_error(@() guatiao.time.parse('span', 'P9223372036854775807Y'), 'guatiao:BadValue');
end

function as_values()
  v = guatiao.Value(struct('d', guatiao.Date(2026, 10, 21), 't', guatiao.TimeOfDay(13, 45), ...
    'dt', guatiao.DateTime(guatiao.Date(2026, 10, 21), guatiao.TimeOfDay(13, 45)), 'ts', guatiao.Timestamp(0), ...
    's', guatiao.Span('Weeks', 1), 'l', guatiao.Duration(90, 5e8)));
  expect_equal(v.data(), struct('d', '2026-10-21', 't', '13:45:00', 'dt', '2026-10-21T13:45:00', ...
    'ts', '1970-01-01T00:00:00Z', 's', 'P1W', 'l', {{90, 500000000}}));
  expect_equal(guatiao.time.read_length(v.at('l')), guatiao.Duration(90, 5e8));
  expect_equal(guatiao.time.read_length(1.5), guatiao.Duration(1, 5e8));
  expect_equal(guatiao.Duration(1.5).seconds(), 1.5);
  expect_error(@() guatiao.Duration(-1), 'guatiao:BadValue');
end

function refusals()
  [x, problem] = guatiao.time.parse('date', '2026-13-01');
  expect_equal(x, []);
  expect_equal(problem.at, 5);
  e = expect_error(@() guatiao.time.parse('date', 'abcdefgh'), 'guatiao:BadValue:unparsed');
  expect_true(isempty(strfind(e.message, 'abcdefgh')), 'not quoted');
end
