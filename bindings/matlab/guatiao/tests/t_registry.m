% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_registry()
% The registry against a real build: scanning, answers, instances, retire,
% collect and unload, and sources.
  cases = {
    'scans by rule and lists the providers', @scans
    'says a scan of a file names no directory, and a bad rule is the rule', @scan_refusals
    'answers the documented fields for the best provider', @best
    'answers a single table, and a table per kind', @tables
    'builds an instance, and gives a copy of its configuration schema', @instances
    'a provider''s meta is a copy carrying its form', @meta
    'a provider''s form, read from its meta, fits its schema', @form_fits
    'the catalog lists every provider of a folder and builds none', @catalog_lists
    'a catalog entry gives each provider its form', @catalog_forms
    'a catalog names why a library has no digest', @catalog_digest_error
    'keys, ranks and re-keys', @keys
    'the import search is the library''s until the host names path', @import_searches
    'retires a library and frees its key', @retires
    'unloads only when the library agrees', @unloads
    'collects a retired library once nothing of it is alive', @collects
    'refuses use once closed, and closes twice quietly', @closed
    'serves libraries that come and go from a source file', @source_file
    'refuses with the source''s own words, and a file that is no source', @source_refusals
    'tells a source''s note once', @source_note
    'follows the files of a watched folder', @watched
  };
end

function reg = scanned()
  gt_env('examples');
  reg = guatiao.Registry('m-tests', '0.1');
  reg.scan_dir(gt_env('dir'), 'kind=greeter');
end

function scan_refusals()
  gt_env('examples');
  reg = guatiao.Registry('m-tests', '0.1');
  file = [tempname() '.txt'];
  fid = fopen(file, 'w');
  fclose(fid);
  e = expect_error(@() reg.scan_dir(file, 'kind=greeter'), 'guatiao:NotFound');
  expect_true(~isempty(strfind(e.message, 'no directory')), e.message);
  e = expect_error(@() reg.scan_dir(gt_env('dir'), 'not a rule'), 'guatiao:BadValue');
  expect_true(~isempty(strfind(e.message, 'a rule that is not one')), e.message);
  delete(file);
  reg.close();
end

function out = ids(list)
  out = cellfun(@(p) p.id, list, 'UniformOutput', false);
end

function scans()
  reg = guatiao.Registry('m-tests', '0.1');
  report = reg.scan_dir(gt_env('dir'), 'kind=greeter');
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

function meta()
  reg = scanned();
  m = reg.provider_meta('derived_greeter_shouter');
  reg.close();
  sections = m.at('form').at('sections').data();
  expect_equal(cellfun(@(s) s.id, sections, 'UniformOutput', false), {'words', 'volume'});
  m.close();
  reg = scanned();
  expect_equal(reg.provider_meta('derived_greeter_hello'), []);
  expect_error(@() reg.provider_meta('nobody'), 'guatiao:NotFound');
  reg.close();
end

function form_fits()
  reg = scanned();
  m = reg.provider_meta('derived_greeter_shouter');
  form = m.at(guatiao.intake.provider_form_key());
  schema = reg.provider_config('derived_greeter_shouter');
  expect_equal(guatiao.intake.check(schema, form), []);
  expect_equal(form.at('fields').at('prefix').at('placeholder').data(), 'hey');
  expect_equal(form.at('fields').at('marks').at('unit').data(), 'marks');
  expect_true(~guatiao.intake.is_visible(schema, form, 'marks', struct('exclaim', false)), 'hidden');
  expect_true(guatiao.intake.is_visible(schema, form, 'marks', struct('exclaim', true)), 'shown');
  schema.close();
  m.close();
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

function import_searches()
  reg = guatiao.Registry('m-tests', '0.1');
  expect_equal(reg.import_search(), 'library');
  reg.set_import_search('path');
  expect_equal(reg.import_search(), 'path');
  expect_error(@() reg.set_import_search('everywhere'), 'guatiao:BadValue');
  expect_equal(reg.import_search(), 'path');
  reg.close();
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
  again = reg.scan_dir(gt_env('dir'), 'kind=greeter');
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
  expect_error(@() reg.add_source_file(fullfile(gt_env('dir'), 'no-such-file')), 'guatiao:NotFound');
  reg.close();
end

function source_note()
% The shelf written in C answers its configuration's `note` from its
% table's `notes` slot.
  shelf = gt_env('c_source', 'm_shelf_note');
  if isempty(shelf)
    skip_case('the C source: no C compiler built it (set GUATIAO_CC)');
  end
  reg = guatiao.Registry('m-sources', '0.1');
  reg.add_source_file(shelf, struct('ids', {{}}, 'note', 'the shelf is dusty, says C'));
  first = reg.refresh();
  expect_equal(numel(first.notes), 1);
  expect_equal(first.notes{1}.subject, 'shelf');
  expect_equal(first.notes{1}.text, 'the shelf is dusty, says C');
  expect_equal(first.notes{1}.detail.said, 'by C');
  expect_true(~isempty(first.notes{1}.source), 'names its source');
  expect_true(~isfield(reg.refresh(), 'notes'), 'told once');
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
  copy_bytes(gt_env('workspace', 'hello_library'), copy);
  r = reg.refresh();
  expect_equal(cellfun(@(s) s.key, r.loaded, 'UniformOutput', false), {'hello_library'});
  % A mapped image cannot be deleted on Windows; moved aside, it is gone.
  movefile(copy, aside);
  r = reg.refresh();
  expect_equal(cellfun(@(s) s.key, r.retired, 'UniformOutput', false), {'hello_library'});
  expect_equal(numel(reg.libraries()), 0);
  reg.close();
