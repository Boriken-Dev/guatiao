// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package valuegreeter calls a greeter table's greet slot the way glue
// does for a kind whose Go method answers a *guatiao.Value: the tree is
// kept by the caller, not decoded and freed inside the call.
package valuegreeter

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/guatiao/include
#include <stddef.h>
#include <stdint.h>
#include "guatiao.h"

static uint32_t vg_greet(void *fn, uintptr_t ctx, const void *name, size_t len, void *out, void *err) {
  guatiao_str s = {(const uint8_t *)name, len};
  return ((guatiao_status (*)(void *, guatiao_str, guatiao_map *, guatiao_provider_error *))fn)(
      (void *)ctx, s, (guatiao_map *)out, (guatiao_provider_error *)err);
}
*/
import "C"

import (
	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// Greet calls slot 0 of the greeter table r wraps and answers the map it
// built, for the caller to close.
func Greet(r guatiao.Remote, name string) (*guatiao.Value, error) {
	c, err := r.Begin(0)
	if err != nil {
		return nil, err
	}
	defer c.End()
	p, n := c.Str(name)
	out := c.Out(guatiao.MapClass)
	errp := c.Err()
	st := C.vg_greet(c.Fn(), C.uintptr_t(c.Ctx()), p, C.size_t(n), out, errp)
	if err := c.Status(uint32(st)); err != nil {
		return nil, err
	}
	var res *guatiao.Value
	if err := c.Answer(out, guatiao.MapClass, &res); err != nil {
		return nil, err
	}
	return res, nil
}
