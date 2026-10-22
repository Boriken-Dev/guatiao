% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function s = provider_spec(p)
% A provider spec with every field settled, from a class name (its
% Provider constant, or the defaults) or a guatiao.provider spec with Class.
  if ischar(p)
    cls = p;
    s = declared(cls);
    if isempty(s)
      s = guatiao.provider();
    end
    s.Class = cls;
  elseif isstruct(p) && isfield(p, 'Class') && ~isempty(p.Class)
    s = p;
  else
    error('guatiao:BadValue', 'a provider is a class name or a guatiao.provider spec with Class set');
  end
  cls = s.Class;
  if isempty(meta.class.fromName(cls))
    error('guatiao:NotFound', 'no class %s', cls);
  end
  if isempty(s.Serves)
    s.Serves = kinds_of(cls);
  end
  if isempty(s.Serves)
    error('guatiao:BadValue', '%s serves no kind: inherit one, or name them with ''Serves''', cls);
  end
  for k = 1:numel(s.Serves)
    e = guatiao.internal.kinds('of', s.Serves{k});
    if e.desc.object
      error('guatiao:BadValue', '%s: %s is an object kind; a provider serves provider kinds', cls, s.Serves{k});
    end
  end
  if isempty(s.Name)
    parts = strsplit(cls, '.');
    s.Name = lower(regexprep(parts{end}, '([a-z0-9])([A-Z])', '$1_$2'));
  end
  configured = ~isempty(s.Config) || ~isempty(s.ConfigSchema);
  if isempty(s.Default)
    s.Default = ~configured;
  end
  s.Default = logical(s.Default);
end

function s = declared(cls)
% The class's own Provider constant, or [].
  s = [];
  mc = meta.class.fromName(cls);
  if isempty(mc)
    return
  end
  props = mc.PropertyList;
  for i = 1:numel(props)
    p = guatiao.internal.item(props, i);
    if strcmp(p.Name, 'Provider') && p.Constant
      s = eval([cls '.Provider']);
      return
    end
  end
end

function found = kinds_of(cls)
% The kind classes a class inherits, nearest first, each once.
  found = {};
  queue = {cls};
  seen = {};
  while ~isempty(queue)
    c = queue{1};
    queue(1) = [];
    mc = meta.class.fromName(c);
    supers = mc.SuperclassList;
    for i = 1:numel(supers)
      super = guatiao.internal.item(supers, i);
      name = super.Name;
      if any(strcmp(seen, name))
        continue
      end
      seen{end + 1} = name; %#ok<AGROW>
      if guatiao.internal.kinds('is_kind', name)
        found{end + 1} = name; %#ok<AGROW>
      else
        queue{end + 1} = name; %#ok<AGROW>
      end
    end
  end
end
