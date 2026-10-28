// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * `Registry`: a host's table of loaded libraries and the providers they
 * offer, over `guatiao_registry_*`.
 *
 * @module
 */

import { SIZE, VIEW, WORD } from './abi.js';
import { BadValue, NotFound, raiseForStatus } from './errors.js';
import { type KindLike, type KindsDocument, type Remote, asKind } from './kinds.js';
import * as n from './native.js';
import { Borrowed, Ref, Value, adopt } from './value.js';

/**
 * Where Windows finds what a file a registry opens imports. `'library'`:
 * beside the file, beside the host's executable and in the system
 * directory, not the current directory or `PATH` (the default). `'path'`:
 * those first; a file they leave an import missing for is opened again
 * with Windows' standard search from its own directory (the system
 * directories, the current directory and `PATH`), as a host adding a
 * Python bridge needs.
 */
export type ImportSearch = 'library' | 'path';

/** `GUATIAO_IMPORT_SEARCH_LIBRARY` and `_PATH`. */
const IMPORT_SEARCH_LIBRARY = 0;
const IMPORT_SEARCH_PATH = 1;

/** What `providerTable` answers: a table, its size, and the provider's `ctx`. */
export interface ProviderTable {
  /** The table's address; `0n` when there is none. */
  readonly table: bigint;
  readonly size: number;
  /** The provider's own instance, or `0n`. */
  readonly ctx: bigint;
}

/** A library this registry keeps alive: anything with an entry and an id. */
export interface RegistrableLibrary {
  readonly id: string;
  /** The address of a `guatiao_library_entry`. */
  readonly entry: bigint;
}

/**
 * An instance `Registry.create` built. `close()` runs the provider's
 * `destroy`; `ctx` is what every call through its tables takes.
 */
export class Instance implements Disposable {
  readonly #registry: Registry;
  readonly #key: string;
  readonly ctx: bigint;
  #closed = false;

  /** @internal */
  constructor(registry: Registry, key: string, ctx: bigint) {
    this.#registry = registry;
    this.#key = key;
    this.ctx = ctx;
  }

  /** This instance as `kind`: a remote whose calls go to it. Valid until it is closed. */
  bind<K extends KindLike>(kind: K): Remote<K> {
    const k = asKind(kind);
    const { table, size } = this.#registry.providerTable(this.#key, k.name);
    if (!table) throw new NotFound(`${this.#key} serves no ${k.name} table`);
    return k.remote(table, size, this.ctx) as unknown as Remote<K>;
  }

  get closed(): boolean {
    return this.#closed;
  }

  /** Destroys the instance, through the provider that built it. */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#registry.destroy(this.#key, this.ctx);
  }

  [Symbol.dispose](): void {
    this.close();
  }
}

export interface ScanOptions {
  /** Highest version first. */
  descending?: boolean;
  /** One `[!]KEY=VALUE` per line, matched against what a library declares. */
  rules?: string;
}

/** A host's registry. Used from one thread at a time, like the handle it wraps. */
export class Registry implements Disposable {
  readonly #handle: bigint;
  readonly #alloc: bigint;
  readonly #kept: RegistrableLibrary[] = [];
  #closed = false;

