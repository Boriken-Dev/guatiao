% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function y = add_span(x, span)
% A guatiao.Date or guatiao.DateTime moved by a guatiao.Span (text is read
% as one): years and months together, clamped to the month's end once,
% then weeks and days, then the time units (refused on a date).
  if ischar(span)
    span = guatiao.Span(span);
  end
  y = x.add(span);
end
