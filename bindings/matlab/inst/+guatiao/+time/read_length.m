% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function d = read_length(x)
% Reads an exact length in any of its three spellings ([seconds,
% nanoseconds], a number of seconds, {secs, nanos}) from a guatiao value, a
% view or MATLAB data: a guatiao.Duration.
  r = guatiao_mex('duration_read', x);
  s = str2double(r{1});
  if s > 9007199254740992
    error('guatiao:BadValue', 'a length past 2^53 seconds is not held exactly by a double');
  end
  d = guatiao.Duration(s, r{2});
end
