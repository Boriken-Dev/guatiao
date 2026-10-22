% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Config
% A configuration's description: field names (MATLAB identifiers) and
% guatiao.field specs, in order, and optionally 'Doc', its description.
% schema() is the JSON Schema a host checks a configuration against;
% read(x) answers a struct with the defaults filled in.
%
%   C = guatiao.Config('prefix', guatiao.field('string', 'Doc', 'What greetings start with.'), ...
%                      'times', guatiao.field('integer', 'Default', 1));
%   C.schema()                           % MATLAB data
%   s = C.read(struct('prefix', 'hey'))  % s.times == 1

  properties (SetAccess = private)
    Names = {}    % the fields' names, in order
    Fields = {}   % their guatiao.field specs
    Doc = ''      % the configuration's own description
  end

  methods
    function self = Config(varargin)
      if mod(numel(varargin), 2) ~= 0
        error('guatiao:BadValue', 'guatiao.Config takes names and guatiao.field specs');
      end
      for i = 1:2:numel(varargin)
        name = varargin{i};
        spec = varargin{i + 1};
        if strcmpi(name, 'Doc') && ischar(spec)
          self.Doc = spec;
          continue
        end
        if ~ischar(name) || ~isvarname(name)
          error('guatiao:BadValue', 'a field''s name is a MATLAB identifier');
        end
        if any(strcmp(self.Names, name))
          error('guatiao:BadValue', 'two fields are named ''%s''', name);
        end
        if ischar(spec) || isa(spec, 'guatiao.Config')
          spec = guatiao.field(spec);
        end
        if ~isstruct(spec) || ~isfield(spec, 'type')
          error('guatiao:BadValue', 'the field ''%s'' is a guatiao.field spec', name);
        end
        self.Names{end + 1} = name;
        self.Fields{end + 1} = spec;
      end
    end

    function s = schema(self)
    % The JSON Schema (2020-12) of the configuration, as MATLAB data: an
    % object whose properties are the fields in order, keys it does not
    % declare refused, a field required unless it has a default or is
    % nullable.
      props = struct();
      required = {};
      for i = 1:numel(self.Names)
        f = self.Fields{i};
        p = guatiao.Config.type_schema(f.type);
        if f.nullable
          p = struct('anyOf', {{p, struct('type', 'null')}});
        end
        if ~isempty(f.doc)
          p.description = f.doc;
        end
        if f.has_default && ~isempty(f.default)
          p.default = f.default;
        end
        for k = 1:size(f.keywords, 1)
          p.(f.keywords{k, 1}) = f.keywords{k, 2};
        end
        props.(self.Names{i}) = p;
        if ~f.has_default && ~f.nullable
          required{end + 1} = self.Names{i}; %#ok<AGROW>
        end
      end
      s = struct('type', 'object', 'properties', props);
      if ~isempty(required)
        s.required = required;
      end
      s.additionalProperties = false;
      if ~isempty(self.Doc)
        s.description = self.Doc;
      end
    end

    function s = read(self, x, at)
    % A configuration (a guatiao value, a view, or MATLAB data) read as a
    % struct: every field present, defaults filled in, each read as its
    % type (an integer exactly, a date as a guatiao.Date, ...). A refusal
    % raises guatiao:BadValue naming the path; keys not declared are ignored.
      if nargin < 3
        at = '';
      end
      if isa(x, 'guatiao.Tree')
        x = x.data();
      end
      if isa(x, 'containers.Map')
        m = x;
      elseif isstruct(x) && isscalar(x)
        m = guatiao.Config.struct_map(x);
      else
        guatiao.Config.refuse(at, 'a map', x);
      end
      s = struct();
      for i = 1:numel(self.Names)
        name = self.Names{i};
        f = self.Fields{i};
        where = guatiao.Config.join(at, name);
        if isKey(m, name) && ~isa(m(name), 'guatiao.Absent')
          v = m(name);
          if isempty(v) && isnumeric(v) && f.nullable
            s.(name) = [];
          else
            s.(name) = guatiao.Config.read_as(f.type, v, where);
          end
        elseif f.has_default
          s.(name) = f.default;
        elseif f.nullable
          s.(name) = [];
        else
          error('guatiao:BadValue', 'at %s: required', where);
        end
      end
    end
  end

  methods (Static, Hidden)
    function p = type_schema(type)
      if isa(type, 'guatiao.Config')
        p = type.schema();
        return
      end
      switch type
        case 'string'
          p = struct('type', 'string');
        case 'integer'
          p = struct('type', 'integer', 'minimum', -9007199254740992, 'maximum', 9007199254740992);
        case 'number'
          p = struct('type', 'number');
        case 'boolean'
          p = struct('type', 'boolean');
        case 'bytes'
          p = struct('type', 'bytes');
        case 'list'
          p = struct('type', 'array', 'items', struct());
        case 'map'
          p = struct('type', 'object', 'additionalProperties', struct());
        case 'any'
          p = struct();
        case 'date'
          p = struct('type', 'string', 'format', 'date');
        case 'time'
          p = struct('type', 'string', 'pattern', ...
                     '^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?$');
        case 'datetime'
          p = struct('type', 'string', 'pattern', ['^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]' ...
                     '([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?$']);
        case 'timestamp'
          p = struct('type', 'string', 'format', 'date-time');
        case 'span'
          p = struct('type', 'string', 'format', 'duration');
        case 'duration'
          secs = struct('type', 'integer', 'minimum', 0, 'maximum', guatiao.Number('18446744073709551615'));
          nanos = struct('type', 'integer', 'minimum', 0, 'maximum', 999999999);
          p = struct('type', 'array', 'prefixItems', {{secs, nanos}}, 'items', false, 'minItems', 2, 'maxItems', 2);
      end
    end

    function v = read_as(type, x, at)
      if isa(type, 'guatiao.Config')
        v = type.read(x, at);
        return
      end
      switch type
        case 'string'
          if ~ischar(x)
            guatiao.Config.refuse(at, 'text', x);
          end
          v = x;
        case 'integer'
          if ~(isnumeric(x) && isscalar(x) && isreal(x)) || double(x) ~= fix(double(x)) || ...
             abs(double(x)) > 9007199254740992
            guatiao.Config.refuse(at, 'an integer', x);
          end
          v = double(x);
        case 'number'
          if isa(x, 'guatiao.Number')
            v = double(x);
          elseif isnumeric(x) && isscalar(x) && isreal(x)
            v = double(x);
          else
            guatiao.Config.refuse(at, 'a number', x);
          end
        case 'boolean'
          if ~(islogical(x) && isscalar(x))
            guatiao.Config.refuse(at, 'a boolean', x);
          end
          v = x;
        case 'bytes'
          if ~isa(x, 'uint8')
            guatiao.Config.refuse(at, 'bytes', x);
          end
          v = x;
        case 'list'
          if ~iscell(x)
            guatiao.Config.refuse(at, 'a list', x);
          end
          v = x;
        case 'map'
          if ~(isstruct(x) || isa(x, 'containers.Map'))
            guatiao.Config.refuse(at, 'a map', x);
          end
          v = x;
        case 'any'
          v = x;
        case {'date', 'time', 'datetime', 'timestamp', 'span'}
          if ~ischar(x)
            guatiao.Config.refuse(at, ['a ' type], x);
          end
          [v, problem] = guatiao.time.parse(type, x);
          if isempty(v)
            error('guatiao:BadValue', 'at %s: not a %s (at byte %d, expected %s)', at, type, problem.at, ...
                  problem.expected);
          end
        case 'duration'
          v = guatiao.time.read_length(x);
      end
    end

    function m = struct_map(s)
      m = containers.Map('KeyType', 'char', 'ValueType', 'any');
      names = fieldnames(s);
      for i = 1:numel(names)
        m(names{i}) = s.(names{i});
      end
    end

    function p = join(at, name)
      if isempty(at)
        p = name;
      else
        p = [at '.' name];
      end
    end

    function refuse(at, want, x)
      if isempty(at)
        at = 'the top';
      end
      error('guatiao:BadValue', 'at %s: want %s, got %s', at, want, guatiao.Config.kind_of(x));
    end

    function k = kind_of(x)
      if isempty(x) && isnumeric(x)
        k = 'null';
      elseif ischar(x)
        k = 'string';
      elseif islogical(x)
        k = 'boolean';
      elseif isa(x, 'uint8')
        k = 'bytes';
      elseif isnumeric(x) || isa(x, 'guatiao.Number')
        k = 'number';
      elseif iscell(x)
        k = 'list';
      elseif isstruct(x) || isa(x, 'containers.Map')
        k = 'map';
      else
        k = class(x);
      end
    end
  end
end
