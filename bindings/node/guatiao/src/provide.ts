// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A library written in JavaScript or TypeScript: provider classes, and
 * the descriptor any host reads, built in memory kept for the process.
 *
 * ```ts
 * // greeters.ts, in a package named node_greeter
 * import { BadValue } from 'guatiao';
 * import { Provider, s } from 'guatiao/provide';
 * import { Counter, Greeter } from 'greeter_kind';
 *
 * export class Hello extends Provider implements Greeter, Counter {
 *   static override readonly kinds = [Greeter, Counter];
 *   static override readonly displayName = 'Hello, in Node';
 *   #n = 0n;
 *   greet(name: string) {
 *     if (!name) throw new BadValue('nobody to greet');
 *     return { greeting: `hello, ${name}` };
 *   }
 *   count() {
 *     return ++this.#n;
 *   }
 * }
 * ```
 *
 * `await libraryOf(new URL('./greeters.js', import.meta.url))` is the
 * library that module is, and `registry.registerLibrary(it)` files it.
 *
 * @module
 */

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { ABI_VERSION, KIND_TABLE, LIBRARY_INFO, PROVIDER_INFO, SIZE, Status, Tag, VALUE, VIEW } from './abi.js';
import * as s from './config.js';
import { NullArgument } from './errors.js';
import { type KindImpl, type ProviderKindLike, asKind, hold, inCall, refuseInsideACall, release, statusOf, tableFor, writeErr } from './kinds.js';
import * as n from './native.js';
import { Borrowed, Ref, Value } from './value.js';

export { s };

/**
 * The base of a provider class. It serves the kinds `static kinds` lists,
 * through the methods it defines; `implements` each kind's type so
 * `tsc --strict` checks them.
 *
 * Static fields say the rest: `named` (default: the class name in
 * snake_case), `id` (default `{library id}_{named}`), `displayName`,
 * `version` (empty: the library's), `meta` (plain data), `config` (an `s`
 * schema: an instance per configuration, its constructor receiving the
 * built object), `configSchema` (a raw JSON Schema: the constructor
 * receives plain data), `default` (also offer `new cls()`; the default
 * when there is no configuration). `static available()` answers why the
 * provider cannot run here, or nothing when it can; it is asked every
 * time a host asks.
 */
export abstract class Provider {
  /** The provider kinds this class serves, which it `implements`. */
  static readonly kinds: readonly ProviderKindLike[] = [];
  static readonly named?: string;
  static readonly id?: string;
  static readonly displayName?: string;
  static readonly version?: string;
  static readonly meta?: { readonly [key: string]: unknown };
  static readonly default?: boolean;
  static readonly config?: s.Schema<unknown>;
  static readonly configSchema?: { readonly [key: string]: unknown };

  /** `undefined` (or `null`) when this provider can run here, else why not. */
  static available(): string | null | undefined {
    return undefined;
  }
}

/** A class extending `Provider`. */
export type ProviderClass = (abstract new (...args: any[]) => Provider) & {
  readonly kinds: readonly ProviderKindLike[];
  readonly named?: string;
  readonly id?: string;
  readonly displayName?: string;
  readonly version?: string;
  readonly meta?: { readonly [key: string]: unknown };
  readonly default?: boolean;
  readonly config?: s.Schema<unknown>;
  readonly configSchema?: { readonly [key: string]: unknown };
  available(): string | null | undefined;
};

interface Spec {
  readonly name: string;
  readonly id: string | undefined;
  readonly displayName: string;
  readonly version: string;
  readonly meta: { readonly [key: string]: unknown } | undefined;
  readonly kinds: readonly KindImpl[];
  readonly schema: { readonly [key: string]: unknown } | undefined;
  readonly build: ((data: unknown) => unknown) | undefined;
  readonly buildsInstances: boolean;
  readonly default: boolean;
  readonly asksAvailable: boolean;
}

function snake(name: string): string {
  return name.replace(/(?<=[a-z0-9])([A-Z])/g, '_$1').toLowerCase();
}

function isProviderClass(value: unknown): value is ProviderClass {
  return typeof value === 'function' && value !== Provider && value.prototype instanceof Provider;
}

const specs = new WeakMap<ProviderClass, Spec>();