end

function copy_bytes(from, to)
  % Octave's copyfile runs xcopy where no cp.exe is on PATH, and xcopy
  % stops to ask whether the target is a file or a directory.
  in = fopen(from, 'r');
  bytes = fread(in, Inf, '*uint8');
  fclose(in);
  out = fopen(to, 'w');
  fwrite(out, bytes, 'uint8');
  fclose(out);
end

function out = by_id(catalog)
  out = struct();
  for i = 1:numel(catalog)
    out.(catalog{i}.id) = catalog{i};
  end
end

function out = opt(entry, name)
% A field an entry may lack, as [] for guatiao.intake.provider_form.
  if isfield(entry, name)
    out = entry.(name);
  else
    out = [];
  end
end

function hex = sha256_hex(file)
  fid = fopen(file, 'r');
  bytes = fread(fid, Inf, 'uint8=>uint8');
  fclose(fid);
  if exist('OCTAVE_VERSION', 'builtin')
    hex = hash('sha256', char(bytes'));
  else
    md = java.security.MessageDigest.getInstance('SHA-256');
    md.update(bytes);
    hex = lower(reshape(dec2hex(typecast(md.digest(), 'uint8'), 2)', 1, []));
  end
end

function catalog_lists()
  file = gt_env('c_catalog');
  if isempty(file) || ~gt_env('fixture')
    skip_case('the C catalog library: no C compiler built it (set GUATIAO_CC)');
  end
  reg = guatiao.Registry('catalog-host', '1.0');
  report = reg.scan_dir(fileparts(file));
  expect_true(isempty(report.failed) && numel(report.loaded) == 1, 'loaded');
  catalog = by_id(reg.catalog());
  expect_equal(sort(fieldnames(catalog))', {'catalog_configured', 'catalog_implied', 'catalog_plain', 'catalog_unavailable'});
  digest = ['sha256:' sha256_hex(file)];

  configured = catalog.catalog_configured;
  expect_equal(configured.kinds, {'counter'});
  expect_equal(configured.display_name, 'Configured, with a form');
  expect_equal(configured.version, '2.0');
  expect_equal(configured.library, 'catalog_library');
  expect_equal(configured.library_version, '1.0');
  expect_equal(configured.digest, digest);
  expect_true(configured.available && ~isfield(configured, 'reason'), 'runs here');
  expect_equal(configured.config.required, {'host', 'port'});
  expect_equal(configured.meta.form.sections{1}.title, 'Network');

  implied = catalog.catalog_implied;
  expect_equal(implied.version, '1.0');
  expect_true(~isfield(implied, 'meta'), 'no meta');
  token = implied.config.properties.token;
  expect_true(token('x-sensitive'), 'sensitive');

  unavailable = catalog.catalog_unavailable;
  expect_true(~unavailable.available, 'cannot run');
  expect_equal(unavailable.reason, 'no device here');
  expect_true(~isfield(unavailable, 'config') && ~unavailable.has_config, 'takes none');
  expect_true(catalog.catalog_plain.available, 'plain runs');

  % Listing asked the library for nothing: its count says so, and so does
  % its refusal of the first instance asked for afterwards.
  expect_equal(gtfixture('catalog_built', file), 0);
  try
    reg.create('catalog_configured', struct('host', 'localhost', 'port', 8080));
    message = 'no refusal';
  catch err
    message = err.message;
  end
  expect_true(~isempty(strfind(message, 'built nothing: the first create this library was asked')), message);
  expect_equal(gtfixture('catalog_built', file), 1);
  reg.close();
end

function catalog_forms()
  reg = scanned();
  catalog = by_id(reg.catalog());
  shouter = catalog.derived_greeter_shouter;
  form = guatiao.intake.provider_form(opt(shouter, 'config'), opt(shouter, 'meta'));
  sections = form.at('sections').data();
  expect_equal(cellfun(@(s) s.id, sections, 'UniformOutput', false), {'words', 'volume'});
  expect_equal(guatiao.intake.check(shouter.config, form), []);
  form.close();
  hello = catalog.derived_greeter_hello;
  expect_equal(guatiao.intake.provider_form(opt(hello, 'config'), opt(hello, 'meta')), []);
  reg.close();
end

function catalog_digest_error()
  file = gt_env('c_catalog');
  if isempty(file)
    skip_case('the C catalog library: no C compiler built it (set GUATIAO_CC)');
  end
  reg = guatiao.Registry('catalog-host', '1.0');
  reg.load_file(file);
  % Another file at the path, before anything asked: renaming a mapped file
  % works on every platform, where overwriting one does not.
  movefile(file, fullfile(fileparts(file), 'moved_away.bin'));
  fid = fopen(file, 'w');
  fwrite(fid, 'not the library');
  fclose(fid);
  catalog = reg.catalog();
  expect_equal(numel(catalog), 4);
  for i = 1:numel(catalog)
    expect_true(~isfield(catalog{i}, 'digest'), 'no digest');
    expect_equal(catalog{i}.digest_error, 'the file changed since it was loaded');
  end
  reg.close();
end
