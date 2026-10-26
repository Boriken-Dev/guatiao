// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What `guatiao-kind` writes for a document of kinds: a JavaScript module
 * that loads them, and its `.d.ts`, which declares each kind as an
 * interface (what a provider implements, its appended slots optional), a
 * `<Kind>Remote` interface (what a caller holds), and a value of the
 * kind's own name, so `implements Greeter` and `static kinds = [Greeter]`
 * name the same import.
 *
 * @module
 */

import { type KindDescription, type KindsDocument, type TypeDescription, methodName } from './kinds.js';

/** The module and its declarations, as text. */
export interface Generated {
  readonly js: string;
  readonly dts: string;
}

export interface GenerateOptions {
  /** Where the document is, as the header comment names it. */
  readonly source?: string;
  /** The command that writes these files again, for the header comment. */
  readonly command?: string;
  /** The module specifier of the guatiao package (default `guatiao`). */
  readonly from?: string;
  /**
   * Object kinds a slot names that another document describes, each with
   * the module `guatiao-kind` wrote for that document: `{ listener:
   * './listener_kind.js' }`. The module loads them as known kinds and the
   * declarations import their types.
   */
  readonly imports?: Readonly<Record<string, string>>;
}

/** A kind's name as a TypeScript type name: `session_backend` is `SessionBackend`. */
export function typeName(kindName: string): string {
  const name = kindName
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((part) => part[0]!.toUpperCase() + part.slice(1))
    .join('');
  if (!name) throw new TypeError(`the kind ${JSON.stringify(kindName)} has no letters to name a type with`);
  return /^[0-9]/.test(name) ? `Kind${name}` : name;
}

const RESERVED = new Set(
  (
    'break case catch class const continue debugger default delete do else enum export extends false finally for ' +
    'function if import in instanceof new null return super switch this throw true try typeof var void while with ' +
    'yield let static implements interface package private protected public await arguments eval'
  ).split(' '),
);

function argName(name: string | undefined, i: number): string {
  const camel = methodName(name ?? `arg${i}`);
  const plain = /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(camel) ? camel : `arg${i}`;
  return RESERVED.has(plain) ? `${plain}_` : plain;
}

const NARROW = new Set(['i8', 'i16', 'i32', 'u8', 'u16', 'u32', 'f32', 'f64']);
const WIDE = new Set(['i64', 'u64', 'isize', 'usize']);

/** How each side spells one argument or answer in TypeScript. */
interface Spelled {
  /** What a caller passes, or gets back. */
  readonly caller: string;
  /** What a method receives, or returns. */
  readonly method: string;
}

function spellArg(arg: TypeDescription, names: ReadonlyMap<string, string>, where: string): Spelled {
  const type = arg.type;
  if (type === 'bool') return { caller: 'boolean', method: 'boolean' };
  if (NARROW.has(type)) return { caller: 'number', method: 'number' };
  if (WIDE.has(type)) return { caller: 'number | bigint', method: 'bigint' };
  switch (type) {
    case 'str':
      return { caller: 'string', method: 'string' };
    case 'bytes':
      return { caller: 'Uint8Array | ArrayBuffer | ArrayBufferView', method: 'Uint8Array' };
    case 'bytes_mut':
      return { caller: 'Uint8Array', method: 'Uint8Array' };
    case 'value':
    case 'map':
    case 'list': {
      const pass = arg.pass ?? 'borrowed';
      const plain = type === 'map' ? 'g.Ref | k.MapAnswer' : type === 'list' ? 'g.Ref | k.ListAnswer' : 'unknown';
      const view = type === 'map' ? 'g.MapRef' : type === 'list' ? 'g.ListRef' : 'g.Ref';
      if (pass === 'taken') return { caller: plain, method: 'g.Value' };
      if (pass === 'lent') return { caller: 'g.Ref', method: view };
      if (type === 'value') return { caller: 'unknown', method: arg.nullable ? 'g.Ref | null' : 'g.Ref' };
      if (type === 'map') return { caller: plain, method: view };
      throw new TypeError(`${where}: a list is passed taken or lent`);
    }
    case 'object': {
      const kind = objectName(arg, names, where);
      const given = `${kind} | k.ObjectHandle<typeof ${kind}>`;
      if (arg.pass === 'taken') return { caller: given, method: `k.ObjectHandle<typeof ${kind}>` };
      if (arg.pass === 'lent') return { caller: given, method: `${kind}Remote` };
      throw new TypeError(`${where}: an object is passed taken or lent`);
    }
    default:
      throw new TypeError(`${where}: ${type} is not a type an argument can have`);
  }
}

function spellAnswer(answer: TypeDescription | undefined, names: ReadonlyMap<string, string>, where: string): Spelled {
  if (answer === undefined) return { caller: 'void', method: 'void' };
  const type = answer.type;
  if (type === 'bool') return { caller: 'boolean', method: 'boolean' };
  if (NARROW.has(type)) return { caller: 'number', method: 'number' };
  if (WIDE.has(type)) return { caller: 'bigint', method: 'number | bigint' };
  switch (type) {
    case 'text':
      return { caller: 'string', method: 'string' };
    case 'value':
      return { caller: 'g.Value', method: 'unknown' };
    case 'map':
      return { caller: 'g.Value', method: 'k.MapAnswer' };
    case 'list':
      return { caller: 'g.Value', method: 'k.ListAnswer' };
    case 'object': {
      const kind = objectName(answer, names, where);
      return { caller: `k.ObjectHandle<typeof ${kind}>`, method: `${kind} | k.ObjectHandle<typeof ${kind}>` };
    }
    default:
      throw new TypeError(`${where}: an answer cannot be ${type}`);
  }
}

