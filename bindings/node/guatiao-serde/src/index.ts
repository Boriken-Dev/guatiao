// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A guatiao value as JSON, TOML or YAML, over the `guatiao_serde` library,
 * which is found and loaded on the first call.
 *
 * @module
 */

import { statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import * as n from 'guatiao/ffi';
import { BadValue, MissingSymbol, SIZE, Status, Tag, VALUE, type Value, adopt, raiseForStatus, withNodes } from 'guatiao/ffi';

/** `GUATIAO_FORMAT_*`: which format a parse error names. */
const FORMAT_NAMES: Readonly<Record<number, string>> = { 0: 'json', 1: 'toml', 2: 'yaml' };

/** `guatiao_parse_error`: three `uint32_t`, then an owned string value. */
const PARSE_ERROR = { format: 0, line: 4, column: 8, message: 16, size: 56 } as const;

/**
 * A document that does not parse, and where it stopped. `format` is
 * `'json'`, `'toml'` or `'yaml'`; `line` and `column` count from 1 (the
 * column in characters) and are 0 when the parser did not say where;
 * `reason` says what it expected, without quoting the document.
 */
export class ParseError extends BadValue {
  readonly format: string;
  readonly line: number;
  readonly column: number;
  readonly reason: string;

  constructor(reason: string, where: { format: string; line: number; column: number }) {
    super(Status.ERR_BAD_VALUE, where.line ? `${reason} at line ${where.line}, column ${where.column}` : reason);
    this.reason = reason;
    this.format = where.format;
    this.line = where.line;
    this.column = where.column;
  }
}

const EXPORTS: readonly n.Declared[] = [
  ['uint32_t guatiao_alloc_default(void *out)'],
  ['uint32_t guatiao_json_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)'],
  ['uint32_t guatiao_json_emit(void *value, uint32_t how, void *alloc, void *out)'],
  ['uint32_t guatiao_json_emit_pretty(void *value, uint32_t how, void *alloc, void *out)'],
  ['uint32_t guatiao_toml_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)', 'toml'],
  ['uint32_t guatiao_toml_emit(void *value, uint32_t how, void *alloc, void *out)', 'toml'],
  ['uint32_t guatiao_yaml_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)', 'yaml'],
  ['uint32_t guatiao_yaml_emit(void *value, uint32_t how, void *alloc, void *out)', 'yaml'],
];

let chosen: string | undefined;
let opened: n.Surface | undefined;

function isFile(p: string): boolean {
  try {
    return statSync(p).isFile();
  } catch {
    return false;
  }
}

/**
 * Names the `guatiao_serde` library to load instead of looking for one: a
 * file exporting its functions, whatever it is called (an application's
 * own library carrying several surfaces, say), or a directory holding
 * `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`.
 * Call it before the first call; naming another file once one is loaded
 * throws, naming the same one does nothing.
 */
export function useLibrary(where: string | URL): void {
  const named = where instanceof URL ? fileURLToPath(where) : where;
  if (opened !== undefined) {
    const same = isFile(named) ? named : path.join(named, n.libraryFilename('guatiao_serde'));
    if (path.resolve(same) !== path.resolve(opened.path)) {
      throw new Error(`guatiao_serde is already loaded from ${opened.path}`);
    }
    return;
  }
  chosen = named;
}

/**
 * The `guatiao_serde` library, found and loaded on first use: what
 * `useLibrary` named, else `GUATIAO_SERDE_LIBRARY`, else `GUATIAO_LIBRARY`
 * (a directory holding it, or a file exporting its functions or sitting
 * beside it), else the platform's search. Throws `LibraryNotFound` naming
 * every place tried.
 */
export function library(): n.Surface {
  return (opened ??= n.openSurface('guatiao_serde', EXPORTS, {
    chosen,
    variable: 'GUATIAO_SERDE_LIBRARY',
    probe: 'guatiao_json_parse_ex',
  }));
}

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
  const lib = library();
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
  const alloc = options.alloc ?? library().alloc;
  const out = n.alloc(SIZE.value);
  const err = n.alloc(PARSE_ERROR.size);
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
  const alloc = library().alloc;
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
