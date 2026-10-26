// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

// The visitor's callbacks and the walker, as Go functions C calls. A file
// with an //export holds only declarations in its preamble.

/*
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
*/
import "C"

import (
	"slices"
	"strconv"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// building is the plain data a visit is building.
var building struct {
	stack []any
	keys  []string
	key   string
	root  any
}

func put(v any) C.uint32_t {
	b := &building
	if len(b.stack) == 0 {
		b.root = v
		return 0
	}
	switch top := b.stack[len(b.stack)-1].(type) {
	case map[string]any:
		top[b.key] = v
	case *[]any:
		*top = append(*top, v)
	}
	return 0
}

func begin(c any) C.uint32_t {
	building.keys = append(building.keys, building.key)
	building.stack = append(building.stack, c)
	return 0
}

func goText(p *C.char, n C.size_t) string { return C.GoStringN(p, C.int(n)) }

//export goAbsent
func goAbsent(unsafe.Pointer) C.uint32_t { return put(nil) }

//export goNull
func goNull(unsafe.Pointer) C.uint32_t { return put(nil) }

//export goBool
func goBool(_ unsafe.Pointer, b C.bool) C.uint32_t { return put(bool(b)) }

//export goNumber
func goNumber(_ unsafe.Pointer, p *C.char, n C.size_t) C.uint32_t {
	return put(guatiao.Number(goText(p, n)))
}

//export goString
func goString(_ unsafe.Pointer, p *C.char, n C.size_t) C.uint32_t { return put(goText(p, n)) }

//export goBytes
func goBytes(_ unsafe.Pointer, p *C.char, n C.size_t) C.uint32_t {
	return put(C.GoBytes(unsafe.Pointer(p), C.int(n)))
}

//export goListBegin
func goListBegin(_ unsafe.Pointer, n C.uint64_t) C.uint32_t {
	l := make([]any, 0, hint(n))
	return begin(&l)
}

// hint is a count to size for: GUATIAO_LEN_UNKNOWN, or anything large, is 0.
func hint(n C.uint64_t) int {
	if n > 1<<20 {
		return 0
	}
	return int(n)
}

//export goMapBegin
func goMapBegin(_ unsafe.Pointer, n C.uint64_t) C.uint32_t {
	return begin(make(map[string]any, hint(n)))
}

//export goKey
func goKey(_ unsafe.Pointer, p *C.char, n C.size_t) C.uint32_t {
	building.key = goText(p, n)
	return 0
}

//export goEnd
func goEnd(unsafe.Pointer) C.uint32_t {
	b := &building
	c := b.stack[len(b.stack)-1]
	b.stack = b.stack[:len(b.stack)-1]
	b.key = b.keys[len(b.keys)-1]
	b.keys = b.keys[:len(b.keys)-1]
	if l, ok := c.(*[]any); ok {
		return put(*l)
	}
	return put(c)
}

// walking is what the walker hands over, and why it stopped.
var walking struct {
	data   any
	failed uint32
}

//export goWalk
func goWalk(_ unsafe.Pointer, into unsafe.Pointer) C.uint32_t {
	walking.failed = 0
	walk(into, walking.data)
	return C.uint32_t(walking.failed)
}

// walk calls the visitor at into once per event of x, keys sorted as the
// package writes a Go map.
func walk(into unsafe.Pointer, x any) {
	if walking.failed != 0 {
		return
	}
	switch v := x.(type) {
	case nil:
		walking.failed = event(into, slotNull)
	case bool:
		walking.failed = boolean(into, v)
	case int:
		walking.failed = text(into, slotNumber, strconv.Itoa(v))
	case float64:
		walking.failed = text(into, slotNumber, strconv.FormatFloat(v, 'g', -1, 64))
	case string:
		walking.failed = text(into, slotString, v)
	case []any:
		walking.failed = count(into, slotList, len(v))
		for _, item := range v {
			walk(into, item)
		}
		if walking.failed == 0 {
			walking.failed = event(into, slotListEnd)
		}
	case map[string]any:
		keys := make([]string, 0, len(v))
		for k := range v {
			keys = append(keys, k)
		}
		slices.Sort(keys)
		walking.failed = count(into, slotMap, len(v))
		for _, k := range keys {
			if walking.failed == 0 {
				walking.failed = text(into, slotKey, k)
			}
			walk(into, v[k])
		}
		if walking.failed == 0 {
			walking.failed = event(into, slotMapEnd)
		}
	default:
		walking.failed = 7
	}
}
