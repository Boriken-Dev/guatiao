% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

classdef Wide < guatiao.Kind
% wide_vtable of include/greeter_kind.h: answers the 64-bit integer it is
% given, unchanged. Pass int64 and uint64 values: a double holds a whole
% number exactly only to 2^53. An answer within 2^53 comes back as a
% double, one past it as int64 or uint64.

  properties (Constant, Hidden)
    WideKind = guatiao.Kind.describe('wide', 3673689684, ...
      guatiao.slot('echo_i64', {'x', 'int64'}, 'Returns', 'int64', 'Required', true, 'Fallible', true), ...
      guatiao.slot('echo_u64', {'x', 'uint64'}, 'Returns', 'uint64', 'Required', true, 'Fallible', true))
  end

  methods
    function out = echo_i64(self, x)
    % `x`, as it arrived.
      out = self.call('echo_i64', x);
    end

    function out = echo_u64(self, x)
    % `x`, as it arrived.
      out = self.call('echo_u64', x);
    end
  end
end
