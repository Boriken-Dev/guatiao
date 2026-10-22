// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package serde_test

// A tree crosses between guatiao_serde and guatiao: built by one library,
// read and freed through the other, every block accounted for; and the
// library is found as each lookup says.

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/native"
	"github.com/Boriken-Dev/guatiao/bindings/go/serde"
)

func counting(t *testing.T) *native.Counting {
	t.Helper()
	c := native.NewCounting()
	t.Cleanup(func() {
		if live := c.Live(); live != 0 {
			t.Errorf("%d blocks still out", live)
		}
		c.Close()
	})
	return c
}

func TestATreeSerdeBuiltIsChangedAndFreedThroughTheCore(t *testing.T) {
	c := counting(t)
	v, err := serde.Parse(`{"a": [1, "two"]}`, serde.JSON, serde.InAllocator(c.Alloc()))
	if err != nil {
		t.Fatal(err)
	}
	if c.Live() == 0 {
		t.Fatal("nothing was built in the counting allocator")
	}
	m, err := v.Map()
	if err != nil {
		t.Fatal(err)
	}
	if err := m.Set("c", "added through the core"); err != nil {
		t.Fatal(err)
	}
	clone, err := v.Clone()
	if err != nil {
		t.Fatal(err)
	}
	clone.Close()
	v.Close()
}

func TestATreeTheCoreBuiltIsWrittenBySerde(t *testing.T) {
	c := counting(t)
	v, err := guatiao.NewValue(map[string]any{"port": 5900}, guatiao.InAllocator(c.Alloc()))
	if err != nil {
		t.Fatal(err)
	}
	text, err := serde.Emit(v, serde.JSON)
	if err != nil || text != `{"port":5900}` {
		t.Fatalf("%q %v", text, err)
	}
	v.Close()
}

func TestFreesAfterADocumentThatDoesNotParse(t *testing.T) {
	c := counting(t)
	_, err := serde.Parse(`{"a": [1,, 2]}`, serde.JSON, serde.InAllocator(c.Alloc()))
	var pe *serde.ParseError
	if !errors.As(err, &pe) {
		t.Fatalf("%v", err)
	}
}

// The lookup is decided once a process, so each case looks through the
// native layer's own candidates for a fresh process's environment.
func TestTheLibraryIsFoundThroughEachRoute(t *testing.T) {
	dir := os.Getenv("GUATIAO_LIBRARY")
	file := func(base string) string { return filepath.Join(dir, native.Filename(base)) }
	for _, c := range []struct {
		name, own, shared, want string
	}{
		{"its own variable names a file", file("guatiao_intake"), filepath.Join(dir, "nowhere"), file("guatiao_intake")},
		{"its own variable names a directory", dir, filepath.Join(dir, "nowhere"), file("guatiao_serde")},
		{"a shared file carrying the surface", "", file("guatiao_serde"), file("guatiao_serde")},
		{"a shared file without it points beside it", "", file("guatiao"), file("guatiao_serde")},
	} {
		t.Run(c.name, func(t *testing.T) {
			t.Setenv("GUATIAO_SERDE_LIBRARY", c.own)
			t.Setenv("GUATIAO_LIBRARY", c.shared)
			got, err := native.Locate(native.LibSerde)
			if err != nil || got != c.want {
				t.Fatalf("found %q (%v), want %q", got, err, c.want)
			}
		})
	}
	t.Setenv("GUATIAO_SERDE_LIBRARY", filepath.Join(dir, "nowhere"))
	t.Setenv("GUATIAO_LIBRARY", filepath.Join(dir, "nowhere"))
	if _, err := native.Locate(native.LibSerde); err == nil ||
		!strings.Contains(err.Error(), "GUATIAO_SERDE_LIBRARY") || !strings.Contains(err.Error(), "GUATIAO_LIBRARY") {
		t.Fatalf("%v", err)
	}
}
