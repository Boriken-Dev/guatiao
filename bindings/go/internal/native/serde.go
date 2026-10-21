// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "native.h"
*/
import "C"

import "unsafe"

// ParseFailure is a guatiao_parse_error read and freed.
type ParseFailure struct {
	Format, Line, Column uint32
	Message              string
}

func (l *Lib) serdeParse(format uint32, text string, how uint32, alloc, out unsafe.Pointer) (ParseFailure, uint32) {
	var e C.guatiao_parse_error
	var st C.guatiao_status
	switch format {
	case FormatJSON:
		st = C.gt_serde_guatiao_json_parse_ex(str(text), C.uint32_t(how), a(alloc), n(out), &e)
	case FormatTOML:
		st = C.gt_serde_guatiao_toml_parse_ex(str(text), C.uint32_t(how), a(alloc), n(out), &e)
	default:
		st = C.gt_serde_guatiao_yaml_parse_ex(str(text), C.uint32_t(how), a(alloc), n(out), &e)
	}
	f := ParseFailure{Format: uint32(e.format), Line: uint32(e.line), Column: uint32(e.column)}
	msg := unsafe.Pointer(&e.message)
	if Tag(msg) == TagString {
		f.Message = TextOf(msg)
	}
	ValueFree(msg)
	return f, uint32(st)
}

// Parse reads a document of format (FormatJSON, FormatTOML, FormatYAML)
// into out, built through alloc.
func (l *Lib) Parse(format uint32, text string, how uint32, alloc, out unsafe.Pointer) (ParseFailure, uint32) {
	return l.serdeParse(format, text, how, alloc, out)
}

// Emit writes the value at node as a document of format into out, a
// string node; pretty picks the indented form.
func (l *Lib) Emit(format uint32, node unsafe.Pointer, how uint32, pretty bool, alloc, out unsafe.Pointer) uint32 {
	switch format {
	case FormatJSON:
		if pretty {
			return uint32(C.gt_serde_guatiao_json_emit_pretty(n(node), C.uint32_t(how), a(alloc), n(out)))
		}
		return uint32(C.gt_serde_guatiao_json_emit(n(node), C.uint32_t(how), a(alloc), n(out)))
	case FormatTOML:
		if pretty {
			how |= Pretty
		}
		return uint32(C.gt_serde_guatiao_toml_emit(n(node), C.uint32_t(how), a(alloc), n(out)))
	default:
		if pretty {
			how |= Pretty
		}
		return uint32(C.gt_serde_guatiao_yaml_emit(n(node), C.uint32_t(how), a(alloc), n(out)))
	}
}

// ---- intake ---------------------------------------------------------------

func IntakeCheck(schema, form, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_check(n(schema), n(form), a(alloc), n(out)))
}

func IntakeLayout(schema, form, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_layout(n(schema), n(form), a(alloc), n(out)))
}

func IntakeForSchema(schema, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_for_schema(n(schema), a(alloc), n(out)))
}

func IntakeFormFor(form, schema unsafe.Pointer, path string, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_form_for(n(form), n(schema), str(path), a(alloc), n(out)))
}

func IntakeIsVisible(schema, form unsafe.Pointer, key string, values unsafe.Pointer) (bool, uint32) {
	var out C.bool
	st := uint32(C.gt_intake_guatiao_intake_is_visible(n(schema), n(form), str(key), n(values), &out))
	return bool(out), st
}

func IntakePathGet(value unsafe.Pointer, path string) unsafe.Pointer {
	return unsafe.Pointer(C.gt_intake_guatiao_intake_path_get(n(value), str(path)))
}

func IntakeResolve(schema unsafe.Pointer, key string) unsafe.Pointer {
	return unsafe.Pointer(C.gt_intake_guatiao_intake_resolve(n(schema), str(key)))
}

func IntakeFlatKeys(schema unsafe.Pointer, key string, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_flat_keys(n(schema), str(key), a(alloc), n(out)))
}

func IntakeFlatten(schema unsafe.Pointer, key string, value, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_flatten(n(schema), str(key), n(value), a(alloc), n(out)))
}

func IntakeUnflatten(schema unsafe.Pointer, key string, store, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_intake_guatiao_intake_unflatten(n(schema), str(key), n(store), a(alloc), n(out)))
}

// ---- the counting allocator ---------------------------------------------------

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
