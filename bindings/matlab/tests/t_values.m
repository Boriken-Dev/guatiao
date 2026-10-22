% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_values()
% Values: building from MATLAB data, reading back, views walked fresh,
% writes.
  cases = {
    'MATLAB data becomes a value and reads back', @round_trip
    'numbers keep their exact text', @numbers
    'absent and null', @absent_null
    'arrays, cells and structs', @shapes
    'maps whose keys are not field names', @maps
    'views are walked again on every access', @views
    'writes: set, push, insert, remove, clear, merge', @writes
    'a copy is its own value', @copies
    'strings are UTF-8 both ways', @strings
    'objects are written through their properties or guatiao_value', @objects
    'nesting stops past 128 containers', @depth
  };
end

function round_trip()
  v = guatiao.Value(struct('host', '10.0.0.1', 'port', 5900, 'ratio', 0.25, 'on', true, 'none', [], ...
                           'tags', {{'a', 'b'}}, 'raw', uint8([0 255]), 'nested', {{struct('k', 1)}}));
  expect_equal(v.tag(), 'map');
  expect_equal(v.length(), 8);
  expect_equal(v.keys(), {'host', 'port', 'ratio', 'on', 'none', 'tags', 'raw', 'nested'});
  d = v.data();
  expect_equal(d.host, '10.0.0.1');
  expect_equal(d.port, 5900);
  expect_equal(d.ratio, 0.25);
  expect_equal(d.on, true);
  expect_equal(d.none, []);
  expect_equal(d.tags, {'a', 'b'});
  expect_equal(d.raw, uint8([0 255]));
  expect_equal(d.nested, {struct('k', 1)});
  expect_equal(v.at('none').tag(), 'null');
  expect_true(v.at('none').isnull());
  expect_equal(v.at('nonesuch'), []);
  expect_true(v.has('host') && ~v.has('nonesuch'));
  expect_equal(v.at('tags').at(2).data(), 'b');
  expect_equal(v.at(2).data(), 5900, 'a map''s position names its key');
  v.close();
  expect_true(~v.is_open());
end

function numbers()
  v = guatiao.Value(struct('big', guatiao.Number('18446744073709551615'), 'frac', guatiao.Number('1.10'), ...
                           'int', 3000000000, 'neg', int32(-2147483647), 'exp', guatiao.Number('1e400'), ...
                           'close', guatiao.Number('9007199254740993'), 'held', guatiao.Number('9007199254740992'), ...
                           'negzero', -0, 'i64', int64(-9223372036854775807) - 1, 'u64', intmax('uint64'), ...
                           'huge', guatiao.Number('123456789012345678901234567890')));
  expect_equal(v.at('big').data(), uint64(18446744073709551615));
  expect_equal(v.at('big').data('Numbers', 'text'), guatiao.Number('18446744073709551615'));
  expect_equal(v.at('frac').data(), 1.1);
  expect_equal(v.at('frac').data('Numbers', 'text'), guatiao.Number('1.10'));
  expect_equal(v.at('int').data(), 3e9);
  expect_equal(v.at('neg').data(), -2147483647);
  expect_equal(class(v.at('exp').data()), 'guatiao.Number');
  expect_equal(v.at('close').data(), int64(9007199254740993));
  expect_equal(v.at('held').data(), 9007199254740992);
  expect_equal(v.at('i64').data(), intmin('int64'));
  expect_equal(v.at('u64').data(), intmax('uint64'));
  expect_equal(class(v.at('huge').data()), 'guatiao.Number');
  expect_equal(v.at('huge').data('Numbers', 'double'), 123456789012345678901234567890);
  expect_equal(v.at('negzero').data('Numbers', 'text'), guatiao.Number('-0'));
  expect_equal(guatiao.serde.emit(v), ['{"big":18446744073709551615,"frac":1.10,"int":3000000000,' ...
    '"neg":-2147483647,"exp":1e400,"close":9007199254740993,"held":9007199254740992,"negzero":0,' ...
    '"i64":-9223372036854775808,"u64":18446744073709551615,"huge":123456789012345678901234567890}']);
  expect_equal(guatiao.serde.emit(struct('a', 0.1, 'b', 1/3, 'c', 2^1000, 'd', 2^60, 'e', single(0.1))), ...
    '{"a":0.1,"b":0.3333333333333333,"c":1.0715086071862673e+301,"d":1152921504606846976,"e":0.1}');
  expect_error(@() guatiao.Value(NaN), 'guatiao:BadValue');
  expect_error(@() guatiao.Value(Inf), 'guatiao:BadValue');
  expect_error(@() guatiao.Number('1.2.3'), 'guatiao:BadValue');
  expect_error(@() guatiao.Value(1 + 2i), 'guatiao:BadValue');
  v.close();
