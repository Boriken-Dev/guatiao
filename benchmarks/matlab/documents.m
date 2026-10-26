% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function documents(save, samples)
% A document to MATLAB data and back, two ways: through a value
% (guatiao.serde.parse(text).data(), guatiao.serde.emit(guatiao.Value(x)))
% and through the wire bytes read and written in the gateway
% (guatiao.serde.data(text), guatiao.serde.emit(x)). No callback way: a
% MATLAB error must never unwind through the library's frames.
%
%   octave-cli --eval "addpath bindings/matlab/guatiao/inst benchmarks/matlab; documents('documents-octave')"
%
% with GUATIAO_LIBRARY naming a release build and the gateway built.
% Writes benchmarks/results/NAME.json in the runner's shape.
  if nargin < 1
    save = '';
  end
  if nargin < 2
    samples = 15;
  end
  servers = cell(1, 50);
  for i = 0:49
    servers{i + 1} = struct('host', sprintf('node-%d.example', i), 'port', 8000 + i, 'weight', i / 7, ...
                            'tags', {{'edge', 'eu', sprintf('%d', i)}}, 'enabled', mod(i, 3) ~= 0, ...
                            'limits', struct('rps', 1000 + i, 'burst', 2.5, 'note', []));
  end
  cases = {
    'config', struct('name', 'fleet', 'servers', {servers})
    'floats_10k', num2cell((0:9999) * 1.25 + 0.5)
    'text_1mb', struct('blob', repmat('x', 1, 2^20))
  };
  names = {};
  results = {};
  for c = 1:size(cases, 1)
    x = cases{c, 2};
    text = guatiao.serde.emit(x);
    assert(isequal(guatiao.serde.data(text), parse_value(text)), 'the two ways read alike');
    assert(strcmp(render_value(x), text), 'the two ways write alike');
    ways = {
      'parse_value', @() parse_value(text)
      'parse_tape', @() guatiao.serde.data(text)
      'render_value', @() render_value(x)
      'render_tape', @() guatiao.serde.emit(x)
    };
    for w = 1:size(ways, 1)
      name = [cases{c, 1} '_' ways{w, 1}];
      ms = time_it(ways{w, 2}, samples);
      names{end + 1} = name; %#ok<AGROW>
      results{end + 1} = ms; %#ok<AGROW>
      fprintf('%-32s %12.1f us\n', name, median(ms) * 1000);
    end
  end
  if ~isempty(save)
    here = fileparts(mfilename('fullpath'));
    path = fullfile(here, '..', 'results', [save '.json']);
    f = fopen(path, 'w');
    fprintf(f, '{\n  "name": "%s",\n  "metrics": {\n', save);
    for i = 1:numel(names)
      ms = results{i};
      sep = ',';
      if i == numel(names)
        sep = '';
      end
      fprintf(f, '    "%s": {"min_ms": %.6f, "median_ms": %.6f, "max_ms": %.6f}%s\n', names{i}, min(ms), median(ms), max(ms), sep);
    end
    fprintf(f, '  },\n  "raw": {"processor": "%s", "interpreter": "%s"}\n}\n', computer(), interpreter());
    fclose(f);
    fprintf('saved %s\n', path);
  end
end

function x = parse_value(text)
  v = guatiao.serde.parse(text);
  x = v.data();
  v.close();
end

function t = render_value(x)
  v = guatiao.Value(x);
  t = guatiao.serde.emit(v);
  v.close();
end

function ms = time_it(f, samples)
  f();
  n = 1;
  while true
    t0 = tic;
    for i = 1:n
      f();
    end
    if toc(t0) >= 0.02
      break
    end
    n = n * 2;
  end
  ms = zeros(1, samples);
  for s = 1:samples
    t0 = tic;
    for i = 1:n
      f();
    end
    ms(s) = toc(t0) / n * 1000;
  end
end

function s = interpreter()
  if exist('OCTAVE_VERSION', 'builtin')
    s = ['Octave ' OCTAVE_VERSION];
  else
    s = ['MATLAB ' version];
  end
end
