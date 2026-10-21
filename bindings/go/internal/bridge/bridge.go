// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package bridge lends the guatiao package's internals to its satellite
// packages (serde, intake, civil), which need a node's address and a way
// to hand one back as a Value without making either part of the API.
// The guatiao package fills these in its init; a satellite imports
// guatiao, so they are set before it runs.
package bridge

import "unsafe"

var (
	// Core opens the guatiao library, as the guatiao package reports a
	// failure.
	Core func() error
	// Lift turns the native layer's failures into the guatiao package's.
	Lift func(error) error
	// WithNodes runs body with each item as a node: a guatiao view as it
	// is, plain data built for the call and freed after.
	WithNodes func(items []any, body func(nodes []unsafe.Pointer) error) error
	// NodeOf is the node a guatiao view names now; false for an item that
	// is not a view.
	NodeOf func(item any) (unsafe.Pointer, bool, error)
	// Adopt is a *guatiao.Value taking over root (from native.Calloc),
	// built in alloc.
	Adopt func(root, alloc unsafe.Pointer) any
	// Borrow is a read-only guatiao.Ref over a node find answers again on
	// every access, valid while keep (a guatiao view) is.
	Borrow func(find func() unsafe.Pointer, keep any) (any, error)
	// Check is nil for status OK, else the guatiao.Error for it.
	Check func(status uint32, message string) error
)
