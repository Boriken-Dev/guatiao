// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A value as JSON, TOML or YAML, over `guatiao_serde`, which is loaded on
 * the first call.
 *
 * @module
 */

import { FORMAT_NAMES, PARSE_ERROR, SIZE, Status, Tag, VALUE } from './abi.js';
import { MissingSymbol, ParseError, raiseForStatus } from './errors.js';
import * as n from './native.js';
import { type Value, adopt, withNodes } from './value.js';

export type Format = 'json' | 'toml' | 'yaml';

/** How a byte string is spelled where a format has none: the low byte of `how`. */
export const BYTES_DATA_URI = 0;
export const BYTES_BASE64 = 1;
export const BYTES_ARRAY = 2;
export const BYTES_REFUSE = 3;
/** Turns a `data:;base64,...` string back into bytes when reading. */
export const READ_DATA_URIS = 1 << 8;
/** Indents the output for a person to read. */
export const PRETTY = 1 << 9;

const FORMATS: Readonly<Record<Format, { parse: string; emit: string; pretty?: string; feature?: string }>> = {
  json: { parse: 'guatiao_json_parse_ex', emit: 'guatiao_json_emit', pretty: 'guatiao_json_emit_pretty' },
  toml: { parse: 'guatiao_toml_parse_ex', emit: 'guatiao_toml_emit', feature: 'toml' },
  yaml: { parse: 'guatiao_yaml_parse_ex', emit: 'guatiao_yaml_emit', feature: 'yaml' },
};

function formatOf(format: Format): (typeof FORMATS)[Format] {
  const found = FORMATS[format];
  if (found === undefined) throw new TypeError(`unknown format: ${String(format)}`);
  return found;
}

function resolved(name: string, feature: string | undefined): (...args: unknown[]) => number {
  const lib = n.serde();
  if (lib.missing.has(name)) throw new MissingSymbol(lib.path, name, feature);
  return lib.fn[name]!;
}

export interface LoadOptions {
  format?: Format;
  /** `READ_DATA_URIS` and the bytes flags. */
  how?: number;
  /** The address of a `guatiao_alloc` to build in. */
  alloc?: bigint;
}

/**
 * Parses `text` into a new `Value`, which the caller closes. A document
 * that does not parse throws `ParseError` (a `BadValue`) with its
 * `format`, `line` and `column`; a TOML document must be a map
 * (`WrongKind` otherwise). A number keeps its exact text.
 */
export function loads(text: string, options: LoadOptions = {}): Value {
  const format = options.format ?? 'json';
  const spec = formatOf(format);
  const parse = resolved(spec.parse, spec.feature);
  const alloc = options.alloc ?? n.serde().alloc;
  const out = n.alloc(SIZE.value);
  const err = n.alloc(SIZE.parseError);
  try {
    const status = parse(n.str(text), options.how ?? 0, alloc, out, err);
    const message = takeMessage(err);
    if (status === Status.ERR_BAD_VALUE) {
      throw new ParseError(message, {
        format: FORMAT_NAMES[n.u32(err, PARSE_ERROR.format)] ?? format,
        line: n.u32(err, PARSE_ERROR.line),
        column: n.u32(err, PARSE_ERROR.column),
      });
    }
    raiseForStatus(status, message);
  } catch (e) {
    n.freeNode(out);
    n.release(out);
    throw e;
  } finally {
    n.release(err);
  }
  return adopt(out, alloc);
}

/** The parse error's message, its string value freed. */
function takeMessage(err: bigint): string {
  const node = err + BigInt(PARSE_ERROR.message);
  try {
    if (n.u32(node, VALUE.tag) !== Tag.STRING) return '';
    return n.stringText(node + BigInt(VALUE.payload));
  } finally {
    n.freeNode(node);
  }
}

export interface DumpOptions {
  format?: Format;
  /** Indented for a person to read. */
  pretty?: boolean;
  /** The bytes flags. */
  how?: number;
}

/**
 * `value` (a `Ref`, or plain data) written as `format`. A TOML document
 * must be a map.
 */
export function dumps(value: unknown, options: DumpOptions = {}): string {
  const spec = formatOf(options.format ?? 'json');
  let how = options.how ?? 0;
  let name = spec.emit;
  if (options.pretty) {
    if (spec.pretty !== undefined) name = spec.pretty;
    else how |= PRETTY;
  }
  const emit = resolved(name, spec.feature);
  const alloc = n.serde().alloc;
  return withNodes([value], ([node]) => {
    const out = n.alloc(SIZE.value);
    try {
      raiseForStatus(emit(node, how, alloc, out));
      return n.u32(out, VALUE.tag) === Tag.STRING ? n.stringText(out + BigInt(VALUE.payload)) : '';
    } finally {
      n.freeNode(out);
      n.release(out);
    }
  });
}
