// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include <stdlib.h>
#include "native.h"

static void gt_call_destroy(void *fn, uintptr_t ctx) { ((void (*)(void *))fn)((void *)ctx); }

// An instance is a ctx, which may be a number shaped like an address:
// it crosses as an integer, so Go never holds one in a pointer.
static guatiao_status gt_create(const guatiao_registry *reg, guatiao_str key, const guatiao_value *config,
                                uintptr_t *out, guatiao_provider_error *err) {
  void *made = NULL;
  guatiao_status st = gt_core_guatiao_registry_provider_create(reg, key, config, &made, err);
  *out = (uintptr_t)made;
  return st;
}

static void gt_destroy(const guatiao_registry *reg, guatiao_str key, uintptr_t instance) {
  gt_core_guatiao_registry_provider_destroy(reg, key, (void *)instance);
}

static uintptr_t gt_ctx(const guatiao_registry *reg, guatiao_str key) {
  return (uintptr_t)gt_core_guatiao_registry_provider_ctx(reg, key);
}
*/
import "C"

import "unsafe"

func reg(p unsafe.Pointer) *C.guatiao_registry { return (*C.guatiao_registry)(p) }

// RegistryNew is a registry for a host, or nil when refused.
func RegistryNew(id, version string, alloc unsafe.Pointer) unsafe.Pointer {
	return unsafe.Pointer(C.gt_core_guatiao_registry_new(str(id), str(version), a(alloc)))
}

func RegistryFree(r unsafe.Pointer) { C.gt_core_guatiao_registry_free(reg(r)) }

func RegistryHost(r unsafe.Pointer) unsafe.Pointer {
	return unsafe.Pointer(C.gt_core_guatiao_registry_host(reg(r)))
}

// RegistryKeyCall runs one of the calls taking a registry and a key or
// template: name is the export's name without its prefix.
func RegistryKeyCall(name string, r unsafe.Pointer, key string) uint32 {
	k := str(key)
	switch name {
	case "keyed_by":
		return uint32(C.gt_core_guatiao_registry_keyed_by(reg(r), k))
	case "libraries_keyed_by":
		return uint32(C.gt_core_guatiao_registry_libraries_keyed_by(reg(r), k))
	case "retire":
		return uint32(C.gt_core_guatiao_registry_retire(reg(r), k))
	case "unload":
		return uint32(C.gt_core_guatiao_registry_unload(reg(r), k))
	case "unload_unchecked":
		return uint32(C.gt_core_guatiao_registry_unload_unchecked(reg(r), k))
	}
	return ErrNotFound
}

// RegistryAnswer runs one of the calls answering a value through
// (alloc, out): name is the export's name without its prefix, and text
// its one text argument, when it has one.
func RegistryAnswer(name string, r unsafe.Pointer, text string, alloc, out unsafe.Pointer) uint32 {
	t := str(text)
	switch name {
	case "load_file":
		return uint32(C.gt_core_guatiao_registry_load_file(reg(r), t, a(alloc), n(out)))
	case "libraries":
		return uint32(C.gt_core_guatiao_registry_libraries(reg(r), a(alloc), n(out)))
	case "providers":
		return uint32(C.gt_core_guatiao_registry_providers(reg(r), t, a(alloc), n(out)))
	case "provider":
		return uint32(C.gt_core_guatiao_registry_provider(reg(r), t, a(alloc), n(out)))
	case "provider_meta":
		return uint32(C.gt_core_guatiao_registry_provider_meta(reg(r), t, a(alloc), n(out)))
	case "available":
		return uint32(C.gt_core_guatiao_registry_available(reg(r), t, a(alloc), n(out)))
	case "why_not":
		return uint32(C.gt_core_guatiao_registry_why_not(reg(r), t, a(alloc), n(out)))
	case "best":
		return uint32(C.gt_core_guatiao_registry_best(reg(r), t, a(alloc), n(out)))
	case "collect":
		return uint32(C.gt_core_guatiao_registry_collect(reg(r), a(alloc), n(out)))
	case "refresh":
		return uint32(C.gt_core_guatiao_registry_refresh(reg(r), a(alloc), n(out)))
	}
	return ErrNotFound
}

