// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "pools.h"
*/
import "C"

import "unsafe"

// What the pooled descriptor slots and the object destroy slot call, set
// by the guatiao package: index first, the C arguments after, each ctx a
// number.
var (
	OnEntry         func(i int, host unsafe.Pointer) unsafe.Pointer
	OnUnload        func(i int) uint32
	OnCreate        func(i int, ctx uintptr, config, out, err unsafe.Pointer) uint32
	OnDestroy       func(i int, ctx, instance uintptr)
	OnAvailable     func(i int, ctx uintptr, reason unsafe.Pointer) bool
	OnDestroyObject func(ctx uintptr)
)

//export gtGoEntry
func gtGoEntry(i C.int, host unsafe.Pointer) unsafe.Pointer {
	if OnEntry == nil {
		return nil
	}
	return OnEntry(int(i), host)
}

//export gtGoUnload
func gtGoUnload(i C.int) C.uint32_t {
	if OnUnload == nil {
		return C.uint32_t(ErrBusy)
	}
	return C.uint32_t(OnUnload(int(i)))
}

//export gtGoCreate
func gtGoCreate(i C.int, ctx C.uintptr_t, config unsafe.Pointer, out *C.uintptr_t, err unsafe.Pointer) C.uint32_t {
	if OnCreate == nil {
		return C.uint32_t(ErrInternal)
	}
	return C.uint32_t(OnCreate(int(i), uintptr(ctx), config, unsafe.Pointer(out), err))
}

//export gtGoDestroy
func gtGoDestroy(i C.int, ctx, instance C.uintptr_t) {
	if OnDestroy != nil {
		OnDestroy(int(i), uintptr(ctx), uintptr(instance))
	}
}

//export gtGoAvailable
func gtGoAvailable(i C.int, ctx C.uintptr_t, reason unsafe.Pointer) C.bool {
	if OnAvailable == nil {
		return true
	}
	return C.bool(OnAvailable(int(i), uintptr(ctx), reason))
}

//export gtGoDestroyObject
func gtGoDestroyObject(ctx C.uintptr_t) {
	if OnDestroyObject != nil {
		OnDestroyObject(uintptr(ctx))
	}
}

// PoolLibraries and PoolProviders are how many libraries and providers a
// process can build.
func PoolLibraries() int { return int(C.gt_pool_libraries()) }
func PoolProviders() int { return int(C.gt_pool_providers()) }

// PoolEntry and the rest are the pooled slots at index i.
func PoolEntry(i int) unsafe.Pointer     { return C.gt_pool_entry(C.int(i)) }
func PoolUnload(i int) unsafe.Pointer    { return C.gt_pool_unload(C.int(i)) }
func PoolCreate(i int) unsafe.Pointer    { return C.gt_pool_create(C.int(i)) }
func PoolDestroy(i int) unsafe.Pointer   { return C.gt_pool_destroy(C.int(i)) }
func PoolAvailable(i int) unsafe.Pointer { return C.gt_pool_available(C.int(i)) }

// ObjectDestroy is the destroy slot of every object table this binding serves.
func ObjectDestroy() unsafe.Pointer { return C.gt_pool_object_destroy() }
