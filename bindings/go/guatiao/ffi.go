// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"unsafe"
)

// What a package over another guatiao C library builds on: the serde and
// intake packages call their own libraries and pass trees to and from
// this one through these, as may any package binding a C library that
// takes and answers guatiao values. A tree crosses as the address of its
// root guatiao_value. Whichever library built it, any of them frees it,
// since a value records its own allocator; the library that built it
// stays loaded while the tree lives.

// Load opens the guatiao library now, as the first call needing it would:
// nil once it is open, else a *LibraryNotFound or *MissingSymbol.
func Load() error {
	_, err := core()
	return err
}

// Adopt answers a *Value taking over root, a guatiao_value in memory from
// C's calloc that a call wrote: the Value frees the tree and that memory.
// alloc is the guatiao_alloc the tree was built in, kept to grow it with.
// Load must have succeeded.
func Adopt(root, alloc unsafe.Pointer) *Value {
	return adopt(root, alloc)
}

// WithNodes runs body with each item as a node: a Ref or *Value as it is,
// plain data (as New takes it) built for the call in the core library's
// allocator and freed after. A node is valid for the call only.
func WithNodes(items []any, body func(nodes []unsafe.Pointer) error) error {
	return withNodes(items, body)
}

// NodeOf is the node a Ref or *Value names now, to pass to a call while
// nothing changes its tree; false for an item that is neither. An error
// once its value is closed, or when its path names nothing.
func NodeOf(item any) (unsafe.Pointer, bool, error) {
	r, ok := asRef(item)
	if !ok {
		return nil, false, nil
	}
	node, err := r.requireNode()
	return node, true, err
}

// Borrow is a read-only Ref onto the node find answers, asked again on
// every access, valid while keep (a Ref or *Value) is: it refuses to read
// once keep's value is closed, and refuses every write. find answers nil
// once the node is gone.
func Borrow(find func() unsafe.Pointer, keep any) (Ref, error) {
	k, ok := asRef(keep)
	if !ok {
		return Ref{}, fmt.Errorf("guatiao: %T is not a value", keep)
	}
	return Ref{o: &borrowed{find: find, keep: k.o}}, nil
}

// StatusError is nil for status OK (0), else the *Error for it carrying
// message, the library's own words.
func StatusError(status uint32, message string) error {
	return check(status, message)
}