// RegistryRegisterEntry registers a library the host links.
func RegistryRegisterEntry(r unsafe.Pointer, name string, entry, alloc, out unsafe.Pointer) uint32 {
	fn := (*[0]byte)(entry)
	return uint32(C.gt_core_guatiao_registry_register_entry(reg(r), str(name), fn, a(alloc), n(out)))
}

// RegistryScan scans a directory (rules empty: every library), or walks
// a search path when path is set.
func RegistryScan(r unsafe.Pointer, where string, descending bool, rules string, path bool, alloc, out unsafe.Pointer) uint32 {
	if path {
		return uint32(C.gt_core_guatiao_registry_scan_path(reg(r), str(where), C.bool(descending), str(rules), a(alloc), n(out)))
	}
	if rules == "" {
		return uint32(C.gt_core_guatiao_registry_scan_dir(reg(r), str(where), C.bool(descending), a(alloc), n(out)))
	}
	return uint32(C.gt_core_guatiao_registry_scan_dir_rules(reg(r), str(where), C.bool(descending), str(rules), a(alloc), n(out)))
}

// RegistryProviderAvailable is whether the provider can run here, and
// when not, its reason.
func RegistryProviderAvailable(r unsafe.Pointer, key string) (bool, string) {
	var reason C.guatiao_str
	ok := bool(C.gt_core_guatiao_registry_provider_available(reg(r), str(key), &reason))
	if ok {
		return true, ""
	}
	return false, Text(unsafe.Pointer(reason.ptr), uintptr(reason.len))
}

func RegistrySetPriority(r unsafe.Pointer, id string, p int32) uint32 {
	return uint32(C.gt_core_guatiao_registry_set_priority(reg(r), str(id), C.int32_t(p)))
}

func RegistryPriority(r unsafe.Pointer, id string) int32 {
	return int32(C.gt_core_guatiao_registry_priority(reg(r), str(id)))
}

// RegistryTable is the provider's table for kind (its single one when
// kind is empty and single), its size, and its ctx.
func RegistryTable(r unsafe.Pointer, key, kind string, single bool) (unsafe.Pointer, uintptr, uintptr) {
	var size C.size_t
	var table unsafe.Pointer
	if single {
		table = unsafe.Pointer(C.gt_core_guatiao_registry_provider_vtable(reg(r), str(key), &size))
	} else {
		table = unsafe.Pointer(C.gt_core_guatiao_registry_provider_table(reg(r), str(key), str(kind), &size))
	}
	ctx := uintptr(C.gt_ctx(reg(r), str(key)))
	return table, uintptr(size), ctx
}

// RegistryConfig is the provider's configuration schema node, or nil.
func RegistryConfig(r unsafe.Pointer, key string) unsafe.Pointer {
	return unsafe.Pointer(C.gt_core_guatiao_registry_provider_config(reg(r), str(key)))
}

// RegistryCreate builds an instance; err is a provider error slot.
func RegistryCreate(r unsafe.Pointer, key string, config, err unsafe.Pointer) (uintptr, uint32) {
	var out C.uintptr_t
	st := uint32(C.gt_create(reg(r), str(key), n(config), &out, (*C.guatiao_provider_error)(err)))
	return uintptr(out), st
}

// RegistryDestroy releases an instance RegistryCreate built.
func RegistryDestroy(r unsafe.Pointer, key string, instance uintptr) {
	C.gt_destroy(reg(r), str(key), C.uintptr_t(instance))
}

// RegistryAddSourceFile maps a source file and adds what it answers.
func RegistryAddSourceFile(r unsafe.Pointer, path string, config, err unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_registry_add_source_file(reg(r), str(path), n(config),
		(*C.guatiao_provider_error)(err)))
}

func RegistryWatchDir(r unsafe.Pointer, dir, rules string) uint32 {
	return uint32(C.gt_core_guatiao_registry_watch_dir(reg(r), str(dir), str(rules)))
}

// ---- provider errors, objects and tables ------------------------------------------

