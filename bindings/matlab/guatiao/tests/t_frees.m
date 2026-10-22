% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_frees()
% Everything built is freed on every path, counted by an allocator that
% knows how many blocks it handed out and has not had back.
  cases = {
    'a clean build and close leaves nothing', @clean
    'a conversion that fails part way frees what it built', @failed_builds
    'a failed write leaves the tree as it was', @failed_writes
    'answers are built in the allocator named, and freed', @answers
    'a value nobody holds is freed', @unreferenced
  };
end

function clean()
  c = guatiao.Allocator.counting();
  v = guatiao.Value(struct('a', {{1, 'two', uint8(3)}}, 'm', struct('x', [])), 'Alloc', c);
  expect_true(c.live() > 0, 'blocks out');
  expect_true(c.records(v), 'the root records it');
  v.set('b', 'more');
  v.at('a').push(4);
  v.at('a').insert(1, 'zero');
  v.at('a').set(2, 'one');
  expect_true(c.records(v.at('b')), 'what is added is built in the recorded allocator');
  v.close();
  expect_equal(c.live(), 0, 'all back');
end

function failed_builds()
  c = guatiao.Allocator.counting();
  bad = {struct('a', {{1, 2, @sin}}), {1, struct('x', NaN)}, struct('d', {{containers.Map({1}, {2})}}), ...
         {1, 2, zeros(2, 2, 2)}, {'ok', 1 + 1i}};
  for i = 1:numel(bad)
    expect_error(@() guatiao.Value(bad{i}, 'Alloc', c), 'guatiao:', sprintf('case %d', i));
    expect_equal(c.live(), 0, sprintf('case %d frees', i));
  end
end

function failed_writes()
  c = guatiao.Allocator.counting();
  v = guatiao.Value(struct('a', {{1}}), 'Alloc', c);
  before = c.live();
  expect_error(@() v.set('b', {1, @sin}), 'guatiao:BadValue');
  expect_error(@() v.at('a').push(struct('x', Inf)), 'guatiao:BadValue');
  expect_error(@() v.at('a').set(1, {@sin}), 'guatiao:BadValue');
  expect_error(@() v.at('a').insert(1, @sin), 'guatiao:BadValue');
  expect_equal(c.live(), before, 'nothing kept');
  expect_equal(v.data(), struct('a', {{1}}), 'as it was');
  v.close();
  expect_equal(c.live(), 0);
end

function answers()
  c = guatiao.Allocator.counting();
  v = guatiao.serde.parse('{"a": [1, 2], "b": "x"}', 'Alloc', c);
  expect_true(c.live() > 0);
  v.close();
  expect_error(@() guatiao.serde.parse('{', 'Alloc', c), 'guatiao:BadValue:parse');
  expect_equal(c.live(), 0, 'a refused parse frees');
  r = guatiao.redact(struct('type', 'object'), struct('a', 1), [], 'Alloc', c);
  expect_true(c.live() > 0);
  r.close();
  expect_equal(c.live(), 0);
  w = guatiao.Value(struct('a', {{1}}), 'Alloc', c);
  x = w.at('a').copy();
  w.close();
  expect_equal(c.live(), 0, 'a copy without Alloc is the binding''s');
  y = w;  %#ok<NASGU> a closed handle copied
  x.close();
  z = guatiao.Value(struct('a', {{1, 2}}), 'Alloc', c).at('a').copy('Alloc', c);
  expect_true(c.live() > 0);
  z.close();
  expect_equal(c.live(), 0, 'the view''s value went with its last reference');
end

function unreferenced()
  c = guatiao.Allocator.counting();
  make_and_drop(c);
  expect_equal(c.live(), 0, 'delete ran when the last reference went');
  v = guatiao.Value({1, 2, 3}, 'Alloc', c);
  w = v;  %#ok<NASGU>
  clear('v');
  expect_true(c.live() > 0, 'still referenced');
  clear('w');
  expect_equal(c.live(), 0);
end

function make_and_drop(c)
  v = guatiao.Value(struct('a', {{1, 2, 3}}), 'Alloc', c);  %#ok<NASGU>
end
