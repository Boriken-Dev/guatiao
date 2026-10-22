// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// An allocator counting the blocks it handed out and has not taken back,
// over a library's default one: a tree built in it by one library and
// freed through another comes back to zero.

import { ALLOC, SIZE, alloc, koffi, ptrAt, release, setPtr, setU32 } from 'guatiao/ffi';

let protos: { alloc: unknown; free: unknown } | undefined;

/** The two callback prototypes, declared once: koffi refuses a name twice. */
function allocProtos(): { alloc: unknown; free: unknown } {
  const k = koffi();
  protos ??= {
    alloc: k.proto('void *counting_alloc_fn(void *ctx, size_t size, size_t align)'),
    free: k.proto('void counting_free_fn(void *ctx, void *ptr, size_t size, size_t align)'),
  };
  return protos;
}

export class Counting implements Disposable {
  live = 0;
  readonly alloc: bigint;
  readonly #callbacks: bigint[];

  /** Counts over `base`, the address of a library's `guatiao_alloc`. */
  constructor(base: bigint) {
    const k = koffi();
    const { alloc: allocProto, free: freeProto } = allocProtos() as { alloc: never; free: never };
    const ctx = ptrAt(base, ALLOC.ctx);
    const baseAlloc = k.decode(ptrAt(base, ALLOC.alloc), allocProto) as (c: bigint, size: number, align: number) => bigint | null;
    const baseFree = k.decode(ptrAt(base, ALLOC.free), freeProto) as (c: bigint, p: bigint, size: number, align: number) => void;
    const onAlloc = k.register((_c: unknown, size: number, align: number) => {
      const p = baseAlloc(ctx, size, align);
      if (p) this.live += 1;
      return p;
    }, k.pointer(allocProto));
    const onFree = k.register((_c: unknown, p: bigint | null, size: number, align: number) => {
      if (p) this.live -= 1;
      baseFree(ctx, p ?? 0n, size, align);
    }, k.pointer(freeProto));
    this.#callbacks = [onAlloc, onFree];
    this.alloc = alloc(SIZE.alloc);
    setU32(this.alloc, ALLOC.structSize, SIZE.alloc);
    setPtr(this.alloc, ALLOC.alloc, onAlloc);
    setPtr(this.alloc, ALLOC.free, onFree);
  }

  /** Lets the callbacks go; only once nothing built through it is left. */
  [Symbol.dispose](): void {
    const k = koffi();
    for (const cb of this.#callbacks) k.unregister(cb);
    release(this.alloc);
  }
}
