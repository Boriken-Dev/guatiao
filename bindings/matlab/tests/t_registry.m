% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_registry()
% The registry against a real build: scanning, answers, instances, retire,
% collect and unload, and sources.
  cases = {
    'scans by rule and lists the providers', @scans
    'answers the documented fields for the best provider', @best
    'answers a single table, and a table per kind', @tables
    'builds an instance, and gives a copy of its configuration schema', @instances
    'keys, ranks and re-keys', @keys
    'retires a library and frees its key', @retires
    'unloads only when the library agrees', @unloads
    'collects a retired library once nothing of it is alive', @collects
    'refuses use once closed, and closes twice quietly', @closed
    'serves libraries that come and go from a source file', @source_file
    'refuses with the source''s own words, and a file that is no source', @source_refusals
    'follows the files of a watched folder', @watched
  };
end

function reg = scanned()
  gt_env('examples');
  reg = guatiao.Registry('m-tests', '0.1');
  reg.scan_dir(gt_env('library'), 'kind=greeter');
end

function out = ids(list)
  out = cellfun(@(p) p.id, list, 'UniformOutput', false);
end

function scans()
  reg = guatiao.Registry('m-tests', '0.1');
  report = reg.scan_dir(gt_env('library'), 'kind=greeter');
  loaded = strjoin(cellfun(@(s) guatiao.serde.emit(s), report.loaded, 'UniformOutput', false), ' ');
  expect_true(~isempty(strfind(loaded, 'derived_greeter')), 'the Rust greeter loaded');
  expect_true(~isempty(strfind(loaded, 'hello_library')), 'hello_library loaded');
  expect_true(any(strcmp(ids(reg.providers('greeter')), 'derived_greeter_hello')));
  expect_true(any(strcmp(ids(reg.providers('greeter')), 'hello_library_greeter')));
  reg.close();
end

function best()
  reg = scanned();
  b = reg.best('greeter');
  fields = {'key', 'id', 'version', 'library', 'display_name', 'from', 'kinds', 'has_config', 'vtable_size'};
  for i = 1:numel(fields)
    expect_true(isfield(b, fields{i}), fields{i});
  end
  expect_true(any(strcmp(b.kinds, 'greeter')));
  why = reg.why_not('nothing-claims-this-kind');
  expect_equal(why.available, false);
  expect_equal(why.why, 'nothing-claims-it');
  expect_error(@() reg.provider('nonesuch'), 'guatiao:NotFound');
  reg.close();
end

function tables()
  reg = scanned();
  t = reg.provider_table('hello_library_greeter');
  expect_true(~isempty(t.table) && t.size > 0, 'a single table');
  t = reg.provider_table('derived_greeter_hello');
  expect_true(isempty(t.table), 'no single table');
  t = reg.provider_table('derived_greeter_hello', 'greeter');
  expect_true(~isempty(t.table), 'a table per kind');
  t = reg.provider_table('derived_greeter_hello', 'codec');
  expect_true(isempty(t.table), 'none for another kind');
  c = reg.bind('derived_greeter_hello', 'greeterkind.Counter');
  expect_true(isnumeric(c.count()), 'count');
  reg.close();
end

function instances()
  reg = scanned();
  schema = reg.provider_config('derived_greeter_shouter');
  expect_equal(schema.tag(), 'map');
  expect_equal(reg.provider_config('derived_greeter_hello'), []);
  inst = reg.create('derived_greeter_shouter', struct('prefix', 'hey'));
  g = inst.bind('greeterkind.Greeter');
  answer = g.greet('ana');
  greeting = answer.at('greeting').data();
  expect_true(strncmp(greeting, 'hey', 3), 'built from the configuration');
  inst.close();
  inst.close();
  expect_error(@() g.greet('ana'), 'guatiao:Gone:closed');
  e = expect_error(@() reg.create('derived_greeter_shouter', struct()), 'guatiao:BadValue');
  expect_true(~isempty(e.message), 'says why');
  reg.close();
  expect_equal(schema.tag(), 'map', 'a copy outlives the registry');
  schema.close();
end

function keys()
  reg = scanned();
  reg.set_priority('derived_greeter_hello', 7);
  expect_equal(reg.priority('derived_greeter_hello'), 7);
  reg.keyed_by('%id@%version');
  keyed = cellfun(@(p) p.key, reg.providers('greeter'), 'UniformOutput', false);
  expect_true(any(strncmp(keyed, 'derived_greeter_hello@', 22)));
  expect_error(@() reg.keyed_by('%nonsense'), 'guatiao:');
  reg.close();
end

function retires()
  reg = scanned();
  reg.retire('derived_greeter');
  expect_true(~any(strcmp(ids(reg.providers('greeter')), 'derived_greeter_hello')), 'out');
  expect_true(any(strcmp(ids(reg.providers('greeter')), 'hello_library_greeter')), 'the other stays');
  again = reg.scan_dir(gt_env('library'), 'kind=greeter');
  loaded = strjoin(cellfun(@(s) guatiao.serde.emit(s), again.loaded, 'UniformOutput', false), ' ');
  expect_true(~isempty(strfind(loaded, 'derived_greeter')), 'its key is free again');
  expect_error(@() reg.retire('derived_greeter_hello'), 'guatiao:NotFound');
  reg.close();
