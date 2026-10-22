% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function s = statuses()
% The status codes a guatiao call answers, by name. An error's identifier
% is guatiao:<name>[:<detail>]; a provider's method raises one with
% error('guatiao:BadValue', 'nobody to greet').
  s = struct('OK', 0, 'BadValue', 7, 'AllocFailed', 8, 'WrongKind', 9, 'NotFound', 10, ...
             'NullArgument', 11, 'Gone', 12, 'Unsupported', 13, 'Busy', 14, 'Internal', 100);
end
