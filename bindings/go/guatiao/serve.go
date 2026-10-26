// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"unicode/utf8"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Served is one call a C caller made into a slot this binding serves, as
// guatiao-kind's generated exports receive it: argument methods (what is
// handed over first), Target, Ran, the method, the answer. What it lends
// the method is valid until the call returns. Nothing else needs to use
// it.
type Served struct {
	kind   *kindDesc
	slot   *SlotSpec
	ctx    uintptr
	open   bool
	ran    bool
	taken  []func()
	after  []func()
	blocks []unsafe.Pointer
}

// Serve runs one call into slot i of the kind: body receives the
// arguments, finds the target and calls its method; what it returns, or
// a panic, becomes the slot's status, with the words written to errp
// when the slot can refuse.
func (k *Kind[R]) Serve(ctx uintptr, errp unsafe.Pointer, i int, body func(s *Served) error) uint32 {
	return serve(k.d, ctx, errp, i, body)
}

func serve(d *kindDesc, ctx uintptr, errp unsafe.Pointer, i int, body func(s *Served) error) (status uint32) {
	s := &Served{kind: d, slot: &d.spec.Slots[i], ctx: ctx, open: true}
	err := s.run(body)
	if !s.ran {
		for _, f := range s.taken {
			f()
		}
	}
	s.done()
	if err == nil {
		return native.OK
	}
	st, message := statusOf(err)
	if s.slot.Fallible {
		native.WriteProviderError(errp, uint32(st), message)
	} else {
		warn(fmt.Errorf("%s.%s cannot fail, and failed: %s", d.spec.Name, s.slot.Name, message))
	}
	return uint32(st)
}

func (s *Served) run(body func(s *Served) error) (err error) {
	defer func() {
		if p := recover(); p != nil {
			err = &Error{Status: Internal, Message: fmt.Sprintf("panic: %v", p)}
		}
	}()
	return body(s)
}

func (s *Served) done() {
	for i := len(s.after) - 1; i >= 0; i-- {
		s.after[i]()
	}
	s.open = false
	for _, b := range s.blocks {
		native.Free(b)
	}
	s.after, s.blocks, s.taken = nil, nil, nil
}

func (s *Served) isOpen() bool { return s.open }

func (s *Served) block(n uintptr) unsafe.Pointer {
	p := native.Calloc(n)
	s.blocks = append(s.blocks, p)
	return p
}

// Target is the Go value the call's ctx names.
func (s *Served) Target() (any, error) { return held(s.ctx) }

// Ran marks the method about to run: from now on it owns what was handed
// over.
func (s *Served) Ran() { s.ran = true }

// Str reads a guatiao_str argument; ErrBadValue when it is not UTF-8.
func (s *Served) Str(p unsafe.Pointer, n uintptr) (string, error) {
	text := native.Text(p, n)
	if !utf8.ValidString(text) {
		return "", &Error{Status: BadValue, Message: "the text argument is not UTF-8"}
	}
	return text, nil
}

// Bytes copies a guatiao_bytes argument.
func (s *Served) Bytes(p unsafe.Pointer, n uintptr) []byte { return native.Bytes(p, n) }

// BytesMut is a guatiao_bytes_mut argument: the caller's own memory, valid
// for the call, which the method writes into and must not keep.
func (s *Served) BytesMut(p unsafe.Pointer, n uintptr) []byte {
	if n == 0 || p == nil {
		return []byte{}
	}
	return unsafe.Slice((*byte)(p), n)
}

// nodeOver is a node of this side's over the struct a call passed: the
// pointer itself for a value, a scratch node holding a copy of a map's or
// a list's struct.
func (s *Served) nodeOver(p unsafe.Pointer, class TreeClass) unsafe.Pointer {
	if class == ValueClass {
		return p
	}
	node := s.block(native.SizeValue)
	native.SetTag(node, class.tag())
	native.Copy(native.Payload(node), p, native.SizeMap)
	return node
}

func (s *Served) view(node unsafe.Pointer, alloc unsafe.Pointer) Ref {
	return Ref{o: &borrowed{find: func() unsafe.Pointer { return node }, alloc: alloc, open: s.isOpen}}
}

func wrapRef(r Ref, class TreeClass) any {
	switch class {
	case MapClass:
		return MapRef{r}
	case ListClass:
		return ListRef{r}
	}
	return r
}

// Borrowed is a borrowed tree argument as a read-only view valid for the
// call: a Ref, MapRef or ListRef as class says. A null value is the zero
// Ref; a null map or list is ErrNullArgument.
func (s *Served) Borrowed(p unsafe.Pointer, class TreeClass) (Ref, error) {
	if p == nil {
		if class == ValueClass {
			return Ref{}, nil
		}
		return Ref{}, &Error{Status: NullArgument, Message: "the " + class.String() + " argument is null"}
	}
	return s.view(s.nodeOver(p, class), nil), nil
}

// Lent is a tree lent for the call as a writable view: what the method
// adds is built in the allocator the tree records, and a map's or list's
// struct is written back when the call returns.
func (s *Served) Lent(p unsafe.Pointer, class TreeClass) (Ref, error) {
	if p == nil {
		return Ref{}, &Error{Status: NullArgument, Message: "the lent " + class.String() + " is null"}
	}
	node := s.nodeOver(p, class)
	if class != ValueClass {
		s.after = append(s.after, func() { native.Copy(p, native.Payload(node), native.SizeMap) })
	}
	alloc := native.Recorded(node)
	if alloc == nil {
		var err error
		if alloc, err = coreAlloc(); err != nil {
			return Ref{}, err
		}
	}
	return s.view(node, alloc), nil
}

