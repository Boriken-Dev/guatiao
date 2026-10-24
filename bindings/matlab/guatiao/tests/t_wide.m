% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_wide()
% 64-bit integers, which a double holds exactly only to 2^53, cross a slot
% as int64 and uint64: through a provider written in Rust, C or M, and from
% Rust into the M one. What does not fit its slot is refused naming the
% argument.
  cases = {
    'a slot of a provider in Rust or C carries 64-bit integers exactly', @rust_and_c
    'a provider written in M takes and answers 64-bit integers exactly', @written_in_m
    'what does not fit, or is no whole number, is refused', @refusals
    'an answer that does not fit fails the call that asked', @overshoot
    'Rust calls a wide provider in M, Rust and C, exactly', @rust_calls
  };
end

function v = signed_limits()
  v = {bitshift(int64(1), 53) + 1, intmin('int64'), intmax('int64')};
end

function v = unsigned_limits()
  v = {bitshift(uint64(1), 53) + 1, bitshift(uint64(1), 63), intmax('uint64')};
end

function reg = wide_host(with_m)
  gt_env('examples');
  reg = guatiao.Registry('m-wide', '0.1');
  reg.watch_dir(gt_env('dir'), 'kind=greeter');
  c_dir = gt_env('c_greeter');
  if ~isempty(c_dir)
    reg.watch_dir(c_dir, 'kind=greeter');
  end
  reg.refresh();
  if with_m
    reg.register_library(guatiao.Library('m_wide', '0.1.0', {'gtt.Echo', 'gtt.Overshoot'}));
  end
end

function crosses_exactly(w, which)
  s = signed_limits();
  for i = 1:numel(s)
    expect_equal(w.echo_i64(s{i}), s{i}, [which ' echo_i64']);
  end
  u = unsigned_limits();
  for i = 1:numel(u)
    expect_equal(w.echo_u64(u{i}), u{i}, [which ' echo_u64']);
  end
  % A double that is whole converts, and what fits a double answers one.
  expect_equal(w.echo_i64(5), 5, [which ' a double']);
  expect_equal(w.echo_i64(int32(-5)), -5, [which ' another integer class']);
  expect_equal(w.echo_i64(-2^63), intmin('int64'), [which ' -2^63 as a double']);
  expect_equal(w.echo_u64(2^63), bitshift(uint64(1), 63), [which ' 2^63 as a double']);
  expect_equal(w.echo_u64(true), 1, [which ' a logical']);
end

function rust_and_c()
  reg = wide_host(false);
  crosses_exactly(reg.bind('derived_greeter_hello', 'greeterkind.Wide'), 'rust');
  if isempty(gt_env('c_greeter'))
    skip_case('no C compiler: the greeter written in C is not built');
  end
  crosses_exactly(reg.bind('c_greeter', 'greeterkind.Wide'), 'c');
end

function written_in_m()
  reg = wide_host(true);
  crosses_exactly(reg.bind('m_wide_echo', 'greeterkind.Wide'), 'm');
end

function refused_by(w, which)
  too_big = {bitshift(uint64(1), 63), 2^63, uint64(intmax('uint64')), 1e30};
  for i = 1:numel(too_big)
    e = expect_error(@() w.echo_i64(too_big{i}), 'guatiao:BadValue', [which ' echo_i64 too big']);
    expect_true(~isempty(strfind(e.message, 'out of range for int64')), e.message);
  end
  too_small = {-1, int8(-1), intmin('int64'), -2^63, -1e30};
  for i = 1:numel(too_small)
    e = expect_error(@() w.echo_u64(too_small{i}), 'guatiao:BadValue', [which ' echo_u64 negative']);
    expect_true(~isempty(strfind(e.message, 'out of range for uint64')), e.message);
  end
  e = expect_error(@() w.echo_u64(2^64), 'guatiao:BadValue', [which ' echo_u64 2^64']);
  expect_true(~isempty(strfind(e.message, 'out of range for uint64')), e.message);
  for bad = {1.5, NaN, Inf, 'a', {1}, [1 2], []}
    expect_error(@() w.echo_i64(bad{1}), 'guatiao:BadValue', [which ' echo_i64 no whole number']);
    expect_error(@() w.echo_u64(bad{1}), 'guatiao:BadValue', [which ' echo_u64 no whole number']);
  end
end

function refusals()
  reg = wide_host(true);
  refused_by(reg.bind('derived_greeter_hello', 'greeterkind.Wide'), 'rust');
  refused_by(reg.bind('m_wide_echo', 'greeterkind.Wide'), 'm');
  if ~isempty(gt_env('c_greeter'))
    refused_by(reg.bind('c_greeter', 'greeterkind.Wide'), 'c');
  end
  w = reg.bind('m_wide_echo', 'greeterkind.Wide');
  expect_error(@() w.echo_i64(7), 'guatiao:BadValue', 'the provider''s own words');
  expect_error(@() w.echo_u64(7), 'guatiao:BadValue', 'the provider''s own words');
end

function overshoot()
  reg = wide_host(true);
  o = reg.bind('m_wide_overshoot', 'greeterkind.Wide');
  expect_error(@() o.echo_i64(5), 'guatiao:BadValue', 'an answer past INT64_MAX');
  expect_error(@() o.echo_u64(5), 'guatiao:BadValue', 'a negative answer');
end

function rust_calls()
  reg = wide_host(true);
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  keys = {'m', 'm_wide_echo'; 'rust', 'derived_greeter_hello'};
  if ~isempty(gt_env('c_greeter'))
    keys(end + 1, :) = {'c', 'c_greeter'};
  end
  for r = 1:size(keys, 1)
    out = guatiao.Value();
    st = guatiao.serving(@() gtfixture('wide_caller', gt_env('workspace', 'greeter_caller'), reg.host(), ...
                                       keys{r, 2}, guatiao_mex('node_pointer', out.owner_id(), {})));
    expect_equal(st, 0, [keys{r, 1} ' greeter_caller_wide']);
    report = out.data();
    out.close();
    % A tree holds an integer past INT64_MAX as uint64 and any other as int64.
    s = signed_limits();
    u = unsigned_limits();
    for i = 1:3
      expect_true(isinteger(report.i64{i}) && isinteger(report.u64{i}), [keys{r, 1} ' integers, not doubles']);
      expect_equal(report.i64{i}, cast(s{i}, class(report.i64{i})), [keys{r, 1} ' i64']);
      expect_equal(report.u64{i}, cast(u{i}, class(report.u64{i})), [keys{r, 1} ' u64']);
    end
  end
end
