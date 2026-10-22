% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Chat < greeterkind.Conversation
% One conversation `start` hands back, holding the listener it was handed.

  properties
    listener = []
    transcript = {}
  end

  methods
    function self = Chat(listener)
      if nargin
        self.listener = listener;
      end
    end

    function say(self, what)
      if isempty(what)
        error('guatiao:BadValue', 'nothing to say');
      end
      self.transcript{end + 1} = what;
      self.listener.heard(what);
    end

    function n = read(self, dst)
      text = unicode2native(strjoin(self.transcript, sprintf('\n')), 'UTF-8');
      n = min(numel(text), numel(dst.Data));
      dst.Data(1:n) = text(1:n);
    end

    function n = turns(self)
      n = numel(self.transcript);
    end

    function close(self)
    % The listener a conversation holds is let go with it.
      if ~isempty(self.listener)
        self.listener.close();
        self.listener = [];
      end
    end
  end
end
