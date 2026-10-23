// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package testenv

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#include <stdint.h>
#include <stdlib.h>
#include "guatiao.h"
#ifdef _WIN32
#include <windows.h>
#endif

typedef struct te_counting {
  guatiao_alloc alloc;
  int64_t live;
} te_counting;

static void *te_alloc(void *ctx, size_t size, size_t align) {
  (void)align;
  if (size == 0) return NULL;
  void *p = malloc(size);
  if (p) {
#ifdef _WIN32
    InterlockedIncrement64((volatile LONG64 *)&((te_counting *)ctx)->live);
#else
    __atomic_add_fetch(&((te_counting *)ctx)->live, 1, __ATOMIC_SEQ_CST);
#endif
  }
  return p;
}

static void te_free(void *ctx, void *p, size_t size, size_t align) {
  (void)size;
  (void)align;
  if (!p) return;
#ifdef _WIN32
  InterlockedDecrement64((volatile LONG64 *)&((te_counting *)ctx)->live);
#else
  __atomic_sub_fetch(&((te_counting *)ctx)->live, 1, __ATOMIC_SEQ_CST);
#endif
  free(p);
}

static te_counting *te_counting_new(void) {
  te_counting *c = (te_counting *)calloc(1, sizeof(te_counting));
  if (!c) return NULL;
  c->alloc.struct_size = (uint32_t)sizeof(guatiao_alloc);
  c->alloc.ctx = c;
  c->alloc.alloc = te_alloc;
  c->alloc.free = te_free;
  return c;
}

static int64_t te_counting_live(te_counting *c) {
#ifdef _WIN32
  return InterlockedCompareExchange64((volatile LONG64 *)&c->live, 0, 0);
#else
  return __atomic_load_n(&c->live, __ATOMIC_SEQ_CST);
#endif
}
*/
import "C"

import "unsafe"

// Counting is an allocator that counts the blocks it handed out and has
// not had back, for tests that prove every path frees.
type Counting struct{ c *C.te_counting }

// NewCounting answers a counting allocator, kept until Close.
func NewCounting() *Counting { return &Counting{c: C.te_counting_new()} }

// Alloc is the guatiao_alloc to build in.
func (c *Counting) Alloc() unsafe.Pointer { return unsafe.Pointer(&c.c.alloc) }

// Live is how many blocks are out.
func (c *Counting) Live() int64 { return int64(C.te_counting_live(c.c)) }

// Close releases the counter; nothing built through it may be used after.
func (c *Counting) Close() { C.free(unsafe.Pointer(c.c)) }
