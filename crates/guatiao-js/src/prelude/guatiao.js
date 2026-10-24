// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// `import ... from 'guatiao'`: the errors a provider throws, and an exact number.

const core = globalThis[Symbol.for('guatiao.core')];
export const {
  Status,
  GuatiaoError,
  BadValue,
  AllocFailed,
  WrongKind,
  NotFound,
  NullArgument,
  Gone,
  Unsupported,
  Busy,
  Internal,
  errorFor,
  ExactNumber,
} = core;
