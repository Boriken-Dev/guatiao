// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// The binding frees what it allocates on every path, proved with an
// allocator that counts the blocks it handed out and has not had back.

import (
	"runtime"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/native"
)

func counting(t *testing.T) (*native.Counting, guatiao.Option) {
	t.Helper()
	c := native.NewCounting()
	t.Cleanup(func() {
		if live := c.Live(); live != 0 {
			t.Errorf("%d blocks still out", live)
		}
		c.Close()
	})
	return c, guatiao.InAllocator(c.Alloc())
}

func TestFreesACleanBuild(t *testing.T) {
	c, in := counting(t)
	v, err := guatiao.NewValue([]any{[]any{1, "x"}, map[string]any{"k": []int{2, 3}}}, in)
	if err != nil {
		t.Fatal(err)
	}
	if c.Live() == 0 {
		t.Fatal("nothing was built in the counting allocator")
	}
	v.Close()
}

func TestFreesWhatAFailedConversionBuilt(t *testing.T) {
	_, in := counting(t)
	for _, bad := range []any{
		[]any{[]any{1, "x", make(chan int)}},
		map[string]any{"a": []int{1, 2}, "b": map[string]any{"c": "d", "e": func() {}}},
		[]any{map[string]any{"a": 1}, map[string]any{"b": []any{true, nil, complex(1, 1)}}},
		[]any{"ok", "\xff"},
	} {
		if v, err := guatiao.NewValue(bad, in); err == nil {
			v.Close()
			t.Errorf("%#v converted", bad)
		}
	}
}

func TestFreesThePartOfAFailedWrite(t *testing.T) {
	c, in := counting(t)
	v, err := guatiao.NewValue(map[string]any{"a": 1}, in)
	if err != nil {
		t.Fatal(err)
	}
	before := c.Live()
	m, _ := v.Map()
	if err := m.Set("b", []any{1, "two", make(chan int)}); err == nil {
		t.Fatal("Set took a channel")
	}
	if c.Live() != before || m.Has("b") {
		t.Fatalf("a refused Set left %d blocks, %d before", c.Live(), before)
	}
	v.Close()
	l, list := mustListIn(t, in)
	before = c.Live()
	if err := l.Push(map[string]any{"k": []any{"x", func() {}}}); err == nil {
		t.Fatal("Push took a func")
	}
	if c.Live() != before || l.Len() != 1 {
		t.Fatalf("a refused Push left %d blocks, %d before", c.Live(), before)
	}
	list.Close()
}

func mustListIn(t *testing.T, in guatiao.Option) (guatiao.ListRef, *guatiao.Value) {
	t.Helper()
	v, err := guatiao.NewValue([]any{"a"}, in)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { v.Close() })
	l, _ := v.List()
	return l, v
}

func TestFreesTheCopiesOfAFailedListRebuild(t *testing.T) {
	c, in := counting(t)
	v, err := guatiao.NewValue([]any{"a", []any{"b", "c"}, map[string]any{"d": "e"}}, in)
	if err != nil {
		t.Fatal(err)
	}
	l, _ := v.List()
	before := c.Live()
	for _, err := range []error{
		l.Set(0, make(chan int)),
		l.Set(1, []any{"x", func() {}}),
		l.Set(9, "x"),
		l.Insert(1, []any{complex(1, 1)}),
	} {
		if err == nil {
			t.Fatal("a refused change was made")
		}
	}
	if c.Live() != before {
		t.Fatalf("%d blocks before, %d after", before, c.Live())
	}
	if err := l.Set(0, map[string]any{"k": []int{1}}); err != nil {
		t.Fatal(err)
	}
	if err := l.Insert(1, "mid"); err != nil {
		t.Fatal(err)
	}
	v.Close()
}

func TestAValueDroppedUnclosedIsFreedByTheCollector(t *testing.T) {
	c, in := counting(t)
	func() {
		v, err := guatiao.NewValue(map[string]any{"k": []string{"a", "b"}}, in)
		if err != nil {
			t.Fatal(err)
		}
		_ = v.Key("k")
	}()
	deadline := time.Now().Add(10 * time.Second)
	for c.Live() != 0 && time.Now().Before(deadline) {
		runtime.GC()
		time.Sleep(10 * time.Millisecond)
	}
	if c.Live() != 0 {
		t.Fatalf("%d blocks after collection", c.Live())
	}
}
