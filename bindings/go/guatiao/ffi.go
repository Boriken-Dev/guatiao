// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"sync"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// What a package over another guatiao C library builds on: the serde and
// intake modules open their own libraries as a Surface and pass trees to
// and from this one through these, as may any package binding a C library
// that takes and answers guatiao values. A tree crosses as the address of
// its root guatiao_value. Whichever library built it, any of them frees
// it, since a value records its own allocator; the library that built it
// stays loaded while the tree lives.

// Surface is a guatiao C library other than this package's, declared once
// by the package over it, in a package-level variable (a Surface is not
// copied):
//
//	var library = &guatiao.Surface{
//		Basename: "guatiao_serde",
//		Variable: "GUATIAO_SERDE_LIBRARY",
//		Probe:    "guatiao_json_parse_ex",
//		Exports:  map[string]string{"guatiao_json_parse_ex": "", "guatiao_toml_parse_ex": "toml"},
//	}
//
// It is found in this order: the path given to Use; the file or directory
// its Variable names; GUATIAO_LIBRARY, a directory holding it under its
// platform file name, or a file, taken when it exports Probe, else the
// file beside it; the platform's search for its file name. Once open it
// stays open for the process.
type Surface struct {
	// Basename is the file name without the platform's prefix and
	// suffix: guatiao_serde is guatiao_serde.dll, libguatiao_serde.so or
	// libguatiao_serde.dylib.
	Basename string
	// Variable is its own environment variable, consulted before
	// GUATIAO_LIBRARY; "" for none.
	Variable string
	// Probe is an export only this library has. "" takes a file
	// GUATIAO_LIBRARY names as it is.
	Probe string
	// Exports maps each export the package calls to the feature a build
	// needs for it: "" for one every build has, which a file must export
	// to be opened. guatiao_alloc_default is always resolved.
	Exports map[string]string

	mu      sync.Mutex
	chosen  string
	opened  *native.Opened
	failure error
}

func (s *Surface) spec() native.Spec {
	return native.Spec{Base: s.Basename, Variable: s.Variable, Probe: s.Probe}
}

// Use names the library to open instead of looking for one: a file
// exporting it, whatever it is called, or a directory holding it under
// its platform file name. Call it before the first Open; naming another
// file once one is open is an error, naming the same one is not.
func (s *Surface) Use(where string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.opened != nil {
		if !native.Same(s.spec(), where, s.opened.Path) {
			return fmt.Errorf("guatiao: the %s library is already loaded from %s", s.Basename, s.opened.Path)
		}
		return nil
	}
	s.chosen = where
	return nil
}

// Find is the file the lookup settles on now, without opening it: a
// *LibraryNotFound naming every route when none names a file, the
// platform's search being all that is left.
func (s *Surface) Find() (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	path, err := native.Locate(s.spec(), s.chosen)
	return path, lift(err)
}

// Open opens this package's library as Load does, then the Surface's,
// resolving its Exports; once it has, it does nothing. Not found is a
// *LibraryNotFound, kept for the process; a file lacking an export every
// build has is a *MissingSymbol.
func (s *Surface) Open() error {
	if err := Load(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.opened != nil {
		return nil
	}
	if s.failure != nil {
		return s.failure
	}
	o, err := native.Open(s.spec(), s.chosen, s.Exports)
	if err != nil {
		err = lift(err)
		if _, ok := err.(*LibraryNotFound); ok {
			s.failure = err
		}
		return err
	}
	s.opened = o
	return nil
}

// Symbol opens the library as Open does and answers the address of an
// export its Exports name, to call from C through a pointer of the
// export's own type: a *MissingSymbol naming its feature when this build
// lacks it.
func (s *Surface) Symbol(name string) (unsafe.Pointer, error) {
	if err := s.Open(); err != nil {
		return nil, err
	}
	if p, ok := s.opened.Symbols[name]; ok {
		return p, nil
	}
	if feature, ok := s.opened.Missing[name]; ok {
		return nil, &MissingSymbol{Path: s.opened.Path, Symbol: name, Feature: feature}
	}
	return nil, fmt.Errorf("guatiao: %s is not among the exports declared for %s", name, s.Basename)
}

// Alloc is the open library's default guatiao_alloc, kept for the
// process (a tree built in it records this address); nil before Open.
func (s *Surface) Alloc() unsafe.Pointer {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.opened == nil {
		return nil
	}
	return s.opened.Alloc
}

// Path is the file the library was opened from; "" before Open.
func (s *Surface) Path() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.opened == nil {
		return ""
	}
	return s.opened.Path
}

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
