// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind written as TypeScript interfaces, read into the description a
// Rust trait writes.

import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterAll, describe, expect, it } from 'vitest';

import { describeInterfaces, floorHashOf } from '../src/describe.js';
import { type KindsDocument, kindsOf } from '../src/kinds.js';
import { REPO, readText } from './helpers.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.join(HERE, '..', 'src');
const DIST = path.join(HERE, '..', 'dist');
const TSCONFIG = path.join(HERE, '..', 'tsconfig.json');
const GREETER_DOCUMENT = path.join(REPO, 'examples', 'greeter_kind', 'include', 'greeter_kind.kinds.json');

const scratch: string[] = [];
afterAll(() => {
  for (const dir of scratch) rmSync(dir, { recursive: true, force: true });
});

/** Describes `source` as a file of its own, in a project that finds guatiao in this checkout's sources. */
function describeText(source: string, ...more: [string, string][]): Promise<KindsDocument> {
  return describeIn({ guatiao: [path.join(SRC, 'index.ts')], 'guatiao/*': [path.join(SRC, '*.ts')] }, source, ...more);
}

/** Describes `source` in a project whose `paths` say where guatiao is. */
async function describeIn(paths: Record<string, string[]>, source: string, ...more: [string, string][]): Promise<KindsDocument> {
  const dir = mkdtempSync(path.join(tmpdir(), 'guatiao-describe-'));
  scratch.push(dir);
  const main = path.join(dir, 'kinds.ts');
  writeFileSync(main, source);
  for (const [name, text] of more) writeFileSync(path.join(dir, name), text);
  writeFileSync(
    path.join(dir, 'tsconfig.json'),
    JSON.stringify({
      compilerOptions: {
        target: 'es2024',
        lib: ['es2024', 'esnext.disposable'],
        module: 'nodenext',
        moduleResolution: 'nodenext',
        types: [],
        strict: true,
        noEmit: true,
        allowImportingTsExtensions: true,
        paths,
      },
      include: ['*.ts'],
    }),
  );
  return describeInterfaces(main);
}

const HEAD = `import type { ListRef, MapRef, Ref } from 'guatiao';
import type * as k from 'guatiao/kinds';
import type { Borrowed, BytesMut, Lent, ListAnswer, MapAnswer, ObjectKind, Taken, ValueAnswer } from 'guatiao/kinds';
import type { f32, f64, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize } from 'guatiao/kinds';
`;

function fnv1a(text: string): number {
  let hash = 0x811c9dc5;
  for (const byte of Buffer.from(text)) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
  return hash;
}

describe('the greeter kinds, as interfaces', () => {
  it('describe themselves as #[guatiao::kind] does', async () => {
    const written = await describeInterfaces(path.join(HERE, 'fixtures', 'greeter_interfaces.ts'), { project: TSCONFIG });
    expect(written).toEqual(JSON.parse(readText(GREETER_DOCUMENT)));
    expect(`${JSON.stringify(written, null, 2)}\n`).toBe(readText(GREETER_DOCUMENT));
  });

});