// TakeProviderError reads a guatiao_provider_error's message, frees it and
// leaves the slot empty.
func TakeProviderError(err unsafe.Pointer) string {
	if err == nil {
		return ""
	}
	e := (*C.guatiao_provider_error)(err)
	msg := unsafe.Pointer(&e.message)
	text := Text(OwnedPtr(msg), OwnedLen(msg))
	FreeString(msg)
	Zero(msg, SizeString)
	return text
}

// WriteProviderError fills a guatiao_provider_error with status and an
// owned copy of message, built in the core library's allocator.
func WriteProviderError(err unsafe.Pointer, status uint32, message string) {
	if err == nil {
		return
	}
	e := (*C.guatiao_provider_error)(err)
	e.status = C.guatiao_status(status)
	msg := unsafe.Pointer(&e.message)
	Zero(msg, SizeString)
	if message == "" || coreLib == nil {
		return
	}
	node := Calloc(SizeValue)
	defer Free(node)
	if ValueString(coreLib.Alloc(), message, node) == OK {
		Copy(msg, Payload(node), SizeString)
	}
}

// FreeString frees an owned guatiao_string at p through what it records.
func FreeString(p unsafe.Pointer) {
	if OwnedCap(p) == 0 {
		return
	}
	node := Calloc(SizeValue)
	defer Free(node)
	SetTag(node, TagString)
	Copy(Payload(node), p, SizeString)
	ValueFree(node)
}

// ObjectAt reads a guatiao_object.
func ObjectAt(p unsafe.Pointer) (unsafe.Pointer, uintptr, uintptr) {
	o := (*C.guatiao_object)(p)
	return unsafe.Pointer(o.table), uintptr(o.size), *(*uintptr)(unsafe.Pointer(&o.ctx))
}

// SetObject writes a guatiao_object.
func SetObject(p, table unsafe.Pointer, size, ctx uintptr) {
	o := (*C.guatiao_object)(p)
	o.table = table
	o.size = C.size_t(size)
	*(*uintptr)(unsafe.Pointer(&o.ctx)) = ctx
}

// TableHeader reads a kind table's header.
func TableHeader(table unsafe.Pointer) (size, floorHash uint32) {
	h := (*C.guatiao_kind_header)(table)
	return uint32(h.struct_size), uint32(h.floor_hash)
}

// SlotAt is the function pointer at offset in a table.
func SlotAt(table unsafe.Pointer, offset uintptr) unsafe.Pointer {
	return *(*unsafe.Pointer)(unsafe.Add(table, offset))
}

// NewTable builds a kind table in C memory kept for the process: the
// header, then each slot pointer (nil for a slot not served).
func NewTable(floorHash uint32, slots []unsafe.Pointer) unsafe.Pointer {
	size := SizeKindHeader + uintptr(len(slots))*Word
	t := Calloc(size)
	h := (*C.guatiao_kind_header)(t)
	h.struct_size = C.uint32_t(size)
	h.floor_hash = C.uint32_t(floorHash)
	for i, s := range slots {
		*(*unsafe.Pointer)(unsafe.Add(t, SizeKindHeader+uintptr(i)*Word)) = s
	}
	return t
}

// CallDestroy calls an object's destroy slot with its ctx.
func CallDestroy(fn unsafe.Pointer, ctx uintptr) { C.gt_call_destroy(fn, C.uintptr_t(ctx)) }

// SetUintptr writes v at p, as an out slot of a pointer type receives a
// ctx that is a number shaped like an address.
func SetUintptr(p unsafe.Pointer, v uintptr) { *(*uintptr)(p) = v }

// Uintptr reads a pointer-sized word at p as a number.
func Uintptr(p unsafe.Pointer) uintptr { return *(*uintptr)(p) }

// CopyBytesIn copies a Go string into C memory kept for the process,
// answering the pointer (nil for empty) and the length.
func CopyBytesIn(s string) (unsafe.Pointer, uintptr) {
	if s == "" {
		return nil, 0
	}
	return C.CBytes([]byte(s)), uintptr(len(s))
}
