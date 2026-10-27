// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package tallykind

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"sort"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// The document and the glue are what guatiao-kind writes from these
// interfaces: the document byte for byte the one the TypeScript
// declaration is read into, which the Node suite checks from its side.
func TestTheDocumentAndTheGlueAreCurrent(t *testing.T) {
	out, err := exec.Command("go", "run", "github.com/Boriken-Dev/guatiao/bindings/go/guatiao/cmd/guatiao-kind",
		"-document", "../include/tally_kind.kinds.json", "-check").CombinedOutput()
	if err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
}

// counts is a tally served from Go: every slot, required and appended.
type counts struct{ by map[string]int64 }

var (
	_ Tally       = (*counts)(nil)
	_ AllCounter  = (*counts)(nil)
	_ Knower      = (*counts)(nil)
	_ Checksummer = (*counts)(nil)
	_ Reporter    = (*counts)(nil)
	_ Loader      = (*counts)(nil)
)

func (c *counts) Add(name string, by uint32) (int64, error) {
	if name == "" {
		return 0, guatiao.Errorf(guatiao.BadValue, "nothing to count")
	}
	c.by[name] += int64(by)
	return c.by[name], nil
}

func (c *counts) Total(name string) (int64, error) { return c.by[name], nil }

func (c *counts) All() (map[string]int64, error) { return c.by, nil }

func (c *counts) Known(name string) (bool, error) {
	_, ok := c.by[name]
	return ok, nil
}

func (c *counts) Checksum(data []byte) (uint32, error) {
	var sum uint32
	for _, b := range data {
		sum += uint32(b)
	}
	return sum, nil
}

func (c *counts) Report(w Watcher) error {
	names := make([]string, 0, len(c.by))
	for n := range c.by {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		if err := w.Changed(n, c.by[n]); err != nil {
			return err
		}
	}
	return nil
}

func (c *counts) Load(more map[string]int64) (uint32, error) {
	for n, v := range more {
		c.by[n] += v
	}
	return uint32(len(more)), nil
}

// heard is the host's watcher, lent for one call.
type heard struct{ said []string }

func (h *heard) Changed(name string, total int64) error {
	h.said = append(h.said, fmt.Sprintf("%s=%d", name, total))
	return nil
}

func (h *heard) Close() error { return nil }

func TestAKindBornInGoIsServedAndCalledInEverySlot(t *testing.T) {
	if os.Getenv("GUATIAO_LIBRARY") == "" {
		os.Setenv("GUATIAO_LIBRARY", filepath.Join("..", "..", "..", "target", "debug"))
	}
	reg, err := guatiao.NewRegistry("tally-host", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	defer reg.Close()
	lib, err := guatiao.LibraryOf(
		guatiao.Single("counts", &counts{by: map[string]int64{}}, TallyKind),
		guatiao.LibraryID("go_tally"),
		guatiao.LibraryVersion("0.1.0"),
	)
	if err != nil {
		t.Fatal(err)
	}
	defer lib.Close()
	if _, err := reg.RegisterLibrary(lib); err != nil {
		t.Fatal(err)
	}
	tally, err := TallyKind.Bind(reg, "go_tally_counts")
	if err != nil {
		t.Fatal(err)
	}
	if got, err := tally.Add("apples", 3); err != nil || got != 3 {
		t.Fatalf("add = %d, %v", got, err)
	}
	if _, err := tally.Add("", 1); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("add of nothing: %v", err)
	}
	if got, err := tally.Total("apples"); err != nil || got != 3 {
		t.Fatalf("total = %d, %v", got, err)
	}
	if got, err := tally.Load(map[string]int64{"pears": 2, "apples": 1}); err != nil || got != 2 {
		t.Fatalf("load = %d, %v", got, err)
	}
	if got, err := tally.All(); err != nil || !reflect.DeepEqual(got, map[string]int64{"apples": 4, "pears": 2}) {
		t.Fatalf("all = %v, %v", got, err)
	}
	if got, err := tally.Known("plums"); err != nil || got {
		t.Fatalf("known(plums) = %v, %v", got, err)
	}
	if got, err := tally.Checksum([]byte{250, 10, 1}); err != nil || got != 261 {
		t.Fatalf("checksum = %d, %v", got, err)
	}
	w := &heard{}
	if err := tally.Report(w); err != nil || !reflect.DeepEqual(w.said, []string{"apples=4", "pears=2"}) {
		t.Fatalf("report: %v, %v", w.said, err)
	}
	if TallyKind.FloorHash() != 2688319391 || WatcherKind.FloorHash() != 297588550 {
		t.Fatalf("floor hashes %d, %d", TallyKind.FloorHash(), WatcherKind.FloorHash())
	}
}
