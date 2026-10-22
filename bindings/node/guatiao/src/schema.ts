// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What a schema says about a value: here, which parts are secrets.
 *
 * @module
 */

import { SIZE } from './abi.js';
import { raiseForStatus } from './errors.js';
import * as n from './native.js';
import { type Value, adopt, withNodes } from './value.js';

/**
 * A copy of `value` with every secret replaced by `placeholder`: every
 * field the schema marks (`x-sensitive` or `writeOnly`), at any depth, and
 * whatever the schema does not describe. Local `$ref`s are followed.
 * `schema` and `value` are `Ref`s or plain data; the copy is a new `Value`
 * the caller closes. `WrongKind` for a schema that is not a map.
 */
export function redact(schema: unknown, value: unknown, placeholder = '[redacted]'): Value {
  return withNodes([schema, value], ([s, v]) => {
    const lib = n.core();
    const out = n.alloc(SIZE.value);
    try {
      raiseForStatus(lib.fn.guatiao_schema_redact!(s, v, n.str(placeholder), lib.alloc, out));
    } catch (e) {
      n.freeNode(out);
      n.release(out);
      throw e;
    }
    return adopt(out, lib.alloc);
  });
}