end

function absent_null()
  expect_true(guatiao.Value().isabsent());
  expect_equal(class(guatiao.Value().data()), 'guatiao.Absent');
  expect_equal(guatiao.Value([]).tag(), 'null');
  expect_equal(guatiao.serde.emit(struct('a', [], 'b', guatiao.Absent(), 'c', 1)), '{"a":null,"c":1}');
  expect_error(@() guatiao.serde.emit({[], guatiao.Absent()}), 'guatiao:BadValue', 'a list holds no absent');
end

function shapes()
  expect_equal(guatiao.serde.emit([1 2 3]), '[1,2,3]');
  expect_equal(guatiao.serde.emit([1; 2; 3]), '[1,2,3]');
  expect_equal(guatiao.serde.emit([1 2; 3 4]), '[[1,2],[3,4]]');
  expect_equal(guatiao.serde.emit([true false]), '[true,false]');
  expect_equal(guatiao.serde.emit(int8([-1 2])), '[-1,2]');
  expect_equal(guatiao.serde.emit({1, 'a', {}}), '[1,"a",[]]');
  expect_equal(guatiao.serde.emit({1, 2; 3, 4}), '[[1,2],[3,4]]');
  expect_equal(guatiao.serde.emit(struct()), '{}');
  expect_equal(guatiao.serde.emit(struct('a', {1, 2})), '[{"a":1},{"a":2}]');
  expect_equal(guatiao.serde.emit(''), '""');
  expect_equal(guatiao.serde.emit(uint8([])), '"data:;base64,"');
  expect_equal(guatiao.Value({}).tag(), 'list');
  expect_equal(guatiao.Value({}).data(), cell(1, 0));
  expect_error(@() guatiao.Value(['ab'; 'cd']), 'guatiao:BadValue');
  expect_error(@() guatiao.Value(zeros(2, 2, 2)), 'guatiao:BadValue');
  expect_error(@() guatiao.Value(sparse(1)), 'guatiao:BadValue');
  expect_error(@() guatiao.Value(@sin), 'guatiao:BadValue');
end

function maps()
  m = containers.Map({'a b', 'c'}, {1, 'x'});
  v = guatiao.Value(m);
  expect_equal(v.keys(), {'a b', 'c'});
  back = v.data();
  expect_equal(class(back), 'containers.Map');
  expect_equal(back('a b'), 1);
  expect_equal(back('c'), 'x');
  s = guatiao.Value(struct('x', 1)).data('Maps', 'map');
  expect_equal(class(s), 'containers.Map');
  expect_equal(s('x'), 1);
  expect_error(@() guatiao.Value(containers.Map({1}, {2})), 'guatiao:BadValue');
  long = repmat('k', 1, 64);
  w = guatiao.serde.parse(['{"' long '": 1, "_x": 2}']);
  expect_equal(class(w.data()), 'containers.Map', 'a key too long or not an identifier');
end