  /** A registry for a host named `id` at `version`. `{ alloc }` names the allocator its answers are built in. */
  constructor(id: string, version: string, options: { alloc?: bigint } = {}) {
    const lib = n.core();
    this.#alloc = options.alloc ?? lib.alloc;
    const handle = lib.fn.guatiao_registry_new!(n.str(id), n.str(version), this.#alloc) as bigint | null;
    if (!handle) throw new Error('guatiao_registry_new refused: an id or version not UTF-8, or an incomplete allocator');
    this.#handle = handle;
  }

  #open(): bigint {
    if (this.#closed) throw new TypeError('this registry is closed');
    return this.#handle;
  }

  get closed(): boolean {
    return this.#closed;
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    n.core().fn.guatiao_registry_free!(this.#handle);
  }

  [Symbol.dispose](): void {
    this.close();
  }

  /** One `(reg, ..., alloc, out)` answer, as plain data, freed. */
  #answer(name: string, ...args: unknown[]): unknown {
    const handle = this.#open();
    const out = n.alloc(SIZE.value);
    let status: number;
    try {
      status = n.core().fn[name]!(handle, ...args, this.#alloc, out) as number;
    } catch (e) {
      n.freeNode(out);
      n.release(out);
      throw e;
    }
    const value = adopt(out, this.#alloc);
    try {
      raiseForStatus(status);
      return value.toJS();
    } finally {
      value.close();
    }
  }

  #keyCall(name: string, key: string): void {
    raiseForStatus(n.core().fn[name]!(this.#open(), n.str(key)) as number);
  }

  // ---- loading ------------------------------------------------------------

  /** Maps the library at `path`: an answer saying what was loaded or skipped. */
  loadFile(path: string): Record<string, unknown> {
    return this.#answer('guatiao_registry_load_file', n.str(path)) as Record<string, unknown>;
  }

  /** Registers a library from its entry point, the address of a `guatiao_library_entry`. */
  registerEntry(name: string, entry: bigint): Record<string, unknown> {
    return this.#answer('guatiao_registry_register_entry', n.str(name), entry) as Record<string, unknown>;
  }

  /**
   * Registers a library written here (`library()`, `libraryOf()`) under its
   * id, and keeps it alive as long as this registry. It is linked, not
   * mapped: `unload` answers `WrongKind`, `retire` takes it out.
   */
  registerLibrary(library: RegistrableLibrary): Record<string, unknown> {
    this.#kept.push(library);
    return this.registerEntry(library.id, library.entry);
  }

  /** Loads every library in `dir` that `rules` admit. */
  scanDir(dir: string, options: ScanOptions = {}): Record<string, unknown> {
    if (options.rules === undefined) {
      return this.#answer('guatiao_registry_scan_dir', n.str(dir), options.descending ?? false) as Record<string, unknown>;
    }
    return this.#answer(
      'guatiao_registry_scan_dir_rules',
      n.str(dir),
      options.descending ?? false,
      n.str(options.rules),
    ) as Record<string, unknown>;
  }

  /** Loads from each directory of a search path, as the platform separates them. */
  scanPath(spec: string, options: ScanOptions = {}): Record<string, unknown> {
    return this.#answer(
      'guatiao_registry_scan_path',
      n.str(spec),
      options.descending ?? false,
      n.str(options.rules ?? ''),
    ) as Record<string, unknown>;
  }

  // ---- what is loaded ---------------------------------------------------------

  libraries(): Record<string, unknown>[] {
    return this.#answer('guatiao_registry_libraries') as Record<string, unknown>[];
  }

  /** The providers claiming `kind` (every one for `''`), best first. */
  providers(kind = ''): Record<string, unknown>[] {
    return this.#answer('guatiao_registry_providers', n.str(kind)) as Record<string, unknown>[];
  }

  /** The providers of `kind` that can run here, best first. */
  available(kind = ''): Record<string, unknown>[] {
    return this.#answer('guatiao_registry_available', n.str(kind)) as Record<string, unknown>[];
  }

  /** Why nothing, or something, answers for `kind`. */
  whyNot(kind: string): Record<string, unknown> {
    return this.#answer('guatiao_registry_why_not', n.str(kind)) as Record<string, unknown>;
  }

  /** The provider filed under `key`. `NotFound` for none. */
  provider(key: string): Record<string, unknown> {
    return this.#answer('guatiao_registry_provider', n.str(key)) as Record<string, unknown>;
  }

  /** The best provider of `kind`. */
  best(kind: string): Record<string, unknown> {
    return this.#answer('guatiao_registry_best', n.str(kind)) as Record<string, unknown>;
  }

  /** Whether the provider under `key` can run here, and if not, its reason. */
  providerAvailable(key: string): { available: boolean; reason?: string } {
    const reason = Buffer.alloc(SIZE.str);
    const ok = n.core().fn.guatiao_registry_provider_available!(this.#open(), n.str(key), reason) as boolean;
    if (ok) return { available: true };
    const len = Number(n.wordOf(reason, VIEW.len));
    const text = len ? Buffer.from(n.bytesAt(n.wordOf(reason, VIEW.ptr), len)).toString('utf8') : '';
    return { available: false, reason: text };
  }

  // ---- keys and priority -------------------------------------------------------

  /** Files providers under keys made from `template` (`%id`, `%name`, `%library`, `%version`). */
  keyedBy(template: string): this {
    raiseForStatus(n.core().fn.guatiao_registry_keyed_by!(this.#open(), n.str(template)) as number);
    return this;
  }

  /** Files libraries under keys made from `template` (`%id`, `%version`). */
  librariesKeyedBy(template: string): this {
    raiseForStatus(n.core().fn.guatiao_registry_libraries_keyed_by!(this.#open(), n.str(template)) as number);
    return this;
  }

  setPriority(id: string, priority: number): void {
    raiseForStatus(n.core().fn.guatiao_registry_set_priority!(this.#open(), n.str(id), priority) as number);
  }

  priority(id: string): number {
    return n.core().fn.guatiao_registry_priority!(this.#open(), n.str(id)) as number;
  }

  /**
   * Where Windows finds what the files this registry opens from now on
   * import: `'library'` (the default) or `'path'`. Unix ignores it.
   */
  get importSearch(): ImportSearch {
    const code = n.core().fn.guatiao_registry_import_search!(this.#open()) as number;
    return code === IMPORT_SEARCH_PATH ? 'path' : 'library';
  }

  set importSearch(search: ImportSearch) {
    const code = search === 'path' ? IMPORT_SEARCH_PATH : search === 'library' ? IMPORT_SEARCH_LIBRARY : -1;
    if (code < 0) throw new BadValue(`no import search ${String(search)}: 'library' or 'path'`);
    raiseForStatus(n.core().fn.guatiao_registry_set_import_search!(this.#open(), code) as number);
  }

  // ---- a provider's table and configuration --------------------------------------

  /**
   * The table the provider under `key` speaks `kind` through; without
   * `kind`, its single table. `table` is `0n` when there is none.
   */
  providerTable(key: string, kind?: string): ProviderTable {
    const handle = this.#open();
    const fn = n.core().fn;
    const size = Buffer.alloc(WORD);
    const table =
      kind === undefined
        ? (fn.guatiao_registry_provider_vtable!(handle, n.str(key), size) as bigint | null)
        : (fn.guatiao_registry_provider_table!(handle, n.str(key), n.str(kind), size) as bigint | null);
    const ctx = fn.guatiao_registry_provider_ctx!(handle, n.str(key)) as bigint | null;
    return { table: table ?? 0n, size: Number(n.wordOf(size)), ctx: ctx ?? 0n };
  }

  /**
   * The provider under `key` as `kind`: a remote calling its table with
   * its own instance. `NotFound` when it serves no table for the kind,
   * `FloorMismatch` when its table is not the one `kind` describes. A
   * provider that only builds instances is bound through
   * `create(...).bind(kind)`. Valid while the library is: until it is
   * unloaded or, once retired, collected.
   */
  bind<K extends KindLike>(key: string, kind: K): Remote<K> {
    const k = asKind(kind);
    const { table, size, ctx } = this.providerTable(key, k.name);
    if (!table) throw new NotFound(`${key} serves no ${k.name} table`);
    return k.remote(table, size, ctx) as unknown as Remote<K>;
  }

  /**
   * A read-only view of the provider's configuration schema, borrowed from
   * this registry (never closed), or `undefined` when it declares none.
   */
  providerConfig(key: string): Ref | undefined {
    const handle = this.#open();
    const text = n.str(key);
    const find = (): bigint | null => {
      if (this.#closed) return null;
      return (n.core().fn.guatiao_registry_provider_config!(handle, text) as bigint | null) ?? null;
    };
    if (find() === null) return undefined;
    return new Ref(new Borrowed(find), []);
  }

  /**
   * The descriptions of the kinds the provider filed under `key` serves,
   * and of every object kind their slots take or answer, as the document
   * `kindsOf` loads: how a host binds a kind it has no `.kinds.json` for.
   * `undefined` when the provider carries none (one written by hand).
   * `NotFound` when
   * nothing answers to `key`; `BadValue` when what it carries is not a
   * description.
   *
   * ```ts
   * const { greeter } = kindsOf(reg.kindDescriptions('derived_greeter_hello')!);
   * ```
   */
  kindDescriptions(key: string): KindsDocument | undefined {
    const document = this.#answer('guatiao_registry_kind_descriptions', n.str(key));
    return document === undefined ? undefined : (document as KindsDocument);
  }

  /**
   * What the provider filed under `key` declared beyond its schema -- its
   * legal notices, or its form under `'form'` (`providerFormKey` of
   * `guatiao-intake`) -- as a `Value` the caller closes, or `undefined` when
   * it declares nothing. A copy, unlike `providerConfig`: it outlives the
   * library. `NotFound` when nothing answers to `key`.
   */
  providerMeta(key: string): Value | undefined {
    const handle = this.#open();
    const out = n.alloc(SIZE.value);
    let status: number;
    try {
      status = n.core().fn.guatiao_registry_provider_meta!(handle, n.str(key), this.#alloc, out) as number;
    } catch (e) {
      n.freeNode(out);
      n.release(out);
      throw e;
    }
    const value = adopt(out, this.#alloc);
    try {
      raiseForStatus(status);
    } catch (e) {
      value.close();
      throw e;
    }
    if (value.isAbsent()) {
      value.close();
      return undefined;
    }
    return value;
  }

  /**
   * An instance of the provider under `key`, built from `config` (a
   * `Value`, a `Ref` or plain data), which the host checks against the
   * provider's schema first. The instance is closed by the caller.
   */
  create(key: string, config: unknown): Instance {
    const handle = this.#open();
    const owned = config instanceof Ref ? undefined : new Value(config, { alloc: this.#alloc });
    const node = (owned ?? (config as Ref)).requireNode();
    const out = Buffer.alloc(WORD);
    const err = n.alloc(SIZE.providerError);
    try {
      const status = n.core().fn.guatiao_registry_provider_create!(handle, n.str(key), node, out, err) as number;
      n.check(status, err);
    } finally {
      n.release(err);
      owned?.close();
    }
    return new Instance(this, key, n.wordOf(out));
  }

  /** @internal */
  destroy(key: string, ctx: bigint): void {
    if (this.#closed) return;
    n.core().fn.guatiao_registry_provider_destroy!(this.#handle, n.str(key), ctx);
  }

  /**
   * Takes the library under `key` (the LIBRARY key) out of this registry
   * and leaves it mapped until nothing uses it. `NotFound` for none.
   */
  retire(key: string): void {
    this.#keyCall('guatiao_registry_retire', key);
  }

  /** Closes every retired library nothing uses, and answers their keys. */
  collect(): string[] {
    return this.#answer('guatiao_registry_collect') as string[];
  }

  /**
   * Unmaps the library under `key` when it agrees: `Busy` while something
   * it handed out lives, `Unsupported` when it has no `unload` function,
   * `WrongKind` for a library linked rather than mapped, `NotFound` for
   * none. `{ unchecked: true }` insists on a library with no `unload`
   * function. No table, `ctx` or schema taken from it is used again.
   */
  unload(key: string, options: { unchecked?: boolean } = {}): void {
    this.#keyCall(options.unchecked ? 'guatiao_registry_unload_unchecked' : 'guatiao_registry_unload', key);
  }

  // ---- sources: libraries that come and go -------------------------------------

  /**
   * Maps a file exporting `guatiao_source` (a bridge to another language,
   * say) and adds the source it answers for `config`. Answers the source's
   * name, the path. Nothing it serves loads until `refresh()`.
   */
  addSourceFile(path: string, config: unknown = {}): string {
    const handle = this.#open();
    const owned = config instanceof Ref ? undefined : new Value(config, { alloc: this.#alloc });
    const node = (owned ?? (config as Ref)).requireNode();
    const err = n.alloc(SIZE.providerError);
    try {
      n.check(n.core().fn.guatiao_registry_add_source_file!(handle, n.str(path), node, err) as number, err);
    } finally {
      n.release(err);
      owned?.close();
    }
    return path;
  }

  /** Watches a directory: each `refresh()` loads, retires and reloads its libraries as they change. */
  watchDir(dir: string, options: { rules?: string } = {}): void {
    raiseForStatus(
      n.core().fn.guatiao_registry_watch_dir!(this.#open(), n.str(dir), n.str(options.rules ?? '')) as number,
    );
  }

  /**
   * Asks every source and watched directory what it serves now and makes
   * this registry agree: `{ loaded, retired, skipped, refused }`.
   */
  refresh(): { loaded: Record<string, unknown>[]; retired: Record<string, unknown>[]; skipped: Record<string, unknown>[]; refused: Record<string, unknown>[] } {
    return this.#answer('guatiao_registry_refresh') as {
      loaded: Record<string, unknown>[];
      retired: Record<string, unknown>[];
      skipped: Record<string, unknown>[];
      refused: Record<string, unknown>[];
    };
  }

  /** The `const guatiao_host_info *` this registry hands its libraries. */
  host(): bigint {
    return (n.core().fn.guatiao_registry_host!(this.#open()) as bigint | null) ?? 0n;
  }
}