end

function unloads()
  reg = scanned();
  inst = reg.create('derived_greeter_shouter', struct('prefix', 'hey'));
  expect_error(@() reg.unload('derived_greeter'), 'guatiao:Busy');
  inst.close();
  reg.unload('derived_greeter');
  expect_true(~any(strcmp(ids(reg.libraries()), 'derived_greeter')), 'unloaded');
  expect_error(@() reg.unload('derived_greeter'), 'guatiao:NotFound');
  reg.close();
end

function collects()
  reg = scanned();
  expect_equal(reg.collect(), cell(1, 0));
  inst = reg.create('derived_greeter_shouter', struct('prefix', 'hey'));
  reg.retire('derived_greeter');
  expect_equal(reg.collect(), cell(1, 0), 'an instance holds it');
  inst.close();
  expect_equal(reg.collect(), {'derived_greeter'});
  expect_equal(reg.collect(), cell(1, 0));
  reg.close();
end

function closed()
  reg = guatiao.Registry('m-tests', '0.1');
  reg.close();
  expect_error(@() reg.libraries(), 'guatiao:Gone:closed');
  reg.close();
  expect_true(~reg.is_open());
end

function source_file()
  gt_env('examples');
  shelf = gt_env('c_source', 'm_shelf_a');
  if isempty(shelf) || ~gt_env('fixture')
    skip_case('the C source: no C compiler built it (set GUATIAO_CC)');
  end
  reg = guatiao.Registry('m-sources', '0.1');
  reg.add_source_file(shelf, struct('ids', {{'shelf_a', 'shelf_b'}}));
  expect_equal(numel(reg.libraries()), 0);
  first = reg.refresh();
  expect_equal(cellfun(@(s) s.id, first.loaded, 'UniformOutput', false), {'shelf_a', 'shelf_b'});
  c = reg.bind('shelf_a_counter', 'greeterkind.Counter');
  expect_equal(c.count(), 1);
  quiet = reg.refresh();
  expect_true(isempty(quiet.loaded) && isempty(quiet.retired) && isempty(quiet.refused), 'nothing changed');
  shelf_call(shelf, 'shelf_take', 1);
  second = reg.refresh();
  expect_equal(cellfun(@(s) s.id, second.retired, 'UniformOutput', false), {'shelf_b'});
  reg.collect();
  expect_equal(shelf_call(shelf, 'shelf_closed', 1), 1);
  shelf_call(shelf, 'shelf_refuse', true);
  shelf_call(shelf, 'shelf_put', 2);
  third = reg.refresh();
  expect_equal(numel(third.refused), 1);
  expect_equal(third.refused{1}.id, 'shelf_c');
  expect_equal(third.refused{1}.why, 'the shelf refuses, says C');
  reg.close();
end

function out = shelf_call(shelf, name, arg)
% The C shelf's controls, called by the fixture.
  out = gtfixture('shelf', shelf, name, double(arg));
end

function source_refusals()
  shelf = gt_env('c_source', 'm_shelf_b');
  if isempty(shelf)
    skip_case('the C source: no C compiler built it (set GUATIAO_CC)');
  end
  reg = guatiao.Registry('m-sources', '0.1');
  e = expect_error(@() reg.add_source_file(shelf, struct()), 'guatiao:');
  expect_true(~isempty(strfind(e.message, '`ids`')), 'the source''s own words');
  reg.add_source_file(shelf, struct('ids', {{}}));
  e = expect_error(@() reg.add_source_file(shelf, struct('ids', {{}})), 'guatiao:');
  expect_true(~isempty(strfind(e.message, 'already added')), 'once');
  expect_error(@() reg.add_source_file(gt_env('workspace', 'derived_greeter')), 'guatiao:WrongKind');
  expect_error(@() reg.add_source_file(fullfile(gt_env('library'), 'no-such-file')), 'guatiao:NotFound');
  reg.close();
end

function watched()
  dir = [tempname() '-watched'];
  w = fullfile(dir, 'watched');
  aside = fullfile(dir, 'aside');
  mkdir(w);
  mkdir(aside);
  reg = guatiao.Registry('m-sources', '0.1');
  reg.watch_dir(w, 'kind=greeter');
  expect_error(@() reg.watch_dir(w), 'guatiao:');
  r = reg.refresh();
  expect_equal(numel(r.loaded), 0);
  copy = fullfile(w, gt_env('file', 'hello_m_watched'));
  copyfile(gt_env('workspace', 'hello_library'), copy);
  r = reg.refresh();
  expect_equal(cellfun(@(s) s.key, r.loaded, 'UniformOutput', false), {'hello_library'});
  % A mapped image cannot be deleted on Windows; moved aside, it is gone.
  movefile(copy, aside);
  r = reg.refresh();
  expect_equal(cellfun(@(s) s.key, r.retired, 'UniformOutput', false), {'hello_library'});
  expect_equal(numel(reg.libraries()), 0);
  reg.close();
end