describe('an interface', () => {
  it('spells every shape a slot has', async () => {
    const document = await describeText(`${HEAD}
type Count = k.u64;
/** What takes text. */
export interface Sink extends ObjectKind {
  put(text: string): void;
}
/** Every shape. */
export interface Shapes {
  scalars(a: i8, b: i16, c: i32, d: i64, e: isize, f: u8, g: u16, h: u32, i: u64, j: usize, l: f32, m: f64, n: boolean): void;
  bytes(a: Uint8Array, b: BytesMut): u32;
  trees(a: Ref, b: Ref | null, c: MapRef, d: Borrowed<MapRef>): void;
  taken(a: Taken<Ref>, b: Taken<MapRef>, c: Taken<ListRef>): void;
  lent(a: Lent<Ref>, b: Lent<MapRef>, c: Lent<ListRef>): void;
  objects(a: Taken<Sink>, b: Lent<Sink>): Sink;
  aliased(a: Count, b: k.i8): k.u8;
  text(): string;
  value(): ValueAnswer;
  map(): MapAnswer;
  list(): ListAnswer;
  fails?(): f64;
}
`);
    const a = (name: string, type: string, rest: object = {}): object => ({ name, type, ...rest });
    const tree = (name: string, type: string, pass: string, rest: object = {}): object => ({ name, type, pass, ...rest });
    const slotOf = (name: string, args: object[], answer?: string, rest: object = {}): object => ({
      name,
      required: true,
      fallible: false,
      args,
      ...(answer === undefined ? {} : { answer: { type: answer } }),
      ...rest,
    });
    const scalars = ['i8', 'i16', 'i32', 'i64', 'isize', 'u8', 'u16', 'u32', 'u64', 'usize', 'f32', 'f64', 'bool'];
    const shapes = document.kinds.find((kind) => kind.name === 'shapes');
    expect(shapes?.slots).toEqual([
      slotOf('scalars', scalars.map((type, i) => a('abcdefghijlmn'[i]!, type))),
      slotOf('bytes', [a('a', 'bytes'), a('b', 'bytes_mut')], 'u32'),
      slotOf('trees', [
        tree('a', 'value', 'borrowed'),
        tree('b', 'value', 'borrowed', { nullable: true }),
        tree('c', 'map', 'borrowed'),
        tree('d', 'map', 'borrowed'),
      ]),
      slotOf('taken', [tree('a', 'value', 'taken'), tree('b', 'map', 'taken'), tree('c', 'list', 'taken')]),
      slotOf('lent', [tree('a', 'value', 'lent'), tree('b', 'map', 'lent'), tree('c', 'list', 'lent')]),
      {
        ...slotOf('objects', [
          { name: 'a', type: 'object', kind: 'sink', pass: 'taken' },
          { name: 'b', type: 'object', kind: 'sink', pass: 'lent' },
        ]),
        answer: { type: 'object', kind: 'sink' },
      },
      slotOf('aliased', [a('a', 'u64'), a('b', 'i8')], 'u8'),
      slotOf('text', [], 'text'),
      slotOf('value', [], 'value'),
      slotOf('map', [], 'map'),
      slotOf('list', [], 'list'),
      slotOf('fails', [], 'f64', { required: false }),
    ]);
    expect(document.kinds.map((kind) => kind.name)).toEqual(['sink', 'shapes']);
    expect(shapes?.doc).toBe('Every shape.');
    // What it describes is what the runtime loads.
    expect(Object.keys(kindsOf(document))).toEqual(['sink', 'shapes']);
  });

  it('reads the types from what the package ships, as a project that installs it does', async (ctx) => {
    if (!existsSync(path.join(DIST, 'kinds.d.ts'))) return ctx.skip('the package is not built: npm run build');
    // The declarations in dist/ must keep the tags the types are found by.
    const document = await describeIn(
      { guatiao: [path.join(DIST, 'index.d.ts')], 'guatiao/*': [path.join(DIST, '*.d.ts')] },
      `import type { ListRef, MapRef, Ref } from 'guatiao';
import type { BytesMut, Lent, MapAnswer, ObjectKind, Taken, i64, u32 } from 'guatiao/kinds';
export interface Sink extends ObjectKind {
  put(data: BytesMut): u32;
}
export interface Shipped {
  every(a: Ref | null, b: MapRef, c: Taken<ListRef>, d: Lent<Sink>, e: i64): MapAnswer;
}
`,
    );
    expect(document.kinds.map((kind) => kind.name)).toEqual(['sink', 'shipped']);
    expect(document.kinds[1]?.slots[0]?.args).toEqual([
      { name: 'a', type: 'value', pass: 'borrowed', nullable: true },
      { name: 'b', type: 'map', pass: 'borrowed' },
      { name: 'c', type: 'list', pass: 'taken' },
      { name: 'd', type: 'object', kind: 'sink', pass: 'lent' },
      { name: 'e', type: 'i64' },
    ]);
    expect(document.kinds[0]?.slots[0]).toEqual({
      name: 'put',
      required: true,
      fallible: false,
      args: [{ name: 'data', type: 'bytes_mut' }],
      answer: { type: 'u32' },
    });
  });

  it('is named after its interface, and says its hash when it must', async () => {
    const document = await describeText(`${HEAD}
/**
 * Kept apart.
 *
 * @kind store.v2
 * @floorHash 4242
 */
export interface KeyStore {
  /** @throws */
  open(path: string): void;
  readAll?(): ListAnswer;
}
export interface Other {
  ping(): void;
}
`);
    expect(document.kinds.map((kind) => [kind.name, kind.floor_hash, kind.doc])).toEqual([
      ['store.v2', 4242, 'Kept apart.'],
      ['other', fnv1a('provider;ping()->()'), undefined],
    ]);
  });

  it('gets the hash Rust gives the same required slots', () => {
    const slot = (name: string, args: object[], answer: object | undefined, fallible: boolean): never =>
      ({ name, required: true, fallible, args, ...(answer === undefined ? {} : { answer }) }) as never;
    expect(floorHashOf(false, [slot('greet', [{ type: 'str' }], { type: 'map' }, true)])).toBe(2695856491);
    expect(
      floorHashOf(false, [
        slot('f', [{ type: 'value', pass: 'borrowed' }, { type: 'map', pass: 'borrowed' }], { type: 'i64' }, false),
      ]),
    ).toBe(fnv1a('provider;f(&Value,&Map)->i64'));
    expect(
      floorHashOf(true, [
        slot('read', [{ type: 'bytes_mut' }, { type: 'object', kind: 'sink_x', pass: 'lent' }], { type: 'text' }, true),
      ]),
    ).toBe(fnv1a('object;read(&mut[u8],&mutdynSinkX)->Result<String,ProviderError>'));
    expect(floorHashOf(false, [])).toBe(fnv1a('provider'));
  });

  it('reaches the object kinds its slots name, in another file too', async () => {
    const document = await describeText(
      `${HEAD}
import type { Far } from './far.ts';
export interface Near {
  use(far: Lent<Far>): Far;
}
`,
      ['far.ts', `import type { ObjectKind } from 'guatiao/kinds';\nexport interface Far extends ObjectKind {\n  go(): void;\n}\n`],
    );
    expect(document.kinds.map((kind) => [kind.name, kind.object])).toEqual([
      ['near', false],
      ['far', true],
    ]);
  });

  it('is refused where it says something a table cannot', async () => {
    const refusal = (body: string, why: RegExp, more: [string, string][] = []): Promise<void> =>
      expect(describeText(`${HEAD}\n${body}`, ...more)).rejects.toThrow(why);
    await Promise.all([
      refusal('export interface A { f(x: number): void }', /A\.f argument 0: number says nothing of the C type/),
      refusal('export interface A { f(): bigint }', /A\.f answer: bigint says nothing/),
      refusal('export interface A { f(x: string | number): void }', /a union is not a type a slot has/),
      refusal('export interface A { f(x: MapRef | null): void }', /only a borrowed value \(`Ref \| null`\) may be null/),
      refusal('export interface L extends ObjectKind { g(): void }\nexport interface A { f(x: L): void }', /an object is passed as Taken<L> or Lent<L>/),
      refusal('export interface N { g(): void }\nexport interface A { f(x: Lent<N>): void }', /N is not a tree \(Ref, MapRef, ListRef\) or an object kind/),
      refusal('export interface A { f(x: ListRef): void }', /a list is passed Taken or Lent/),
      refusal('export interface A { f(): MapRef }', /a slot answers MapAnswer, ListAnswer or ValueAnswer/),
      refusal('export interface A { f(): Uint8Array }', /not bytes/),
      refusal('export interface A { f(x: MapAnswer): void }', /is what a slot answers/),
      refusal('export interface A { f(x: ObjectKind): void }', /ObjectKind is what an interface extends/),
      refusal('export interface A { a?(): void; b(): void }', /b is required and follows a, which is not/),
      refusal('export interface A { x: string }', /a kind's slots are methods/),
      refusal('export interface A { f(x?: string): void }', /argument 0 is a plain, required parameter/),
      refusal('export interface A { parseHTTP(): void }', /write the method as parseHttp/),
      refusal('export interface A<T> { f(): void }', /a kind has no type parameters/),
      refusal('export interface A { f<T>(): void }', /a slot has no type parameters/),
      refusal('export interface A { f(): void; f(x: string): void }', /A: two slots are f/),
      refusal('interface Base { f(): void }\nexport interface A extends Base { g(): void }', /a kind extends only ObjectKind/),
      refusal('export interface A extends ObjectKind { destroy(): void }', /destroy before its slots/),
      refusal('export interface A { f(): void }\n/** @kind a */\nexport interface B { g(): void }', /two interfaces are the kind a/),
      refusal('/** @floorHash soon */\nexport interface A { f(): void }', /@floorHash is a 32-bit unsigned integer/),
      refusal('export interface A { f(x: Missing): void }', /Missing is not found/),
      refusal('export interface A { f(x: Date): void }', /Date is not a type a slot has; an interface is one only as an object kind/),
    ]);
    await expect(describeText('export type NotAKind = string;\n')).rejects.toThrow(/exports no interface/);
    await expect(describeText(`${HEAD}\ninterface Hidden { f(): void }\n`)).rejects.toThrow(/exports no interface/);
  });
});
