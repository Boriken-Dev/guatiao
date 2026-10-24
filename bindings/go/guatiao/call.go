// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Call is one call through a table, as guatiao-kind's generated remotes
// make it: Begin, an argument method per argument (handed-over ones
// last), the C call, Status, the answer, End. It keeps what the call
// needs alive and frees it at End. Nothing else needs to use it.
type Call struct {
	r      *remoteState
	slot   *SlotSpec
	fn     unsafe.Pointer
	blocks []unsafe.Pointer
	after  []func()
	given  []func()
	called bool
	err    unsafe.Pointer
}

// Begin starts a call to slot i: ErrUnsupported for a slot the table
// lacks (past its size, or null), ErrClosed for a remote that is.
func (r Remote) Begin(i int) (*Call, error) {
	if err := r.check(); err != nil {
		return nil, err
	}
	slot := &r.r.kind.spec.Slots[i]
	fn := r.slotFn(i)
	if fn == nil {
		return nil, &Error{Status: Unsupported, Message: fmt.Sprintf("this %s table has no %s", r.r.kind.spec.Name, slot.Name)}
	}
	return &Call{r: r.r, slot: slot, fn: fn}, nil
}

// Fn is the slot's function.
func (c *Call) Fn() unsafe.Pointer { return c.fn }

// Ctx is the ctx the slot takes.
func (c *Call) Ctx() uintptr { return c.r.ctx }

func (c *Call) block(n uintptr) unsafe.Pointer {
	p := native.Calloc(n)
	c.blocks = append(c.blocks, p)
	return p
}

// Str is text for a guatiao_str argument: the Go string's own bytes, for
// the call.
func (c *Call) Str(s string) (unsafe.Pointer, uintptr) {
	if len(s) == 0 {
		return nil, 0
	}
	return unsafe.Pointer(unsafe.StringData(s)), uintptr(len(s))
}

// Bytes is a byte slice for a guatiao_bytes or guatiao_bytes_mut
// argument: the slice's own memory, which a mutable one's callee writes
// into for the call.
func (c *Call) Bytes(b []byte) (unsafe.Pointer, uintptr) {
	if len(b) == 0 {
		return nil, 0
	}
	return unsafe.Pointer(unsafe.SliceData(b)), uintptr(len(b))
}

// arm is the struct that crosses for class: the node for a value, its
// payload for a map or a list.
func arm(node unsafe.Pointer, class TreeClass) (unsafe.Pointer, error) {
	if class == ValueClass {
		return node, nil
	}
	if native.Tag(node) != class.tag() {
		return nil, &Error{Status: WrongKind, Message: fmt.Sprintf("expected a %s, got a %s", class, Tag(native.Tag(node)))}
	}
	return native.Payload(node), nil
}

// Borrow is x for a borrowed tree argument (`const guatiao_value *` and
// the like): a view as it is, plain data built for the call; nil x is a
// null value.
func (c *Call) Borrow(x any, class TreeClass) (unsafe.Pointer, error) {
	if x == nil && class == ValueClass {
		return nil, nil
	}
	if r, ok := asRef(x); ok {
		if r.o == nil && class == ValueClass {
			return nil, nil
		}
		node, err := r.requireNode()
		if err != nil {
			return nil, err
		}
		return arm(node, class)
	}
	v, err := NewValue(x)
	if err != nil {
		return nil, err
	}
	c.after = append(c.after, func() { v.Close() })
	return arm(v.o.node, class)
}

// Lend is x, a Value or a writable view, for a tree lent to the callee:
// changed where it stands.
func (c *Call) Lend(x any, class TreeClass) (unsafe.Pointer, error) {
	r, ok := asRef(x)
	if !ok {
		return nil, &Error{Status: WrongKind, Message: fmt.Sprintf("a lent %s is a *guatiao.Value or a view, so what the callee changes has somewhere to go; not a %T", class, x)}
	}
	node, alloc, err := r.writable()
	if err != nil {
		return nil, err
	}
	// What the callee grew it with may be in its library's allocator.
	c.after = append(c.after, func() {
		home := native.Recorded(node)
		if home == nil {
			home = alloc
		}
		native.ValueMakeLocal(node, home)
	})
	return arm(node, class)
}

// Take is x for a tree handed over: a *Value is moved (left absent), a
// view copied, plain data built; freed if the call never happens.
func (c *Call) Take(x any, class TreeClass) (unsafe.Pointer, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return nil, err
	}
	var node unsafe.Pointer
	if v, ok := x.(*Value); ok && v != nil {
		if class != ValueClass && v.Tag() != Tag(class.tag()) {
			return nil, &Error{Status: WrongKind, Message: fmt.Sprintf("expected a %s, got a %s", class, v.Tag())}
		}
		if node, err = v.takeRoot(); err != nil {
			return nil, err
		}
	} else {
		node = native.Calloc(native.SizeValue)
		if err := writeGo(node, x, alloc); err != nil {
			native.Free(node)
			return nil, err
		}
	}
	c.blocks = append(c.blocks, node)
	c.given = append(c.given, func() { native.ValueFree(node) })
	return arm(node, class)
}