/** What a provider class says, checked: `TypeError` naming what is wrong. */
function specOf(cls: ProviderClass): Spec {
  const found = specs.get(cls);
  if (found !== undefined) return found;
  if (!isProviderClass(cls)) throw new TypeError(`${String(cls)} is not a class extending Provider`);
  const kinds = (Array.isArray(cls.kinds) ? cls.kinds : []).map((k) => asKind(k));
  if (!kinds.length) throw new TypeError(`${cls.name} lists no kind in static kinds`);
  for (const kind of kinds) {
    if (kind.isObject) throw new TypeError(`${cls.name}: ${kind.name} is an object kind; a provider serves provider kinds`);
    tableFor(kind, cls.prototype as object, cls.name);
  }
  const config = cls.config;
  if (config !== undefined && !s.isSchema(config)) {
    throw new TypeError(`${cls.name}.config is a schema made with s; say a raw JSON Schema with configSchema`);
  }
  const buildsInstances = config !== undefined || cls.configSchema !== undefined;
  const spec: Spec = {
    name: cls.named ?? snake(cls.name),
    id: cls.id,
    displayName: cls.displayName ?? '',
    version: cls.version ?? '',
    meta: cls.meta,
    kinds,
    schema: cls.configSchema ?? config?.json,
    build: config === undefined ? undefined : (data) => config.build(data),
    buildsInstances,
    default: cls.default ?? !buildsInstances,
    asksAvailable: cls.available !== Provider.available,
  };
  specs.set(cls, spec);
  return spec;
}

// ---- the descriptor ------------------------------------------------------------

// Every library built here stays reachable for the process: a host may hold
// its descriptor's function pointers for as long as it runs.
const kept = new Set<Library>();
// A reason a provider gives for being unavailable outlives every reader.
const reasons = new Map<string, { ptr: bigint; len: number }>();

let protos: { entry: unknown; unload: unknown; available: unknown; create: unknown; destroy: unknown } | undefined;

function libraryProtos(): NonNullable<typeof protos> {
  const k = n.koffi();
  protos ??= {
    entry: k.pointer(k.proto('void *', ['void *'])),
    unload: k.pointer(k.proto('uint32_t', [])),
    available: k.pointer(k.proto('bool', ['void *', 'void *'])),
    create: k.pointer(k.proto('uint32_t', ['void *', 'void *', 'void *', 'void *'])),
    destroy: k.pointer(k.proto('void', ['void *', 'void *'])),
  };
  return protos;
}

/**
 * A built library: its descriptor, and everything the descriptor points
 * at, kept for the process. `entry` is a `guatiao_library_entry`
 * (`Registry.registerEntry` takes it); `infoAddress` the address of the
 * `guatiao_library_info` it answers.
 */
export class Library {
  readonly id: string;
  readonly version: string;
  readonly providers: readonly ProviderClass[];
  /** The kinds its providers serve, each once, in order. */
  readonly kinds: readonly string[];
  /** What its descriptor says about it, as plain data. */
  readonly meta: { readonly [key: string]: unknown } | undefined;
  readonly entry: bigint;
  readonly infoAddress: bigint;
  readonly #instances = new Set<bigint>();
  readonly #defaults: bigint[] = [];
  readonly #values: Value[] = [];
  #closed = false;

