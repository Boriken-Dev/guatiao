% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function [x, problem] = parse(form, text)
% Reads one of the text forms: 'date' (a guatiao.Date), 'time'
% (guatiao.TimeOfDay), 'datetime' (guatiao.DateTime), 'timestamp'
% (guatiao.Timestamp, in UTC) or 'span' (guatiao.Span).
%
% Text the grammar refuses raises guatiao:BadValue:unparsed, naming the
% byte offset in the UTF-8 text and what was expected there, never quoting
% it; with two outputs it answers x = [] and problem = struct(form, at,
% expected) instead.
  [f, problem] = guatiao_mex('time_parse', form, text, false, nargout >= 2);
  if isempty(f)
    x = [];
    return
  end
  problem = [];
  switch form
    case 'date'
      x = guatiao.Date(f(1), f(2), f(3));
    case 'time'
      x = guatiao.TimeOfDay(f(1), f(2), f(3), f(4));
    case 'datetime'
      x = guatiao.DateTime(f(1), f(2), f(3), f(4), f(5), f(6), f(7));
    case 'timestamp'
      x = guatiao.Timestamp(f(1), f(2));
    otherwise
      x = guatiao.Span(f);
  end
end
