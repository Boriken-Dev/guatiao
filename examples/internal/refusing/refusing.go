// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package refusing is an allocator that counts the blocks it has out and
// refuses every allocation while told to, for the tests of a caller's
// failure paths.
package refusing

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/guatiao/include
#include <stdint.h>
#include <stdlib.h>
#include "guatiao.h"

typedef struct rf_alloc {
  guatiao_alloc alloc;
  int refusing;
  int64_t live;
} rf_alloc;

static void *rf_alloc_fn(void *ctx, size_t size, size_t align) {
  (void)align;
  rf_alloc *a = (rf_alloc *)ctx;
  if (a->refusing || size == 0) return NULL;
  void *p = malloc(size);
  if (p) a->live++;
  return p;
}

static void rf_free_fn(void *ctx, void *p, size_t size, size_t align) {
  (void)size;
  (void)align;
  if (!p) return;
  ((rf_alloc *)ctx)->live--;
  free(p);
}

static rf_alloc *rf_new(void) {
  rf_alloc *a = (rf_alloc *)calloc(1, sizeof(rf_alloc));
  if (!a) return NULL;
  a->alloc.struct_size = (uint32_t)sizeof(guatiao_alloc);
  a->alloc.ctx = a;
  a->alloc.alloc = rf_alloc_fn;
  a->alloc.free = rf_free_fn;
  return a;
}
*/
import "C"

import "unsafe"

// Alloc is the allocator, kept until Close. Its blocks come from malloc
// on the calling thread; it is for single-threaded tests.
type Alloc struct{ a *C.rf_alloc }

// New answers an allocator that does not refuse yet.
func New() *Alloc { return &Alloc{a: C.rf_new()} }

// Ptr is the guatiao_alloc to build in.
func (a *Alloc) Ptr() unsafe.Pointer { return unsafe.Pointer(&a.a.alloc) }

// Refuse sets whether every allocation is refused.
func (a *Alloc) Refuse(on bool) {
	a.a.refusing = 0
	if on {
		a.a.refusing = 1
	}
}

// Live is how many blocks are out.
func (a *Alloc) Live() int64 { return int64(a.a.live) }

// Close releases the allocator; nothing built through it may be used after.
func (a *Alloc) Close() { C.free(unsafe.Pointer(a.a)) }
