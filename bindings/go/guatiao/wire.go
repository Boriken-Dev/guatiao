// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"runtime"
	"sync"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// ---- one value as bytes ---------------------------------------------------

// ToWire is the node in the wire encoding: a tag byte per node, LEB128
// lengths, a number as its own text, bytes as bytes, a map in its order.
// FromWire, in any language, reads the same tree back.
func (r Ref) ToWire() ([]byte, error) {
	node, err := r.requireNode()
	if err != nil {
		return nil, err
	}
	lib, err := core()
	if err != nil {
		return nil, err
	}
	return bytesOut(func(out unsafe.Pointer) uint32 { return native.WireEncode(lib.Alloc(), node, out) })
}

// FromWire is the value the wire encoding in data holds, as a Value to
// close. ErrBadValue for bytes that are not exactly one value: truncated,
// an unknown tag, a length not in its shortest form, text that is not
// UTF-8, a number outside the grammar, a repeated key, anything after the
// value.
func FromWire(data []byte, opts ...Option) (*Value, error) {
	if _, err := core(); err != nil {
		return nil, err
	}
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	v := newOwned(o.alloc)
	alloc, err := v.o.allocator()
	if err == nil {
		err = check(native.WireDecode(alloc, data, v.o.node), "bytes that are not a value in the wire encoding")
	}
	if err != nil {
		v.Close()
		return nil, err
	}
	return v, nil
}

// bytesOut runs call with a scratch node it writes a BYTES value into,
// and answers a copy of the bytes.
func bytesOut(call func(out unsafe.Pointer) uint32) ([]byte, error) {
	out := native.Calloc(native.SizeValue)
	defer freeRoot(out)
	if err := check(call(out), ""); err != nil {
		return nil, err
	}
	at := native.Payload(out)
	return native.Bytes(native.OwnedPtr(at), native.OwnedLen(at)), nil
}

// ---- channels -------------------------------------------------------------

// Frame is a channel frame's kind byte: GUATIAO_FRAME_*.
type Frame uint32

// The frame kinds.
const (
	FrameSchema Frame = Frame(native.FrameSchema)
	FrameValue  Frame = Frame(native.FrameValue)
	FrameClose  Frame = Frame(native.FrameClose)
)

func (f Frame) String() string {
	switch f {
	case FrameSchema:
		return "schema"
	case FrameValue:
		return "value"
	case FrameClose:
		return "close"
	}
	return fmt.Sprintf("frame %d", uint32(f))
}

// WireAnnounce is the frame that announces schema (a Value, a view or
// plain data) on channel. A later one on the same channel replaces it.
func WireAnnounce(channel uint64, schema any) ([]byte, error) {
	return frameOf(schema, func(alloc, node, out unsafe.Pointer) uint32 {
		return native.WireAnnounce(alloc, channel, node, out)
	})
}

// WireSend is the frame that carries value (a Value, a view or plain
// data) on channel.
func WireSend(channel uint64, value any) ([]byte, error) {
	return frameOf(value, func(alloc, node, out unsafe.Pointer) uint32 {
		return native.WireSend(alloc, channel, node, out)
	})
}

// WireClose is the frame that ends channel: a receiver forgets its schema.
func WireClose(channel uint64) ([]byte, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	return bytesOut(func(out unsafe.Pointer) uint32 { return native.WireClose(lib.Alloc(), channel, out) })
}

func frameOf(x any, build func(alloc, node, out unsafe.Pointer) uint32) ([]byte, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	var frame []byte
	err = withNodes([]any{x}, func(nodes []unsafe.Pointer) error {
		var err error
		frame, err = bytesOut(func(out unsafe.Pointer) uint32 { return build(lib.Alloc(), nodes[0], out) })
		return err
	})
	return frame, err
}

// Event is what a frame said. Value is the schema now the channel's, or
// the value its schema accepted, as a Value the caller closes; nil for a
// channel's end.
type Event struct {
	Kind    Frame
	Channel uint64
	Value   *Value
}

// Refusal says why a Receiver refused a frame: GUATIAO_WIRE_*.
type Refusal uint32

// The refusals.
const (
	// RefusalMalformed: bytes that are not a frame. Status BadValue.
	RefusalMalformed Refusal = Refusal(native.WireMalformed)
	// RefusalNoSchema: a value on a channel with no schema. Status NotFound.
	RefusalNoSchema Refusal = Refusal(native.WireNoSchema)
	// RefusalNotASchema: a schema frame that is not a schema. Status WrongKind.
	RefusalNotASchema Refusal = Refusal(native.WireNotASchema)
	// RefusalInvalid: a value its channel's schema refuses. Status BadValue.
	RefusalInvalid Refusal = Refusal(native.WireInvalid)
)

