// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"reflect"
	"testing"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// A tree handed over to a served method is moved out of the caller's
// allocator on receipt, so the method may keep it past the caller's
// library. Generated glue reaches Served.Taken for a *Value argument; no
// kind in the repository declares one, so the test drives it directly.
func TestATreeHandedOverIsMovedIntoThisBindingsAllocator(t *testing.T) {
	c := native.NewCounting()
	defer c.Close()
	v, err := NewValue(map[string]any{"name": "ivy"}, InAllocator(c.Alloc()))
	if err != nil {
		t.Fatal(err)
	}
	// The caller's root, as it crosses: its tree is the method's from here.
	root, err := v.takeRoot()
	if err != nil {
		t.Fatal(err)
	}
	defer native.Free(root)
	s := &Served{open: true}
	kept, err := s.Taken(native.Payload(root), MapClass)
	if err != nil {
		t.Fatal(err)
	}
	defer kept.Close()
	if live := c.Live(); live != 0 {
		t.Fatalf("%d blocks of the caller's allocator in what the method keeps", live)
	}
	var got map[string]any
	if err := kept.Decode(&got); err != nil || !reflect.DeepEqual(got, map[string]any{"name": "ivy"}) {
		t.Fatalf("%v %v", got, err)
	}
}
