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

// A call refused while receiving one handed-over tree still takes, and
// frees, every tree handed over after it: the glue receives `both(a, b)`
// as guatiao-kind writes it, with `a` null.
func TestACallRefusedAtOneTreeHandedOverStillTakesTheOthers(t *testing.T) {
	c := native.NewCounting()
	defer c.Close()
	v, err := NewValue(map[string]any{"b": 2}, InAllocator(c.Alloc()))
	if err != nil {
		t.Fatal(err)
	}
	root, err := v.takeRoot()
	if err != nil {
		t.Fatal(err)
	}
	defer native.Free(root)
	d := newKindDesc(KindSpec{Name: "pair", Slots: []SlotSpec{{Name: "both"}}})
	st := serve(d, 0, nil, 0, func(s *Served) error {
		var a, b map[string]any
		if err := s.Decode(nil, MapClass, true, &a); err != nil {
			return err
		}
		if err := s.Decode(native.Payload(root), MapClass, true, &b); err != nil {
			return err
		}
		if _, err := s.Target(); err != nil {
			return err
		}
		s.Ran()
		return nil
	})
	if st != uint32(NullArgument) {
		t.Fatalf("status %d, want the null argument's %d", st, NullArgument)
	}
	if live := c.Live(); live != 0 {
		t.Fatalf("%d blocks of the map handed over after the null one are left", live)
	}
}
