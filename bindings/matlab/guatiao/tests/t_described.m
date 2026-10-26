% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_described()
% Kind classes written from a kinds document by guatiao.write_kinds: the
% committed greeter and tally packages are what their documents write, a
% kind that exists only as a document is served from M and called in
% every slot, and a document no class can be written from is refused.
  cases = {
    'the committed kind classes are what their documents write', @current
    'a kind written from its document is served and called in every slot', @tally
    'a document no class can be written from is refused', @refusals
    'a stale class is found, and written again', @stale
  };
end

function current()
  root = repo_root();
  guatiao.write_kinds(fullfile(root, 'examples', 'greeter_kind', 'include', 'greeter_kind.kinds.json'), ...
                      fullfile(root, 'examples', 'greeter_kind', 'matlab', '+greeterkind'), 'Check', true);
  guatiao.write_kinds(fullfile(root, 'examples', 'tally_kind', 'include', 'tally_kind.kinds.json'), ...
                      fullfile(root, 'examples', 'tally_kind', 'matlab', '+tallykind'), 'Check', true);
end

function tally()
  gt_env('examples');
  e = guatiao.internal.kinds('of', 'tallykind.Tally');
  expect_equal(e.desc.floor_hash, 1286177969, 'the floor hash the document names');
  expect_equal(e.size, 8 + 8 * 7, 'a word per slot');
  w = guatiao.internal.kinds('of', 'tallykind.Watcher');
  expect_equal(w.floor_size, 8 + 8 * 2, 'destroy and changed');
  reg = guatiao.Registry('m-tally', '0.1');
  reg.register_library(guatiao.Library('m_tally', '0.1.0', {'gtt.Tallies'}));
  t = reg.bind('m_tally_tallies', 'tallykind.Tally');
  expect_equal(double(t.add('apples', 3)), 3);
  expect_error(@() t.add('', 1), 'guatiao:BadValue', 'a refusal crosses');
  expect_equal(double(t.total('apples')), 3);
  counts = guatiao.Value(struct('pears', 2, 'apples', 1));
  expect_equal(double(t.load(counts)), 2);
  expect_true(counts.isabsent(), 'handed over');
  everything = t.all();
  got = everything.data();
  expect_equal(sort(fieldnames(got))', {'apples', 'pears'});
  expect_equal([double(got.apples), double(got.pears)], [4, 2]);
  everything.close();
  expect_equal(t.known('plums'), false);
  expect_equal(double(t.checksum(uint8([250 10 1]))), 261);
  heard = gtt.Watching();
  t.report(heard);
  expect_equal(heard.Said, {'apples=4', 'pears=2'});
  t.close();
  reg.close();
end

function refusals()
  dir = tempname();
  mkdir(dir);
  cleanup = onCleanup(@() rmdir(dir, 's'));
  slot = '{"name": "f", "required": true, "fallible": false, "args": [%s]%s}';
  cases = {
    sprintf(slot, '{"name": "l", "type": "list", "pass": "borrowed"}', ''), 'a list is not borrowed'
    sprintf(slot, '{"name": "o", "type": "object", "kind": "far", "pass": "lent"}', ''), 'not in this document'
    sprintf(slot, '', ', "answer": {"type": "bytes"}'), 'an answer cannot be bytes'
    strrep(sprintf(slot, '', ''), '"f"', '"close"'), 'names no method'
    strrep(sprintf(slot, '', ''), '"f"', '"until"'), 'names no method'
    sprintf(slot, '{"name": "x", "type": "u256"}', ''), 'not a type an argument can have'
    sprintf(slot, '{"name": "self", "type": "i8"}, {"name": "self_", "type": "i8"}', ''), 'two arguments are self_'
    sprintf(slot, '{"name": "b", "type": "bytes_mut"}', ''), 'on an object kind only'
    sprintf(slot, '{"name": "o", "type": "object", "kind": "k", "pass": "lent"}', ''), 'no object kind'
    [strrep(sprintf(slot, '', ''), '"required": true', '"required": false') ', ' ...
     strrep(sprintf(slot, '', ''), '"f"', '"g"')], 'the required slots come first'
    [sprintf(slot, '', '') ', ' sprintf(slot, '', '')], 'two slots are f'
  };
  for i = 1:size(cases, 1)
    path = fullfile(dir, 'k.kinds.json');
    fid = fopen(path, 'w');
    fprintf(fid, '{"format": 1, "kinds": [{"name": "k", "floor_hash": 1, "object": false, "slots": [%s]}]}', cases{i, 1});
    fclose(fid);
    e = expect_error(@() guatiao.write_kinds(path, fullfile(dir, '+k')), 'guatiao:BadValue', cases{i, 2});
    expect_true(~isempty(strfind(e.message, cases{i, 2})), sprintf('%s: %s', cases{i, 2}, e.message));
  end
  fid = fopen(fullfile(dir, 'other.json'), 'w');
  fprintf(fid, '{"format": 2, "kinds": []}');
  fclose(fid);
  expect_error(@() guatiao.write_kinds(fullfile(dir, 'other.json'), fullfile(dir, '+k')), 'guatiao:BadValue', 'format 2');
  expect_error(@() guatiao.write_kinds(fullfile(dir, 'none.json'), fullfile(dir, '+k')), 'guatiao:NotFound', 'no file');
  doc = fullfile(repo_root(), 'examples', 'tally_kind', 'include', 'tally_kind.kinds.json');
  e = expect_error(@() guatiao.write_kinds(doc, dir), 'guatiao:BadValue', 'no package folder');
  expect_true(~isempty(strfind(e.message, 'no package folder')), e.message);
  expect_true(~exist(fullfile(dir, '+k'), 'dir'), 'a refusal writes nothing');
end

function stale()
  dir = tempname();
  mkdir(dir);
  cleanup = onCleanup(@() rmdir(dir, 's'));
  doc = fullfile(repo_root(), 'examples', 'tally_kind', 'include', 'tally_kind.kinds.json');
  folder = fullfile(dir, '+tallycopy');
  files = guatiao.write_kinds(doc, folder);
  expect_equal(numel(files), 2);
  text = fileread(files{2});
  expect_true(~isempty(strfind(text, 'guatiao.lent(''tallycopy.Watcher'')')), 'named by its own package');
  guatiao.write_kinds(doc, folder, 'Check', true);
  fid = fopen(files{1}, 'a');
  fprintf(fid, '%% edited\n');
  fclose(fid);
  expect_error(@() guatiao.write_kinds(doc, folder, 'Check', true), 'guatiao:Stale', 'an edited class');
  guatiao.write_kinds(doc, folder);
  guatiao.write_kinds(doc, folder, 'Check', true);
  % A class written for a kind the document no longer holds is stale too,
  % and writing again removes it; a file of the author's own stays.
  fid = fopen(fullfile(folder, 'Gone.m'), 'w');
  fprintf(fid, '%% GENERATED by guatiao.write_kinds from old.kinds.json. DO NOT EDIT.\n');
  fclose(fid);
  fid = fopen(fullfile(folder, 'Mine.m'), 'w');
  fprintf(fid, 'function mine()\nend\n');
  fclose(fid);
  expect_error(@() guatiao.write_kinds(doc, folder, 'Check', true), 'guatiao:Stale', 'an orphan');
  guatiao.write_kinds(doc, folder);
  expect_true(~exist(fullfile(folder, 'Gone.m'), 'file'), 'the orphan is removed');
  expect_true(exist(fullfile(folder, 'Mine.m'), 'file') == 2, 'the author''s file stays');
  guatiao.write_kinds(doc, folder, 'Check', true);
end
