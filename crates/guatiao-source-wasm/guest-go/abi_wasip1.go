// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//go:build wasip1

package guest

import "unsafe"

// guatiao ABI 1: the host's imports.

//go:wasmimport guatiao answer
func importAnswer(ptr unsafe.Pointer, n uint32)

//go:wasmimport guatiao write_back
func importWriteBack(index uint32, ptr unsafe.Pointer, n uint32) uint32

//go:wasmimport guatiao object_call
func importObjectCall(handle int64, ptr unsafe.Pointer, n uint32) uint32

//go:wasmimport guatiao result
func importResult(ptr unsafe.Pointer, capacity uint32) uint32

//go:wasmimport guatiao drop
func importDrop(handle int64)

// at is where b's bytes are, nil for none: the host reads len(b) of them.
func at(b []byte) unsafe.Pointer {
	if len(b) == 0 {
		return nil
	}
	return unsafe.Pointer(&b[0])
}

func init() {
	hostAnswer = func(b []byte) { importAnswer(at(b), uint32(len(b))) }
	hostWriteBack = func(i uint32, b []byte) uint32 { return importWriteBack(i, at(b), uint32(len(b))) }
	hostObjectCall = func(handle int64, request []byte) (uint32, []byte) {
		status := importObjectCall(handle, at(request), uint32(len(request)))
		result := make([]byte, importResult(nil, 0))
		importResult(at(result), uint32(len(result)))
		return status, result
	}
	hostDrop = importDrop
}

// Requests the host wrote, kept reachable until their call takes them: the
// collector does not move what it keeps.
var requests = map[uint32][]byte{}

// guatiao ABI 1: the exports.

//go:wasmexport guatiao_guest_abi
func exportABI() int32 { return 1 }

//go:wasmexport guatiao_guest_alloc
func exportAlloc(n int32) int32 {
	b := make([]byte, n+1)
	address := uint32(uintptr(unsafe.Pointer(&b[0])))
	requests[address] = b[:n]
	return int32(address)
}

//go:wasmexport guatiao_guest_declare
func exportDeclare() int32 { return int32(declare()) }

//go:wasmexport guatiao_guest_call
func exportCall(object int32, target int64, request int32, n int32) int32 {
	address := uint32(request)
	b, ok := requests[address]
	delete(requests, address)
	if !ok || int32(len(b)) != n {
		return int32(refuse(Refuse(StatusBadValue, "no request at %d", address)))
	}
	return int32(call(object != 0, target, b))
}

//go:wasmexport guatiao_guest_release
func exportRelease(object int64) { release(object) }
