// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"reflect"
	"testing"
)

// A list a served method borrows crosses as a map does: the caller passes
// the list's arm of its node, the method reads it through a read-only
// ListRef, and the caller's list is as it was after the call.
func TestAListBorrowedIsReadAndLeftAsItWas(t *testing.T) {
	v, err := NewValue([]any{40, 2})
	if err != nil {
		t.Fatal(err)
	}
	defer v.Close()
	c := &Call{}
	p, err := c.Borrow(v, ListClass)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.Borrow(map[string]any{"a": 1}, ListClass); err == nil {
		t.Fatal("a map borrowed as a list")
	}
	d := newKindDesc(KindSpec{Name: "summer", Slots: []SlotSpec{{Name: "sum"}}})
	var sum int64
	var pushed error
	st := serve(d, 0, nil, 0, func(s *Served) error {
		r, err := s.Borrowed(p, ListClass)
		if err != nil {
			return err
		}
		items := ListRef{r}
		pushed = items.Push(0)
		for _, item := range items.All() {
			var n int64
			if err := item.Decode(&n); err != nil {
				return err
			}
			sum += n
		}
		s.Ran()
		return nil
	})
	for _, f := range c.after {
		f()
	}
	if st != 0 || sum != 42 {
		t.Fatalf("status %d, sum %d", st, sum)
	}
	if pushed == nil {
		t.Fatal("a borrowed list was changed by its callee")
	}
	var got []int64
	if err := v.Decode(&got); err != nil || !reflect.DeepEqual(got, []int64{40, 2}) {
		t.Fatalf("%v %v", got, err)
	}
	st = serve(d, 0, nil, 0, func(s *Served) error {
		_, err := s.Borrowed(nil, ListClass)
		return err
	})
	if st != uint32(NullArgument) {
		t.Fatalf("status %d for a null list, want %d", st, NullArgument)
	}
}