// Taken is a tree handed over, as a Value the method owns and may keep:
// moved into this binding's allocator, out of the caller's, which may go
// with the caller's library; closed for it if the method never runs.
func (s *Served) Taken(p unsafe.Pointer, class TreeClass) (*Value, error) {
	return s.receiveTaken(p, class, true)
}

// receiveTaken is Taken, the tree left in the caller's allocators unless home.
func (s *Served) receiveTaken(p unsafe.Pointer, class TreeClass, home bool) (*Value, error) {
	if p == nil {
		return nil, &Error{Status: NullArgument, Message: "the " + class.String() + " argument is null"}
	}
	alloc, err := coreAlloc()
	if err != nil {
		return nil, err
	}
	node := native.Calloc(native.SizeValue)
	if class == ValueClass {
		native.Copy(node, p, native.SizeValue)
	} else {
		native.SetTag(node, class.tag())
		native.Copy(native.Payload(node), p, native.SizeMap)
	}
	v := adopt(node, alloc)
	if home {
		if err := check(native.ValueMakeLocal(node, alloc), "moving a tree handed over into this binding's allocator"); err != nil {
			v.Close()
			return nil, err
		}
	}
	s.taken = append(s.taken, func() { v.Close() })
	return v, nil
}

// Decode reads a borrowed or handed-over tree into dst; one handed over is
// freed once read, where it stands.
func (s *Served) Decode(p unsafe.Pointer, class TreeClass, taken bool, dst any) error {
	if taken {
		v, err := s.receiveTaken(p, class, false)
		if err != nil {
			return err
		}
		defer v.Close()
		return v.Decode(dst)
	}
	r, err := s.Borrowed(p, class)
	if err != nil {
		return err
	}
	return r.Decode(dst)
}

// TakenObject is an object handed over as a remote the method owns (and
// closes): destroyed for it if the method never runs.
func (s *Served) TakenObject(table unsafe.Pointer, size, ctx uintptr, k AnyKind) (Remote, error) {
	r, err := newRemote(k.desc(), table, size, ctx, true, nil)
	if err != nil {
		rawObject{table: table, size: size, ctx: ctx}.destroy(k.desc())
		return Remote{}, err
	}
	s.taken = append(s.taken, func() { r.Close() })
	return r, nil
}

// LentObject is an object lent for the call, as a remote valid for it.
func (s *Served) LentObject(p unsafe.Pointer, k AnyKind) (Remote, error) {
	if p == nil {
		return Remote{}, &Error{Status: NullArgument, Message: "no object was lent"}
	}
	table, size, ctx := native.ObjectAt(p)
	return newRemote(k.desc(), table, size, ctx, false, s.isOpen)
}

// Answer writes x (a *Value, a view, or Go data) over the carrier in out,
// built in the allocator the carrier names. A *Value answered is handed
// over: moved or copied, and closed.
func (s *Served) Answer(out unsafe.Pointer, class TreeClass, x any) error {
	if out == nil {
		return &Error{Status: NullArgument, Message: "no slot for the answer"}
	}
	var named unsafe.Pointer
	if class == ValueClass {
		named = native.OwnedAlloc(native.Payload(out))
	} else {
		named = native.OwnedAlloc(out)
	}
	alloc := named
	if !native.Complete(alloc) {
		var err error
		if alloc, err = coreAlloc(); err != nil {
			return err
		}
	}
	if v, ok := x.(*Value); ok && v != nil {
		defer v.Close()
	}
	node := native.Calloc(native.SizeValue)
	defer native.Free(node)
	if err := writeGo(node, x, alloc); err != nil {
		return err
	}
	if class != ValueClass && native.Tag(node) != class.tag() {
		got := Tag(native.Tag(node))
		native.ValueFree(node)
		return &Error{Status: WrongKind, Message: fmt.Sprintf("the answer is a %s, not a %s", got, class)}
	}
	if class == ValueClass {
		native.Copy(out, node, native.SizeValue)
	} else {
		native.Copy(out, native.Payload(node), native.SizeMap)
	}
	return nil
}

// AnswerText writes text as an owned guatiao_string into out.
func (s *Served) AnswerText(out unsafe.Pointer, text string) error {
	if out == nil {
		return &Error{Status: NullArgument, Message: "no slot for the answer"}
	}
	alloc, err := coreAlloc()
	if err != nil {
		return err
	}
	node := native.Calloc(native.SizeValue)
	defer native.Free(node)
	if st := native.ValueString(alloc, text, node); st != native.OK {
		return &Error{Status: Status(st), Message: "the text answer is not UTF-8"}
	}
	native.Copy(out, native.Payload(node), native.SizeString)
	return nil
}

// AnswerObject writes x, a Go value implementing the object kind k or a
// remote of it this side owns, into out: the caller owns it from now on.
func (s *Served) AnswerObject(out unsafe.Pointer, k AnyKind, x any) error {
	if out == nil {
		return &Error{Status: NullArgument, Message: "no slot for the answer"}
	}
	d := k.desc()
	var raw rawObject
	if rc, ok := x.(remoteCarrier); ok {
		given, err := rc.guatiaoRemote().giveUp()
		if err != nil {
			return err
		}
		raw = given
	} else {
		served, err := serveObject(d, x)
		if err != nil {
			return err
		}
		raw = served
	}
	native.SetObject(out, raw.table, raw.size, raw.ctx)
	return nil
}