func (r Refusal) String() string {
	switch r {
	case RefusalMalformed:
		return "malformed"
	case RefusalNoSchema:
		return "no schema"
	case RefusalNotASchema:
		return "not a schema"
	case RefusalInvalid:
		return "invalid"
	}
	return fmt.Sprintf("refusal %d", uint32(r))
}

// Refused is a frame a Receiver refused. Channel is the channel the frame
// named (0 when it ended before one) and Kind its kind byte; At is the
// byte of the frame a malformed one went wrong at; for RefusalInvalid,
// Key is the field and Expected what it accepts or Known the fields there
// are. Message is the refusal's words; nothing the frame carried is
// quoted. errors.Is matches the sentinel of its status (ErrBadValue,
// ErrNotFound, ErrWrongKind), and errors.As finds it as an *Error too.
type Refused struct {
	Refusal  Refusal
	Status   Status
	Channel  uint64
	Kind     Frame
	At       int
	Key      string
	Expected string
	Known    []string
	Message  string
}

func (e *Refused) Error() string {
	return "guatiao: " + e.Status.String() + ": " + e.Message
}

// Unwrap is the refusal as the *Error of its status.
func (e *Refused) Unwrap() error { return &Error{Status: e.Status, Message: e.Message} }

// receiverHandle is the C receiver, freed once.
type receiverHandle struct {
	mu      sync.Mutex
	handle  unsafe.Pointer
	cleanup runtime.Cleanup
}

// Receiver keeps each channel's schema and checks every value against it.
// Close frees it; one dropped unclosed is freed when the garbage collector
// finds it. Accept may be called from several goroutines: calls are
// serialised.
type Receiver struct {
	h *receiverHandle
}

// NewReceiver is a receiver building in the core library's allocator.
func NewReceiver() (*Receiver, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	handle := native.ReceiverNew(lib.Alloc())
	if handle == nil {
		return nil, &Error{Status: AllocFailed, Message: "the allocator was refused"}
	}
	h := &receiverHandle{handle: handle}
	h.cleanup = runtime.AddCleanup(h, native.ReceiverFree, handle)
	return &Receiver{h: h}, nil
}

// Accept reads one frame: an Event, or a *Refused saying why not. A
// refusal changes nothing the receiver keeps.
func (r *Receiver) Accept(frame []byte) (Event, error) {
	lib, err := core()
	if err != nil {
		return Event{}, err
	}
	r.h.mu.Lock()
	if r.h.handle == nil {
		r.h.mu.Unlock()
		return Event{}, ErrClosed
	}
	event := native.Calloc(native.SizeWireEvent)
	defer native.Free(event)
	status := native.ReceiverAccept(r.h.handle, frame, event)
	r.h.mu.Unlock()

	value := unsafe.Add(event, native.OffsetEventValue)
	message := unsafe.Add(event, native.OffsetEventMessage)
	detail := unsafe.Add(event, native.OffsetEventDetail)
	defer func() {
		native.ValueFree(value)
		native.ValueFree(message)
		native.ValueFree(detail)
	}()
	words := ""
	if native.Tag(message) == native.TagString {
		words = native.TextOf(message)
	}
	kind := Frame(native.EventKind(event))
	channel := native.EventChannel(event)
	if status == native.OK {
		out := Event{Kind: kind, Channel: channel}
		if native.Tag(value) != native.TagAbsent {
			moved := native.Calloc(native.SizeValue)
			native.Copy(moved, value, native.SizeValue)
			native.Zero(value, native.SizeValue)
			out.Value = adopt(moved, lib.Alloc())
		}
		return out, nil
	}
	refusal := Refusal(native.EventRefusal(event))
	if refusal == Refusal(native.WireAccepted) {
		return Event{}, check(status, words)
	}
	e := &Refused{
		Refusal: refusal,
		Status:  Status(status),
		Channel: channel,
		Kind:    kind,
		At:      int(native.EventAt(event)),
		Message: words,
	}
	if refusal == RefusalInvalid {
		e.Key = textAt(native.Find(detail, "key"))
		e.Expected = textAt(native.Find(detail, "expected"))
		if known := native.Find(detail, "known"); native.Tag(known) == native.TagList {
			for i := uintptr(0); i < native.Len(known); i++ {
				e.Known = append(e.Known, textAt(native.ListElem(known, i)))
			}
		}
	}
	return Event{}, e
}

// textAt is a string node's text, "" for anything else.
func textAt(node unsafe.Pointer) string {
	if native.Tag(node) != native.TagString {
		return ""
	}
	return native.TextOf(node)
}

// Close frees the receiver and the schemas it keeps; a second call does
// nothing. It always answers nil.
func (r *Receiver) Close() error {
	if r == nil || r.h == nil {
		return nil
	}
	r.h.mu.Lock()
	defer r.h.mu.Unlock()
	if r.h.handle != nil {
		r.h.cleanup.Stop()
		native.ReceiverFree(r.h.handle)
		r.h.handle = nil
	}
	return nil
}
