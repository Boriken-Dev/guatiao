// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "native.h"
*/
import "C"

import "unsafe"

// Counting is an allocator that counts the blocks it handed out and has
// not had back, for tests that prove every path frees.
type Counting struct{ c *C.gt_counting }

// NewCounting answers a counting allocator, kept until Close.
func NewCounting() *Counting { return &Counting{c: C.gt_counting_new()} }

// Alloc is the guatiao_alloc to build in.
func (c *Counting) Alloc() unsafe.Pointer { return unsafe.Pointer(&c.c.alloc) }

// Live is how many blocks are out.
func (c *Counting) Live() int64 { return int64(C.gt_counting_live(c.c)) }

// Close releases the counter; nothing built through it may be used after.
func (c *Counting) Close() { C.gt_counting_free(c.c) }