// LendObject is x, a Go value implementing the object kind k or a remote
// of it, lent for the call: a pointer to a guatiao_object on this side.
func (c *Call) LendObject(x any, k AnyKind) (unsafe.Pointer, error) {
	d := k.desc()
	var raw rawObject
	if rc, ok := x.(remoteCarrier); ok {
		r := rc.guatiaoRemote()
		if err := r.check(); err != nil {
			return nil, err
		}
		if r.r.kind != d {
			return nil, &Error{Status: WrongKind, Message: fmt.Sprintf("an object of %s where %s was asked for", r.r.kind.spec.Name, d.spec.Name)}
		}
		raw = rawObject{table: r.r.table, size: r.r.size, ctx: r.r.ctx}
	} else {
		served, err := serveObject(d, x)
		if err != nil {
			return nil, err
		}
		raw = served
		c.after = append(c.after, func() { release(served.ctx) })
	}
	at := c.block(native.SizeObject)
	native.SetObject(at, raw.table, raw.size, raw.ctx)
	return at, nil
}

// TakeObject is x, a Go value implementing the object kind k or a remote
// of it this side owns, handed over: the callee destroys it.
func (c *Call) TakeObject(x any, k AnyKind) (unsafe.Pointer, uintptr, uintptr, error) {
	d := k.desc()
	var raw rawObject
	if rc, ok := x.(remoteCarrier); ok {
		r := rc.guatiaoRemote()
		if r.r != nil && r.r.kind != d {
			return nil, 0, 0, &Error{Status: WrongKind, Message: fmt.Sprintf("an object of %s where %s was asked for", r.r.kind.spec.Name, d.spec.Name)}
		}
		given, err := r.giveUp()
		if err != nil {
			return nil, 0, 0, err
		}
		raw = given
	} else {
		served, err := serveObject(d, x)
		if err != nil {
			return nil, 0, 0, err
		}
		raw = served
	}
	c.given = append(c.given, func() { raw.destroy(d) })
	return raw.table, raw.size, raw.ctx, nil
}

// Out is the out slot of the call's answer: for a tree a carrier naming
// this binding's allocator, for text an empty string, for a scalar eight
// bytes, for an object an empty guatiao_object.
func (c *Call) Out(class TreeClass) unsafe.Pointer {
	alloc, _ := coreAlloc()
	if class == ValueClass {
		out := c.block(native.SizeValue)
		native.SetOwnedAlloc(native.Payload(out), alloc)
		return out
	}
	out := c.block(native.SizeMap)
	native.SetOwnedAlloc(out, alloc)
	return out
}

// OutText is a guatiao_string out slot.
func (c *Call) OutText() unsafe.Pointer { return c.block(native.SizeString) }

// OutScalar is an out slot for a scalar answer.
func (c *Call) OutScalar() unsafe.Pointer { return c.block(8) }

// OutObject is a guatiao_object out slot.
func (c *Call) OutObject() unsafe.Pointer { return c.block(native.SizeObject) }

// Err is the slot's guatiao_provider_error.
func (c *Call) Err() unsafe.Pointer {
	c.err = c.block(native.SizeProviderErr)
	return c.err
}

// Status is the call's status as an error: the provider's words for a
// slot that can refuse, the status alone for one that cannot. The call
// has happened: what was handed over is the callee's.
func (c *Call) Status(st uint32) error {
	c.called = true
	if st == native.OK {
		return nil
	}
	if c.err != nil {
		return &Error{Status: Status(st), Message: native.TakeProviderError(c.err)}
	}
	return &Error{Status: Status(st), Message: c.slot.Name + " failed, which its kind says it cannot"}
}

// Answer reads the tree the call answered into dst: a **Value takes it
// over, anything else decodes it and frees it.
func (c *Call) Answer(out unsafe.Pointer, class TreeClass, dst any) error {
	alloc, _ := coreAlloc()
	node := native.Calloc(native.SizeValue)
	if class == ValueClass {
		native.Copy(node, out, native.SizeValue)
	} else {
		native.SetTag(node, class.tag())
		native.Copy(native.Payload(node), out, native.SizeMap)
	}
	v := adopt(node, alloc)
	if p, ok := dst.(**Value); ok {
		// A slot that ignored the carrier built it in its library's
		// allocator; kept, it is moved into this binding's.
		if err := check(native.ValueMakeLocal(node, alloc), "moving the answer into this binding's allocator"); err != nil {
			v.Close()
			return err
		}
		*p = v
		return nil
	}
	defer v.Close()
	return v.Decode(dst)
}

// AnswerText reads and frees the text the call answered.
func (c *Call) AnswerText(out unsafe.Pointer) string {
	text := native.Text(native.OwnedPtr(out), native.OwnedLen(out))
	native.FreeString(out)
	return text
}

// AnswerObject is the object the call answered, which this side now owns.
func (c *Call) AnswerObject(out unsafe.Pointer, k AnyKind) (Remote, error) {
	table, size, ctx := native.ObjectAt(out)
	r, err := newRemote(k.desc(), table, size, ctx, true, nil)
	if err != nil {
		// Not one this side can call, but it is still this side's to destroy.
		rawObject{table: table, size: size, ctx: ctx}.destroy(k.desc())
	}
	return r, err
}

// End finishes the call: what runs after it, the hand-overs it never
// made undone, every block freed.
func (c *Call) End() {
	for i := len(c.after) - 1; i >= 0; i-- {
		c.after[i]()
	}
	if !c.called {
		for i := len(c.given) - 1; i >= 0; i-- {
			c.given[i]()
		}
	}
	for _, b := range c.blocks {
		native.Free(b)
	}
	c.blocks, c.after, c.given = nil, nil, nil
}
