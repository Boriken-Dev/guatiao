// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "native.h"
*/
import "C"

import "unsafe"

// KindTable is one kind's table on a provider.
type KindTable struct {
	Kind  string
	Table unsafe.Pointer
	Size  uintptr
}

// Provider is what a provider's descriptor says, as Go data.
type Provider struct {
	ID, DisplayName, Version string
	Kinds                    []string
	Tables                   []KindTable
	// Config is the configuration schema's node, or nil.
	Config unsafe.Pointer
	// Meta is a map node, or nil.
	Meta unsafe.Pointer
	// Ctx is the provider's own instance, or 0 for none.
	Ctx uintptr
	// Pool is the index of its pooled slots.
	Pool                  int
	Builds, AsksAvailable bool
}

func setStr(s *C.guatiao_str, text string) {
	p, n := CopyBytesIn(text)
	s.ptr = (*C.uint8_t)(p)
	s.len = C.size_t(n)
}

// mapPtr is the payload of a map node, which a descriptor's meta points at.
func mapPtr(node unsafe.Pointer) C.guatiao_map_ptr {
	if node == nil {
		return nil
	}
	return C.guatiao_map_ptr(Payload(node))
}

// NewLibraryInfo builds a library descriptor in C memory kept for the
// process: every string copied, each provider's arrays allocated, the
// pooled slots named by each provider's Pool, and unload as given.
func NewLibraryInfo(id, version string, providers []Provider, meta, unload unsafe.Pointer) unsafe.Pointer {
	size := unsafe.Sizeof(C.guatiao_provider_info{})
	infos := Calloc(size * uintptr(max(1, len(providers))))
	for i, p := range providers {
		info := (*C.guatiao_provider_info)(unsafe.Add(infos, uintptr(i)*size))
		info.struct_size = C.uint32_t(size)
		names := Calloc(SizeStr * uintptr(max(1, len(p.Kinds))))
		for j, k := range p.Kinds {
			setStr((*C.guatiao_str)(unsafe.Add(names, uintptr(j)*SizeStr)), k)
		}
		info.kinds.ptr = (*C.guatiao_str)(names)
		info.kinds.len = C.size_t(len(p.Kinds))
		setStr(&info.id, p.ID)
		setStr(&info.display_name, p.DisplayName)
		setStr(&info.version, p.Version)
		info.config = (*C.guatiao_value)(p.Config)
		info.meta = mapPtr(p.Meta)
		*(*uintptr)(unsafe.Pointer(&info.ctx)) = p.Ctx
		tsize := unsafe.Sizeof(C.guatiao_kind_table{})
		tables := Calloc(tsize * uintptr(max(1, len(p.Tables))))
		for j, t := range p.Tables {
			kt := (*C.guatiao_kind_table)(unsafe.Add(tables, uintptr(j)*tsize))
			kt.struct_size = C.uint32_t(tsize)
			kt.vtable_size = C.uint32_t(t.Size)
			setStr(&kt.kind, t.Kind)
			kt.vtable = t.Table
		}
		info.tables.ptr = (*C.guatiao_kind_table)(tables)
		info.tables.len = C.size_t(len(p.Tables))
		info.tables.stride = C.size_t(tsize)
		if p.AsksAvailable {
			info.available = (*[0]byte)(PoolAvailable(p.Pool))
		}
		if p.Builds {
			info.create = (*[0]byte)(PoolCreate(p.Pool))
			info.destroy = (*[0]byte)(PoolDestroy(p.Pool))
		}
	}
	lib := (*C.guatiao_library_info)(Calloc(unsafe.Sizeof(C.guatiao_library_info{})))
	lib.struct_size = C.uint32_t(unsafe.Sizeof(C.guatiao_library_info{}))
	lib.abi_version = C.GUATIAO_ABI_VERSION
	setStr(&lib.id, id)
	setStr(&lib.version, version)
	lib.providers.ptr = (*C.guatiao_provider_info)(infos)
	lib.providers.len = C.size_t(len(providers))
	lib.providers.stride = C.size_t(size)
	lib.meta = mapPtr(meta)
	lib.unload = (*[0]byte)(unload)
	return unsafe.Pointer(lib)
}

// SetReason writes a borrowed guatiao_str through reason, as `available`
// answers why not: text must outlive every reader.
func SetReason(reason, text unsafe.Pointer, n uintptr) {
	if reason == nil {
		return
	}
	s := (*C.guatiao_str)(reason)
	s.ptr = (*C.uint8_t)(text)
	s.len = C.size_t(n)
}

// HostAlloc is the allocator a host block names, or nil.
func HostAlloc(host unsafe.Pointer) unsafe.Pointer {
	if host == nil {
		return nil
	}
	h := (*C.guatiao_host_info)(host)
	if uintptr(h.struct_size) < unsafe.Offsetof(C.guatiao_host_info{}.alloc)+Word {
		return nil
	}
	return unsafe.Pointer(h.alloc)
}
