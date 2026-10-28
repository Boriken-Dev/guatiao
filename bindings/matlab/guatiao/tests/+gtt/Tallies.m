% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Tallies < tallykind.Tally
% A tally served from M, its kind class written from the tally kinds'
% description: every slot, required and optional.
  properties
    Names = {}
    Counts = []
  end

  methods
    function out = add(self, name, by)
      if isempty(name)
        error('guatiao:BadValue', 'nothing to count');
      end
      out = self.bump(name, double(by));
    end

    function out = total(self, name)
      hit = find(strcmp(self.Names, name), 1);
      out = int64(0);
      if ~isempty(hit)
        out = int64(self.Counts(hit));
      end
    end

    function out = all(self)
      out = struct();
      for i = 1:numel(self.Names)
        out.(self.Names{i}) = int64(self.Counts(i));
      end
    end

    function out = known(self, name)
      out = any(strcmp(self.Names, name));
    end

    function out = checksum(self, data)
      out = uint32(mod(sum(double(data)), 2^32));
    end

    function report(self, watcher)
      [names, order] = sort(self.Names);
      for i = 1:numel(names)
        watcher.changed(names{i}, int64(self.Counts(order(i))));
      end
    end

    function out = load(self, counts)
      more = counts.data();
      counts.close();
      names = fieldnames(more);
      for i = 1:numel(names)
        self.bump(names{i}, double(more.(names{i})));
      end
      out = uint32(numel(names));
    end
  end

  methods (Access = private)
    function out = bump(self, name, by)
      hit = find(strcmp(self.Names, name), 1);
      if isempty(hit)
        self.Names{end + 1} = name;
        self.Counts(end + 1) = 0;
        hit = numel(self.Names);
      end
      self.Counts(hit) = self.Counts(hit) + by;
      out = int64(self.Counts(hit));
    end
  end
end
