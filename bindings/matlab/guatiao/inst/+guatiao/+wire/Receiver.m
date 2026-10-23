% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Receiver < handle
% Keeps each channel's schema and checks every value against it. A frame
% is kind:u8 channel:LEB128 payload (a schema, a value, or the channel's
% end); the transport says where one ends.
%
%   r = guatiao.wire.Receiver();
%   e = r.accept(guatiao.wire.announce(1, schema));   % e.kind 'schema'
%   e = r.accept(guatiao.wire.send(1, struct('greeting', 'hi')));
%   x = e.value.data();                                % the checked value
%   r.close();
%
% accept answers struct(kind, channel, value): kind 'schema', 'value' or
% 'close', channel a uint64, value a guatiao.Value (the schema's copy or
% the checked value) or [] for a close. A refusal raises
% guatiao:BadValue:malformed (bytes that are not a frame),
% guatiao:NotFound:noSchema, guatiao:WrongKind:notASchema or
% guatiao:BadValue:invalid (a value its schema refuses), never quoting
% what arrived; with two outputs it answers event = [] and problem =
% struct(refusal, status, channel, kind, at, key, expected, known,
% message) instead. A refusal changes nothing the receiver keeps.
%
% guatiao.wire.Receiver('Alloc', a): what it answers is built in `a`.

  properties (Hidden, SetAccess = private)
    Id = 0
  end

  methods
    function self = Receiver(varargin)
      alloc = guatiao.internal.option(varargin, 'Alloc', []);
      self.Id = guatiao_mex('receiver_new', guatiao.internal.alloc_id(alloc));
    end

    function [event, problem] = accept(self, frame)
    % Reads one frame (a uint8 vector).
      if self.Id == 0
        error('guatiao:Gone:closed', 'the receiver is closed');
      end
      [kind, channel, id, problem] = guatiao_mex('receiver_accept', self.Id, frame, nargout >= 2);
      if ~isempty(problem)
        event = [];
        return
      end
      value = [];
      if ~isempty(id)
        value = guatiao.Value.adopt(id);
      end
      event = struct('kind', kind, 'channel', channel, 'value', []);
      event.value = value;
    end

    function close(self)
    % Frees the receiver and the schemas it keeps; a second close does
    % nothing.
      if self.Id ~= 0
        guatiao_mex('receiver_free', self.Id);
        self.Id = 0;
      end
    end

    function tf = is_open(self)
      tf = self.Id ~= 0 && guatiao_mex('receiver_is_open', self.Id);
    end

    function delete(self)
      try
        close(self);
      catch
        % The gateway is gone at the end of a session.
      end
    end
  end
end
