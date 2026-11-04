// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// `import ... from 'guatiao/provide'`: the class a provider extends, and `s`,
// the schema of a configuration it is built from.

const core = globalThis[Symbol.for('guatiao.core')];
export const { Provider, s } = core;
