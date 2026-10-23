// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package serde

// The guatiao_serde exports, called through the addresses the core's
// Surface resolved: one caller per signature, over copies of the
// committed headers.

/*
#cgo CFLAGS: -I${SRCDIR}/include
#include <stdlib.h>
#include "guatiao_serde.h"

typedef guatiao_status (*gs_parse_fn)(guatiao_str, uint32_t, const guatiao_alloc *, guatiao_value *,
                                      guatiao_parse_error *);
typedef guatiao_status (*gs_emit_fn)(const guatiao_value *, uint32_t, const guatiao_alloc *, guatiao_value *);
typedef guatiao_status (*gs_free_fn)(guatiao_value *);

static guatiao_status gs_parse(void *fn, guatiao_str text, uint32_t how, const guatiao_alloc *alloc,
                               guatiao_value *out, guatiao_parse_error *err) {
  return ((gs_parse_fn)fn)(text, how, alloc, out, err);
}

static guatiao_status gs_emit(void *fn, const guatiao_value *v, uint32_t how, const guatiao_alloc *alloc,
                              guatiao_value *out) {
  return ((gs_emit_fn)fn)(v, how, alloc, out);
}

static guatiao_status gs_free(void *fn, guatiao_value *v) { return ((gs_free_fn)fn)(v); }

// The text of a STRING node; empty for any other.
static guatiao_str gs_text(const guatiao_value *v) {
  guatiao_str s = {0};
  if (v->tag == GUATIAO_STRING) {
    s.ptr = v->payload.text.ptr;
    s.len = v->payload.text.len;
  }
  return s;
}
*/
import "C"

import "unsafe"

const (
	sizeValue  = C.sizeof_guatiao_value
	tagString  = uint32(C.GUATIAO_STRING)
	statusOK   = uint32(C.GUATIAO_OK)
	badValue   = uint32(C.GUATIAO_ERR_BAD_VALUE)
	formatJSON = uint32(C.GUATIAO_FORMAT_JSON)
	formatTOML = uint32(C.GUATIAO_FORMAT_TOML)
	formatYAML = uint32(C.GUATIAO_FORMAT_YAML)
	pretty     = uint32(C.GUATIAO_PRETTY)
)

// How bytes are written, and whether data URIs are read back as bytes:
// one Bytes flag, or'd with ReadDataURIs, through How.
const (
	BytesDataURI = uint32(C.GUATIAO_BYTES_DATA_URI)
	BytesBase64  = uint32(C.GUATIAO_BYTES_BASE64)
	BytesArray   = uint32(C.GUATIAO_BYTES_ARRAY)
	BytesRefuse  = uint32(C.GUATIAO_BYTES_REFUSE)
	ReadDataURIs = uint32(C.GUATIAO_READ_DATA_URIS)
)

// str is s as a guatiao_str for one call: the bytes are the Go string's.
func str(s string) C.guatiao_str {
	if len(s) == 0 {
		return C.guatiao_str{}
	}
	return C.guatiao_str{ptr: (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s))), len: C.size_t(len(s))}
}

func calloc() unsafe.Pointer { return C.calloc(1, sizeValue) }

func node(p unsafe.Pointer) *C.guatiao_value { return (*C.guatiao_value)(p) }

func alloc(p unsafe.Pointer) *C.guatiao_alloc { return (*C.guatiao_alloc)(p) }

// release frees what the node at p owns, then p itself.
func release(free, p unsafe.Pointer) {
	C.gs_free(free, node(p))
	C.free(p)
}

// failure is a guatiao_parse_error read and freed.
type failure struct {
	format, line, column uint32
	message              string
}

func parse(fn, free unsafe.Pointer, text string, how uint32, a, out unsafe.Pointer) (failure, uint32) {
	var e C.guatiao_parse_error
	st := uint32(C.gs_parse(fn, str(text), C.uint32_t(how), alloc(a), node(out), &e))
	f := failure{format: uint32(e.format), line: uint32(e.line), column: uint32(e.column)}
	s := C.gs_text(&e.message)
	f.message = C.GoStringN((*C.char)(unsafe.Pointer(s.ptr)), C.int(s.len))
	C.gs_free(free, &e.message)
	return f, st
}

func emit(fn unsafe.Pointer, n unsafe.Pointer, how uint32, a, out unsafe.Pointer) uint32 {
	return uint32(C.gs_emit(fn, node(n), C.uint32_t(how), alloc(a), node(out)))
}

func textOf(p unsafe.Pointer) (string, bool) {
	if node(p).tag != C.uint32_t(tagString) {
		return "", false
	}
	s := C.gs_text(node(p))
	return C.GoStringN((*C.char)(unsafe.Pointer(s.ptr)), C.int(s.len)), true
}
