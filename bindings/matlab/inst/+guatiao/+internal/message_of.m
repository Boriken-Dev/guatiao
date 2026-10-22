% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function m = message_of(ex)
% The words of a trapped error, for the gateway.
  m = ex.message;
end