function views()
  v = guatiao.Value(struct('a', {{1, 2, 3}}, 'm', struct('x', 'y')));
  second = v.at('a').at(2);
  expect_equal(second.path(), 'a[1]');
  expect_equal(second.data(), 2);
  v.at('a').remove(1);
  expect_equal(second.data(), 3);
  v.at('a').remove(1);
  expect_error(@() second.data(), 'guatiao:NotFound');
  expect_equal(v.at('m').at('x').path(), 'm.x');
  expect_error(@() v.at('m').at(5), 'guatiao:NotFound');
  expect_error(@() v.at('a').at('x'), 'guatiao:WrongKind');
  view = v.at('m');
  v.close();
  expect_error(@() view.data(), 'guatiao:Gone:closed');
  expect_error(@() v.at('a'), 'guatiao:Gone:closed');
  held = guatiao.Value(struct('k', 'kept')).at('k');
  expect_equal(held.data(), 'kept', 'a view keeps its value alive');
end

function writes()
  v = guatiao.Value(struct('port', 1, 'tags', {{'b'}}));
  v.set('port', 5901);
  v.set('host', 'h');
  v.at('tags').push('c', 'd');
  v.at('tags').insert(1, 'a');
  v.at('tags').set(2, 'B');
  v.at('tags').set(5, 'e');
  expect_equal(v.data(), struct('port', 5901, 'tags', {{'a', 'B', 'c', 'd', 'e'}}, 'host', 'h'));
  v.set(1, 7);
  expect_equal(v.at('port').data(), 7);
  v.remove('host');
  v.remove('nonesuch');
  v.at('tags').remove(9);
  expect_equal(v.keys(), {'port', 'tags'});
  v.merge(struct('port', 8, 'extra', true));
  expect_equal(v.data(), struct('port', 8, 'tags', {{'a', 'B', 'c', 'd', 'e'}}, 'extra', true));
  v.set('copy', v.at('tags'));
  expect_equal(v.at('copy').data(), v.at('tags').data());
  v.at('tags').set(1, v.at('tags'));
  expect_equal(v.at('tags').at(1).length(), 5);
  v.at('tags').clear();
  expect_equal(v.at('tags').length(), 0);
  expect_error(@() v.push(1), 'guatiao:WrongKind');
  expect_error(@() v.at('port').set('x', 1), 'guatiao:WrongKind');
  expect_error(@() v.at('copy').insert(9, 1), 'guatiao:NotFound');
  v.close();
end

function copies()
  v = guatiao.Value(struct('a', {{1}}));
  w = guatiao.Value(v.at('a'));
  w.push(2);
  expect_equal(v.at('a').length(), 1);
  expect_equal(w.length(), 2);
  c = v.at('a').copy();
  v.close();
  expect_equal(w.data(), {1, 2});
  expect_equal(c.data(), {1});
end

function strings()
  word = native2unicode(uint8([104 195 169 108 108 111 32 228 184 150 231 149 140]), 'UTF-8');   % héllo 世界
  emoji = native2unicode(uint8([240 159 166 128]), 'UTF-8');                                     % a crab
  key = native2unicode(uint8([195 169]), 'UTF-8');
  v = guatiao.Value(containers.Map({key}, {[word emoji]}));
  expect_equal(v.keys(), {key});
  back = v.data();
  expect_equal(back(key), [word emoji]);
  expect_equal(guatiao.serde.emit(v), native2unicode([uint8('{"') uint8([195 169]) uint8('":"') ...
    uint8([104 195 169 108 108 111 32 228 184 150 231 149 140 240 159 166 128]) uint8('"}')], 'UTF-8'));
  bytes = unicode2native(guatiao.serde.emit(emoji), 'UTF-8');
  expect_equal(bytes, uint8([34 240 159 166 128 34]), 'four UTF-8 bytes');
end

function objects()
  p = guatiao.Duration(90, 5e8);
  expect_equal(guatiao.serde.emit(struct('d', p)), '{"d":[90,500000000]}');
  expect_equal(guatiao.serde.emit(guatiao.Date(2026, 10, 21)), '"2026-10-21"');
  a = guatiao.Allocator.counting();
  expect_equal(guatiao.serde.emit(a), ['{"Id":' sprintf('%d', a.Id) '}'], 'a map of its public properties');
end

function depth()
  x = 1;
  for i = 1:128
    x = {x};
  end
  v = guatiao.Value(x);
  expect_equal(v.tag(), 'list');
  expect_error(@() guatiao.Value({x}), 'guatiao:BadValue');
end