  constructor(id: string, version: string, providers: readonly ProviderClass[], meta?: { readonly [key: string]: unknown }) {
    if (typeof id !== 'string' || !id) throw new TypeError('a library has an id');
    if (typeof version !== 'string') throw new TypeError(`${id}'s version is text`);
    // koffi cannot register the descriptor's callbacks from inside a callback.
    refuseInsideACall(`the library ${id}`);
    const resolved = providers.map((cls) => specOf(cls));
    const ids = resolved.map((spec) => spec.id ?? `${id}_${spec.name}`);
    if (new Set(ids).size !== ids.length) throw new TypeError(`two providers of ${id} share an id: ${ids.join(', ')}`);
    this.id = id;
    this.version = version;
    this.providers = Object.freeze([...providers]);
    this.kinds = Object.freeze([...new Set(resolved.flatMap((spec) => spec.kinds.map((k) => k.name)))]);
    this.meta = meta === undefined ? undefined : Object.freeze({ ...meta });

    const k = n.koffi();
    const p = libraryProtos();
    const infos = n.alloc(SIZE.providerInfo * Math.max(1, providers.length));
    providers.forEach((cls, i) => this.#describe(infos + BigInt(i * SIZE.providerInfo), cls, resolved[i]!, ids[i]!));

    const info = n.alloc(SIZE.libraryInfo);
    n.setU32(info, LIBRARY_INFO.structSize, SIZE.libraryInfo);
    n.setU32(info, LIBRARY_INFO.abiVersion, ABI_VERSION);
    writeStr(info + BigInt(LIBRARY_INFO.id), id);
    writeStr(info + BigInt(LIBRARY_INFO.version), version);
    n.setPtr(info, LIBRARY_INFO.providers, infos);
    n.setSize(info, LIBRARY_INFO.providers + VIEW.len, providers.length);
    n.setSize(info, LIBRARY_INFO.providers + VIEW.stride, SIZE.providerInfo);
    n.setPtr(info, LIBRARY_INFO.meta, this.#map(meta));
    n.setPtr(info, LIBRARY_INFO.unload, k.register(() => (this.#instances.size ? Status.ERR_BUSY : Status.OK), p.unload as never));
    this.infoAddress = info;
    this.entry = k.register(() => inCall(() => (this.#closed ? null : info)), p.entry as never);
    kept.add(this);
  }

  #map(meta: unknown): bigint | null {
    if (meta === undefined || meta === null) return null;
    const value = new Value(meta);
    this.#values.push(value);
    if (value.tag !== Tag.MAP) throw new TypeError('meta is a map');
    return value.requireNode() + BigInt(VALUE.payload);
  }

  #describe(info: bigint, cls: ProviderClass, spec: Spec, providerId: string): void {
    const k = n.koffi();
    const p = libraryProtos();
    n.setU32(info, PROVIDER_INFO.structSize, SIZE.providerInfo);
    const names = n.alloc(SIZE.str * Math.max(1, spec.kinds.length));
    spec.kinds.forEach((kind, i) => writeStr(names + BigInt(i * SIZE.str), kind.name));
    n.setPtr(info, PROVIDER_INFO.kinds, names);
    n.setSize(info, PROVIDER_INFO.kinds + VIEW.len, spec.kinds.length);
    writeStr(info + BigInt(PROVIDER_INFO.id), providerId);
    writeStr(info + BigInt(PROVIDER_INFO.displayName), spec.displayName);
    writeStr(info + BigInt(PROVIDER_INFO.version), spec.version);
    n.setPtr(info, PROVIDER_INFO.meta, this.#map(spec.meta));

    const tables = n.alloc(SIZE.kindTable * Math.max(1, spec.kinds.length));
    spec.kinds.forEach((kind, i) => {
      const at = tables + BigInt(i * SIZE.kindTable);
      n.setU32(at, KIND_TABLE.structSize, SIZE.kindTable);
      n.setU32(at, KIND_TABLE.vtableSize, kind.tableSize);
      writeStr(at + BigInt(KIND_TABLE.kind), kind.name);
      n.setPtr(at, KIND_TABLE.vtable, tableFor(kind, cls.prototype as object, cls.name));
    });
    n.setPtr(info, PROVIDER_INFO.tables, tables);
    n.setSize(info, PROVIDER_INFO.tables + VIEW.len, spec.kinds.length);
    n.setSize(info, PROVIDER_INFO.tables + VIEW.stride, SIZE.kindTable);

    if (spec.asksAvailable) {
      const ask = available(cls);
      n.setPtr(info, PROVIDER_INFO.available, k.register((ctx: unknown, reason: bigint | null) => inCall(() => ask(ctx, reason)), p.available as never));
    }
    if (spec.buildsInstances) {
      if (spec.schema !== undefined) {
        const schema = new Value(spec.schema);
        this.#values.push(schema);
        n.setPtr(info, PROVIDER_INFO.config, schema.requireNode());
      }
      const create = this.#create(cls, spec);
      n.setPtr(
        info,
        PROVIDER_INFO.create,
        k.register((ctx: unknown, config: bigint | null, out: bigint | null, err: bigint | null) => inCall(() => create(ctx, config, out, err)), p.create as never),
      );
      n.setPtr(
        info,
        PROVIDER_INFO.destroy,
        k.register((_ctx: unknown, instance: bigint | null) => {
          if (instance) this.#instances.delete(BigInt(instance));
          try {
            inCall(() => release(instance, true));
          } catch (e) {
            process.emitWarning(`${providerId}: disposing an instance threw ${statusOf(e)[1]}`, { type: 'GuatiaoWarning' });
          }
        }, p.destroy as never),
      );
    }
    if (spec.default) {
      const Concrete = cls as unknown as new () => Provider;
      const ctx = hold(new Concrete());
      this.#defaults.push(ctx);
      n.setPtr(info, PROVIDER_INFO.ctx, ctx);
    }
  }

  #create(cls: ProviderClass, spec: Spec): (ctx: unknown, config: bigint | null, out: bigint | null, err: bigint | null) => number {
    const Concrete = cls as unknown as new (config: unknown) => Provider;
    return (_ctx, config, out, err) => {
      try {
        // Refused before anything is built: an instance with no slot to
        // land in would be held for the process.
        if (!out) throw new NullArgument('no slot for the instance');
        n.setPtr(out, 0, null);
        if (!config) throw new NullArgument('no configuration');
        const plain = new Ref(new Borrowed(() => config), []).toJS();
        const made = spec.build === undefined ? plain : spec.build(plain);
        const handle = hold(new Concrete(made));
        this.#instances.add(handle);
        n.setPtr(out, 0, handle);
        return Status.OK;
      } catch (e) {
        const [status, message] = statusOf(e);
        writeErr(err, status, message);
        return status;
      }
    };
  }

  /** How many instances a host built and has not destroyed. */
  liveInstances(): number {
    return this.#instances.size;
  }

  /**
   * Lets go of each provider's own instance and answers null to any host
   * asking for the descriptor from now on. Call it once no host will call
   * this library again; the descriptor stays readable.
   */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    for (const ctx of this.#defaults.splice(0)) release(ctx, true);
  }

  toString(): string {
    return `Library ${this.id} ${this.version}`;
  }
}

function writeStr(at: bigint, text: string): void {
  const bytes = n.encode(text);
  if (bytes.length) {
    const ptr = n.alloc(bytes.length);
    n.write(ptr, bytes);
    n.setPtr(at, 0, ptr);
  }
  n.setSize(at, VIEW.len, bytes.length);
}

function available(cls: ProviderClass): (ctx: unknown, reason: bigint | null) => boolean {
  return (_ctx, reason) => {
    let why: string | null | undefined;
    try {
      why = cls.available();
    } catch (e) {
      why = statusOf(e)[1];
    }
    if (why === undefined || why === null) return true;
    const text = String(why);
    let kept = reasons.get(text);
    if (kept === undefined) {
      const bytes = Buffer.from(text, 'utf8');
      const ptr = n.alloc(Math.max(1, bytes.length));
      n.write(ptr, bytes);
      kept = { ptr, len: bytes.length };
      reasons.set(text, kept);
    }
    if (reason) {
      n.setPtr(reason, 0, kept.ptr);
      n.setSize(reason, VIEW.len, kept.len);
    }
    return false;
  };
}

/**
 * A library of `providers`, its descriptor built now: the primitive
 * `libraryOf` builds a module's with, for a test, a library made at run
 * time, or one that declares `meta`.
 */
export function library(
  id: string,
  version: string,
  options: { readonly providers: readonly ProviderClass[]; readonly meta?: { readonly [key: string]: unknown } },
): Library {
  return new Library(id, version, options.providers, options.meta);
}

// ---- a module is a library -------------------------------------------------------

interface PackageJson {
  name?: unknown;
  version?: unknown;
  description?: unknown;
  license?: unknown;
  author?: unknown;
  contributors?: unknown;
  homepage?: unknown;
  repository?: unknown;
  bugs?: unknown;
  main?: unknown;
  exports?: unknown;
}

function findPackage(file: string): { dir: string; pkg: PackageJson } {
  for (let dir = path.dirname(file); ; dir = path.dirname(dir)) {
    const candidate = path.join(dir, 'package.json');
    if (existsSync(candidate)) return { dir, pkg: JSON.parse(readFileSync(candidate, 'utf8')) as PackageJson };
    if (path.dirname(dir) === dir) throw new TypeError(`${file} belongs to no package: no package.json above it`);
  }
}

function entryOf(pkg: PackageJson): string | undefined {
  const pick = (target: unknown): string | undefined => {
    if (typeof target === 'string') return target;
    if (target !== null && typeof target === 'object') {
      const conditions = target as Record<string, unknown>;
      for (const key of ['import', 'node', 'default', 'require']) {
        const found = pick(conditions[key]);
        if (found !== undefined) return found;
      }
    }
    return undefined;
  };
  const exports = pkg.exports;
  if (exports !== undefined && exports !== null) {
    if (typeof exports === 'object' && !Array.isArray(exports) && '.' in (exports as object)) {
      return pick((exports as Record<string, unknown>)['.']);
    }
    if (typeof exports === 'string' || !Object.keys(exports as object).some((key) => key.startsWith('.'))) {
      return pick(exports);
    }
    return undefined;
  }
  return typeof pkg.main === 'string' ? pkg.main : 'index.js';
}

function person(entry: unknown): string | undefined {
  if (typeof entry === 'string') return entry;
  if (entry !== null && typeof entry === 'object') {
    const { name, email, url } = entry as Record<string, unknown>;
    if (typeof name !== 'string') return undefined;
    return `${name}${typeof email === 'string' ? ` <${email}>` : ''}${typeof url === 'string' ? ` (${url})` : ''}`;
  }
  return undefined;
}

function urlOf(entry: unknown): string | undefined {
  if (typeof entry === 'string') return entry;
  if (entry !== null && typeof entry === 'object' && typeof (entry as { url?: unknown }).url === 'string') {
    return (entry as { url: string }).url;
  }
  return undefined;
}

/** What a package says about itself, as a library's meta. */
function metaOf(dir: string, pkg: PackageJson, name: string): Record<string, unknown> {
  const meta: Record<string, unknown> = {};
  if (typeof pkg.description === 'string' && pkg.description) meta.summary = pkg.description;
  if (typeof pkg.license === 'string' && pkg.license) meta.license = pkg.license;
  const authors = [pkg.author, ...(Array.isArray(pkg.contributors) ? pkg.contributors : [])]
    .map(person)
    .filter((a): a is string => a !== undefined);
  if (authors.length) meta.authors = authors;
  const urls: Record<string, string> = {};
  for (const [label, entry] of [
    ['Homepage', pkg.homepage],
    ['Repository', pkg.repository],
    ['Issues', pkg.bugs],
  ] as const) {
    const url = urlOf(entry);
    if (url !== undefined) urls[label] = url;
  }
  if (Object.keys(urls).length) meta.urls = urls;
  const notices = readdirSync(dir)
    .filter((file) => /^(licen[cs]e|copying)(\..*)?$/i.test(file) && statSync(path.join(dir, file)).isFile())
    .sort()
    .map((file) => ({ component: name, text: readFileSync(path.join(dir, file), 'utf8'), format: 'text' }))
    .filter((notice) => notice.text.trim() !== '');
  if (notices.length) meta.notices = notices;
  return meta;
}

const byUrl = new Map<string, Promise<Library>>();

/**
 * The library an ES module is, built once per module: its id is the
 * nearest `package.json`'s `name` (with the module's path inside the
 * package appended, extension dropped, when it is not the package's
 * entry point), its version that file's `version`, its providers the
 * `Provider` subclasses it exports, in the order its namespace lists them
 * (export names sorted), and its meta that file's `description` (as
 * `summary`), `license`, `author` and `contributors` (as `authors`),
 * `homepage`, `repository` and `bugs` (as `urls`), and its licence files
 * (as `notices`).
 *
 * `where` is a file URL, a path, or a `file:` URL string. `TypeError` for
 * a module outside any package, a package with no name or version, or a
 * module exporting no provider.
 */
export function libraryOf(where: string | URL): Promise<Library> {
  const url =
    where instanceof URL ? where : /^file:/i.test(where) ? new URL(where) : pathToFileURL(path.resolve(where));
  let found = byUrl.get(url.href);
  if (found === undefined) {
    found = build(url);
    byUrl.set(url.href, found);
    found.catch(() => byUrl.delete(url.href));
  }
  return found;
}

async function build(url: URL): Promise<Library> {
  const file = fileURLToPath(url);
  const namespace = (await import(url.href)) as Record<string, unknown>;
  const { dir, pkg } = findPackage(file);
  if (typeof pkg.name !== 'string' || !pkg.name) throw new TypeError(`${path.join(dir, 'package.json')} names no package`);
  if (typeof pkg.version !== 'string') {
    throw new TypeError(`${path.join(dir, 'package.json')}'s version is the library's version, and it has none`);
  }
  const entry = entryOf(pkg);
  const isEntry = entry !== undefined && path.resolve(dir, entry) === path.resolve(file);
  const inside = path.relative(dir, file).replace(/\.[cm]?[jt]s$/, '').split(path.sep).join('/');
  const id = isEntry ? pkg.name : `${pkg.name}/${inside}`;
  const providers = Object.keys(namespace)
    .map((key) => namespace[key])
    .filter(isProviderClass);
  if (!providers.length) throw new TypeError(`${url.href} exports no Provider subclass`);
  return new Library(id, pkg.version, [...new Set(providers)], metaOf(dir, pkg, pkg.name));
}
