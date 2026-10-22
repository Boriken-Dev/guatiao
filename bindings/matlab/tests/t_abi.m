% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_abi()
% The gateway compiles against copies of the headers, which must be the
% committed ones; the status table must be the header's.
  cases = {
    'the header copies are the committed headers', @header_copies
    'the status names and codes are the header''s', @status_table
    'the library is found through GUATIAO_LIBRARY', @library_found
    'errors carry guatiao:<Status> identifiers', @identifiers
    'the gateway stays loaded once the library is open', @locked
  };
end

function header_copies()
  pairs = {'guatiao/include/guatiao.h', 'guatiao-serde/include/guatiao_serde.h', ...
           'guatiao-intake/include/guatiao_intake.h'};
  for i = 1:numel(pairs)
    [~, base, ext] = fileparts(pairs{i});
    committed = bytes_of(fullfile(repo_root(), 'crates', pairs{i}));
    copy = bytes_of(fullfile(repo_root(), 'bindings', 'matlab', 'src', [base ext]));
    expect_true(isequal(committed, copy), [base ext ' matches']);
  end
end

function b = bytes_of(path)
  fid = fopen(path, 'r');
  b = fread(fid, Inf, '*uint8');
  fclose(fid);
end

function status_table()
  header = read_text_file(fullfile(repo_root(), 'crates', 'guatiao', 'include', 'guatiao.h'));
  [tok] = regexp(header, 'GUATIAO_(OK|ERR_[A-Z_]+) = ([0-9]+)', 'tokens');
  codes = cellfun(@(t) str2double(t{2}), tok);
  s = guatiao.statuses();
  ours = cellfun(@(n) s.(n), fieldnames(s));
  expect_equal(sort(ours(:))', sort(codes(:))', 'every code');
  names = cellfun(@(t) t{1}, tok, 'UniformOutput', false);
  expect_equal(s.BadValue, codes(strcmp(names, 'ERR_BAD_VALUE')), 'BadValue');
  expect_equal(s.Internal, codes(strcmp(names, 'ERR_INTERNAL')), 'Internal');
  expect_equal(s.Unsupported, codes(strcmp(names, 'ERR_UNSUPPORTED')), 'Unsupported');
end

function library_found()
  expect_true(guatiao.has_export('guatiao_value_map'), 'core');
  expect_true(~guatiao.has_export('guatiao_no_such_export'), 'absent export');
  expect_true(guatiao.has_export('guatiao_json_parse_ex', 'guatiao_serde'), 'serde');
  expect_true(guatiao.has_export('guatiao_intake_check', 'guatiao_intake'), 'intake');
  expect_error(@() guatiao.has_export('x', 'nonesuch'), 'guatiao:BadValue');
  expect_error(@() guatiao.use_library(fullfile(tempdir(), 'elsewhere', 'guatiao.dll')), 'guatiao:Busy');
end

function identifiers()
  e = expect_error(@() guatiao.Value(NaN), 'guatiao:BadValue');
  expect_true(~isempty(strfind(e.message, 'NaN')), 'the words');
  v = guatiao.Value(1);
  v.close();
  expect_error(@() v.data(), 'guatiao:Gone:closed');
  expect_error(@() guatiao_mex('no_such_command'), 'guatiao:BadValue');
  expect_error(@() guatiao_mex('value_new', @sin), 'guatiao:BadValue');
end

function locked()
  guatiao.Value(1);
  expect_true(mislocked('guatiao_mex'), 'locked');
  clear('guatiao_mex');
  expect_true(mislocked('guatiao_mex'), 'still locked after clear');
  v = guatiao.Value(struct('a', 1));
  expect_equal(v.at('a').data(), 1, 'works after clear');
end
