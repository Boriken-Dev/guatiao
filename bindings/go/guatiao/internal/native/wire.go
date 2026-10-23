// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "native.h"
*/
import "C"

import "unsafe"

// The wire event's layout and codes, from the header.
const (
	SizeWireEvent      = unsafe.Sizeof(C.guatiao_wire_event{})
	OffsetEventValue   = unsafe.Offsetof(C.guatiao_wire_event{}.value)
	OffsetEventMessage = unsafe.Offsetof(C.guatiao_wire_event{}.message)
	OffsetEventDetail  = unsafe.Offsetof(C.guatiao_wire_event{}.detail)

	FrameSchema = uint32(C.GUATIAO_FRAME_SCHEMA)
	FrameValue  = uint32(C.GUATIAO_FRAME_VALUE)
	FrameClose  = uint32(C.GUATIAO_FRAME_CLOSE)

	WireAccepted   = uint32(C.GUATIAO_WIRE_ACCEPTED)
	WireMalformed  = uint32(C.GUATIAO_WIRE_MALFORMED)
	WireNoSchema   = uint32(C.GUATIAO_WIRE_NO_SCHEMA)
	WireNotASchema = uint32(C.GUATIAO_WIRE_NOT_A_SCHEMA)
	WireInvalid    = uint32(C.GUATIAO_WIRE_INVALID)
)

func WireEncode(alloc, value, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_encode(a(alloc), n(value), n(out)))
}
func WireDecode(alloc unsafe.Pointer, data []byte, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_decode(a(alloc), bytesArg(data), n(out)))
}
func WireAnnounce(alloc unsafe.Pointer, channel uint64, schema, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_announce(a(alloc), C.uint64_t(channel), n(schema), n(out)))
}
func WireSend(alloc unsafe.Pointer, channel uint64, value, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_send(a(alloc), C.uint64_t(channel), n(value), n(out)))
}
func WireClose(alloc unsafe.Pointer, channel uint64, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_close(a(alloc), C.uint64_t(channel), n(out)))
}

// ReceiverNew is a receiver building in alloc, or nil.
func ReceiverNew(alloc unsafe.Pointer) unsafe.Pointer {
	return unsafe.Pointer(C.gt_core_guatiao_wire_receiver_new(a(alloc)))
}

// ReceiverAccept reads one frame into the event at event (SizeWireEvent
// bytes of C memory).
func ReceiverAccept(recv unsafe.Pointer, frame []byte, event unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_wire_receiver_accept(
		(*C.guatiao_wire_receiver)(recv), bytesArg(frame), (*C.guatiao_wire_event)(event)))
}

// ReceiverFree frees a receiver; nil does nothing.
func ReceiverFree(recv unsafe.Pointer) {
	C.gt_core_guatiao_wire_receiver_free((*C.guatiao_wire_receiver)(recv))
}

func ev(p unsafe.Pointer) *C.guatiao_wire_event { return (*C.guatiao_wire_event)(p) }

// EventChannel, EventKind, EventRefusal and EventAt read an event's
// scalar fields.
func EventChannel(p unsafe.Pointer) uint64 { return uint64(ev(p).channel) }
func EventKind(p unsafe.Pointer) uint32    { return uint32(ev(p).kind) }
func EventRefusal(p unsafe.Pointer) uint32 { return uint32(ev(p).refusal) }
func EventAt(p unsafe.Pointer) uintptr     { return uintptr(ev(p).at) }
