// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package intake

// The guatiao_intake exports, called through the addresses the core's
// Surface resolved: one caller per signature, over copies of the
// committed headers.

/*
#cgo CFLAGS: -I${SRCDIR}/include
#include <stdbool.h>
#include <stdlib.h>
#include "guatiao_intake.h"

#define gv const guatiao_value

// (schema, form, alloc, out): check, layout.
static guatiao_status gi_two(void *fn, gv *a, gv *b, const guatiao_alloc *alloc, guatiao_value *out) {
  return ((guatiao_status(*)(gv *, gv *, const guatiao_alloc *, guatiao_value *))fn)(a, b, alloc, out);
}

// (schema, alloc, out): for_schema.
static guatiao_status gi_one(void *fn, gv *a, const guatiao_alloc *alloc, guatiao_value *out) {
  return ((guatiao_status(*)(gv *, const guatiao_alloc *, guatiao_value *))fn)(a, alloc, out);
}

// (form, schema, path, alloc, out): form_for.
static guatiao_status gi_two_text(void *fn, gv *a, gv *b, guatiao_str s, const guatiao_alloc *alloc,
                                  guatiao_value *out) {
  return ((guatiao_status(*)(gv *, gv *, guatiao_str, const guatiao_alloc *, guatiao_value *))fn)(a, b, s, alloc,
                                                                                                   out);
}

// (schema, key, alloc, out): flat_keys.
static guatiao_status gi_one_text(void *fn, gv *a, guatiao_str s, const guatiao_alloc *alloc, guatiao_value *out) {
  return ((guatiao_status(*)(gv *, guatiao_str, const guatiao_alloc *, guatiao_value *))fn)(a, s, alloc, out);
}

// (schema, key, value, alloc, out): flatten, unflatten.
static guatiao_status gi_one_text_one(void *fn, gv *a, guatiao_str s, gv *b, const guatiao_alloc *alloc,
                                      guatiao_value *out) {
  return ((guatiao_status(*)(gv *, guatiao_str, gv *, const guatiao_alloc *, guatiao_value *))fn)(a, s, b, alloc,
                                                                                                   out);
}

// (schema, form, key, values, out): is_visible.
static guatiao_status gi_visible(void *fn, gv *a, gv *b, guatiao_str s, gv *c, bool *out) {
  return ((guatiao_status(*)(gv *, gv *, guatiao_str, gv *, bool *))fn)(a, b, s, c, out);
}

// (value, path) -> node: path_get, resolve.
static gv *gi_find(void *fn, gv *a, guatiao_str s) { return ((gv * (*)(gv *, guatiao_str)) fn)(a, s); }
*/
import "C"

import "unsafe"

const (
	sizeValue = C.sizeof_guatiao_value
	statusOK  = uint32(C.GUATIAO_OK)
	badValue  = uint32(C.GUATIAO_ERR_BAD_VALUE)
	wrongKind = uint32(C.GUATIAO_ERR_WRONG_KIND)
)

// The exports this package calls; every build has each.
var exportNames = []string{
	"guatiao_intake_check", "guatiao_intake_layout", "guatiao_intake_for_schema", "guatiao_intake_form_for",
	"guatiao_intake_is_visible", "guatiao_intake_path_get", "guatiao_intake_resolve", "guatiao_intake_flat_keys",
	"guatiao_intake_flatten", "guatiao_intake_unflatten",
}

// str is s as a guatiao_str for one call: the bytes are the Go string's.
func str(s string) C.guatiao_str {
	if len(s) == 0 {
		return C.guatiao_str{}
	}
	return C.guatiao_str{ptr: (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s))), len: C.size_t(len(s))}
}

func calloc() unsafe.Pointer { return C.calloc(1, sizeValue) }

func n(p unsafe.Pointer) *C.guatiao_value { return (*C.guatiao_value)(p) }

func a(p unsafe.Pointer) *C.guatiao_alloc { return (*C.guatiao_alloc)(p) }

func callTwo(fn, x, y, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gi_two(fn, n(x), n(y), a(alloc), n(out)))
}

func callOne(fn, x, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gi_one(fn, n(x), a(alloc), n(out)))
}

func callTwoText(fn, x, y unsafe.Pointer, s string, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gi_two_text(fn, n(x), n(y), str(s), a(alloc), n(out)))
}

func callOneText(fn, x unsafe.Pointer, s string, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gi_one_text(fn, n(x), str(s), a(alloc), n(out)))
}

func callOneTextOne(fn, x unsafe.Pointer, s string, y, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gi_one_text_one(fn, n(x), str(s), n(y), a(alloc), n(out)))
}

func callVisible(fn, x, y unsafe.Pointer, s string, z unsafe.Pointer) (bool, uint32) {
	var out C.bool
	st := uint32(C.gi_visible(fn, n(x), n(y), str(s), n(z), &out))
	return bool(out), st
}

func callFind(fn, x unsafe.Pointer, s string) unsafe.Pointer {
	return unsafe.Pointer(C.gi_find(fn, n(x), str(s)))
}
