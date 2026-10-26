// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What several suites share: the repository's paths, its headers, and an
// allocator that counts the blocks it handed out and has not taken back.

import { readFileSync } from 'node:fs';
import path from 'node:path';

import { ALLOC, SIZE } from '../src/abi.js';
import * as n from '../src/native.js';
import { REPO } from './setup.js';

export { REPO };

export const HEADER = path.join(REPO, 'crates', 'guatiao', 'include', 'guatiao.h');
export const GREETER_KIND_H = path.join(REPO, 'examples', 'greeter_kind', 'include', 'greeter_kind.h');

export function readText(file: string): string {
  return readFileSync(file, 'utf8');
}

let protos: { alloc: unknown; free: unknown } | undefined;

function allocProtos(): { alloc: unknown; free: unknown } {
  const k = n.koffi();
  protos ??= {
    alloc: k.proto('void *test_alloc_fn(void *ctx, size_t size, size_t align)'),
    free: k.proto('void test_free_fn(void *ctx, void *ptr, size_t size, size_t align)'),
  };
  return protos;
}

/** The library's default allocator, wrapped to count live blocks. */
export class Counting implements Disposable {
  live = 0;
  /** Set, every allocation is refused: a caller's failure paths. */
  refusing = false;
  readonly alloc: bigint;
  readonly #callbacks: bigint[];

  constructor() {
    const k = n.koffi();
    const base = n.core().alloc;
    const { alloc: allocProto, free: freeProto } = allocProtos();
    const ctx = n.ptrAt(base, ALLOC.ctx);
    const baseAlloc = k.decode(n.ptrAt(base, ALLOC.alloc), allocProto as never) as (
      c: bigint,
      size: number,
      align: number,
    ) => bigint | null;
    const baseFree = k.decode(n.ptrAt(base, ALLOC.free), freeProto as never) as (
      c: bigint,
      p: bigint,
      size: number,
      align: number,
    ) => void;
    const onAlloc = k.register((_c: unknown, size: number, align: number) => {
      if (this.refusing) return null;
      const p = baseAlloc(ctx, size, align);
      if (p) this.live += 1;
      return p;
    }, k.pointer(allocProto as never));
    const onFree = k.register((_c: unknown, p: bigint | null, size: number, align: number) => {
      if (p) this.live -= 1;
      baseFree(ctx, p ?? 0n, size, align);
    }, k.pointer(freeProto as never));
    this.#callbacks = [onAlloc, onFree];
    this.alloc = n.alloc(SIZE.alloc);
    n.setU32(this.alloc, ALLOC.structSize, SIZE.alloc);
    n.setPtr(this.alloc, ALLOC.alloc, onAlloc);
    n.setPtr(this.alloc, ALLOC.free, onFree);
  }

  /** Lets the callbacks go; only once nothing built through it is left. */
  [Symbol.dispose](): void {
    const k = n.koffi();
    for (const cb of this.#callbacks) k.unregister(cb);
    n.release(this.alloc);
  }
}

export function libraryName(base: string): string {
  return n.libraryFilename(base);
}