function objectName(given: TypeDescription, names: ReadonlyMap<string, string>, where: string): string {
  const found = given.kind === undefined ? undefined : names.get(given.kind);
  if (found === undefined) throw new TypeError(`${where}: the object kind ${String(given.kind)} is not in this document`);
  return found;
}

/** A doc comment over `text`, indented by `indent`; nothing for no text. */
function docComment(text: string | undefined, indent: string, extra: readonly string[] = []): string[] {
  const lines = [...(text ? text.split('\n') : []), ...(text && extra.length ? [''] : []), ...extra].map((line) =>
    line.replace(/\*\//g, '*\\/'),
  );
  if (!lines.length) return [];
  if (lines.length === 1) return [`${indent}/** ${lines[0]} */`];
  return [`${indent}/**`, ...lines.map((line) => (line ? `${indent} * ${line}` : `${indent} *`)), `${indent} */`];
}

function checked(document: KindsDocument): KindsDocument {
  if (document === null || typeof document !== 'object' || document.format !== 1 || !Array.isArray(document.kinds)) {
    throw new TypeError('a kinds document is { format: 1, kinds: [...] }');
  }
  return document;
}

/** Each kind's description, without its doc text: what the module loads. */
function withoutDocs(document: KindsDocument): KindsDocument {
  return {
    format: 1,
    kinds: document.kinds.map(({ doc: _doc, slots, ...kind }) => ({
      ...kind,
      slots: slots.map(({ doc: _slotDoc, ...slot }) => slot),
    })),
  };
}

function declareKind(kind: KindDescription, names: ReadonlyMap<string, string>): string[] {
  const name = names.get(kind.name)!;
  const impl: string[] = [];
  const remote: string[] = [];
  for (const slot of kind.slots) {
    const where = `${kind.name}.${slot.name}`;
    const method = methodName(slot.name);
    const args = (slot.args ?? []).map((arg, i) => ({ name: argName(arg.name, i), ...spellArg(arg, names, `${where} argument ${i}`) }));
    const answer = spellAnswer(slot.answer, names, `${where} answer`);
    impl.push(...docComment(slot.doc, '  '));
    impl.push(`  ${method}${slot.required ? '' : '?'}(${args.map((a) => `${a.name}: ${a.method}`).join(', ')}): ${answer.method};`);
    const lacking = slot.required ? [] : ['Throws `Unsupported` when the table lacks it: `has()` asks first.'];
    remote.push(...docComment(slot.doc, '  ', lacking));
    remote.push(`  ${method}(${args.map((a) => `${a.name}: ${a.caller}`).join(', ')}): ${answer.caller};`);
  }
  return [
    ...docComment(kind.doc, '', [`The \`${kind.name}\` kind: what ${kind.object ? 'an object of it' : 'a provider of it'} implements.`]),
    `export interface ${name} {`,
    ...impl,
    '}',
    '',
    `/** A caller's handle on a \`${kind.name}\` table${kind.object ? ' or object' : ''}: a method per slot. */`,
    `export interface ${name}Remote extends k.RemoteBase<typeof ${name}> {`,
    ...remote,
    '}',
    '',
    ...docComment(kind.doc, ''),
    `export declare const ${name}: k.DescribedKind<${name}, ${name}Remote, ${kind.object ? 'true' : 'false'}>;`,
  ];
}

/** The module and declarations for `document`. */
export function generate(document: KindsDocument, options: GenerateOptions = {}): Generated {
  const given = checked(document);
  const from = options.from ?? 'guatiao';
  const names = new Map<string, string>();
  const imports = new Map<string, string[]>();
  const add = (kind: string): string => {
    const name = typeName(kind);
    if ([...names.values()].includes(name) || name === 'k' || name === 'g') {
      throw new TypeError(`two kinds are ${name} in TypeScript`);
    }
    names.set(kind, name);
    return name;
  };
  for (const kind of given.kinds) add(kind.name);
  for (const [kind, module] of Object.entries(options.imports ?? {})) {
    if (names.has(kind)) throw new TypeError(`${kind} is in this document and imported from ${module}`);
    imports.set(module, [...(imports.get(module) ?? []), add(kind)]);
  }
  const known = [...imports.values()].flat();
  const modules = [...imports.keys()].sort();
  const banner = [
    `// GENERATED by guatiao-kind${options.source ? ` from ${options.source}` : ''}. DO NOT EDIT.`,
    ...(options.command ? [`// Regenerate: ${options.command}`] : []),
  ];

  const js = [
    ...banner,
    '',
    `import { kindsOf } from '${from}/kinds';`,
    ...modules.map((m) => `import { ${imports.get(m)!.join(', ')} } from '${m}';`),
    '',
    `const described = kindsOf(${JSON.stringify(withoutDocs(given), null, 2)}${known.length ? `, { known: [${known.join(', ')}] }` : ''});`,
    '',
    ...given.kinds.flatMap((kind) => [
      ...docComment(kind.doc, ''),
      `export const ${names.get(kind.name)!} = described[${JSON.stringify(kind.name)}];`,
      '',
    ]),
  ];

  const dts = [
    ...banner,
    '',
    `import type * as g from '${from}';`,
    `import type * as k from '${from}/kinds';`,
    ...modules.map((m) => `import type { ${imports.get(m)!.flatMap((n) => [n, `${n}Remote`]).join(', ')} } from '${m}';`),
    '',
    ...given.kinds.flatMap((kind) => [...declareKind(kind, names), '']),
  ];
  return { js: `${js.join('\n').trimEnd()}\n`, dts: `${dts.join('\n').trimEnd()}\n` };
}
