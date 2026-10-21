// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const header = `
typedef struct probe_vtable {
  guatiao_kind_header header;
  guatiao_status (*greet)(void *ctx, guatiao_str name, guatiao_map *out, guatiao_provider_error *err);
  /*
   Lent for the call, not handed over: ` + "`config`" + `.
   */
  guatiao_status (*fill)(void *ctx, guatiao_map *config, bool *out, guatiao_provider_error *err);
  guatiao_status (*take)(void *ctx, guatiao_map *config, guatiao_provider_error *err);
  guatiao_status (*count)(void *ctx, int64_t *out);
} probe_vtable;
#define probe_vtable_FLOOR_HASH 42

typedef struct ear_vtable {
  guatiao_kind_header header;
  void (*destroy)(void *ctx);
  guatiao_status (*heard)(void *ctx, guatiao_str what);
} ear_vtable;
#define ear_vtable_FLOOR_HASH 7
`

const kinds = `package probe

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go"

//guatiao:kind probe probe_vtable
type Probe interface {
	Greet(name string) (map[string]any, error)
}

type Filler interface{ Fill(config guatiao.MapRef) (bool, error) }
type Taker interface{ Take(config *guatiao.Value) error }
type Counter interface{ Count() (int64, error) }

//guatiao:kind ear ear_vtable
type Ear interface {
	Heard(what string) error
	Close() error
}
`

// pkg writes a package of kinds and its header into a module of its own.
func pkg(t *testing.T, goSrc, hdr string) string {
	t.Helper()
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "go.mod"), []byte("module example.com/probe\n\ngo 1.26\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "kinds.go"), []byte(goSrc), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "probe.h"), []byte(hdr), 0o644); err != nil {
		t.Fatal(err)
	}
	return dir
}

func TestReadsATableAsTheHeaderGeneratorRendersIt(t *testing.T) {
	table, err := readTable(header, "probe_vtable")
	if err != nil {
		t.Fatal(err)
	}
	if table.floorHash != 42 || table.object || len(table.slots) != 4 {
		t.Fatalf("%+v", table)
	}
	if !table.slots[1].lent["config"] || table.slots[2].lent["config"] {
		t.Fatalf("lent: %v %v", table.slots[1].lent, table.slots[2].lent)
	}
	if got := table.slots[0].params[2].typ; got != "guatiao_map *" {
		t.Fatalf("%q", got)
	}
	ear, err := readTable(header, "ear_vtable")
	if err != nil || !ear.object || len(ear.slots) != 1 {
		t.Fatalf("%+v %v", ear, err)
	}
	if _, err := readTable(header, "nonesuch_vtable"); err == nil {
		t.Fatal("a table the header lacks")
	}
}

func TestWritesTheGlue(t *testing.T) {
	dir := pkg(t, kinds, header)
	if err := run(dir, "probe.h", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	goSrc, _ := os.ReadFile(filepath.Join(dir, "guatiao_kinds.go"))
	for _, want := range []string{
		"var ProbeKind = guatiao.NewKind", "type ProbeRemote struct{ guatiao.Remote }",
		"func (r *ProbeRemote) Fill(config guatiao.MapRef) (bool, error)",
		"c.Lend(config, guatiao.MapClass)", "c.Take(config, guatiao.MapClass)",
		"s.Lent(a0, guatiao.MapClass)", "s.Taken(a0, guatiao.MapClass)",
		"var _ Filler = (*ProbeRemote)(nil)", "Object:    true",
	} {
		if !strings.Contains(string(goSrc), want) {
			t.Errorf("no %q in\n%s", want, goSrc)
		}
	}
	cSrc, _ := os.ReadFile(filepath.Join(dir, "guatiao_kinds.c"))
	if !strings.Contains(string(cSrc), "guatiao_status (*)(void * ctx, guatiao_map * config, bool * out, guatiao_provider_error * err)") {
		t.Errorf("a slot is called through its own type:\n%s", cSrc)
	}
	if err := run(dir, "probe.h", "guatiao_kinds", true); err != nil {
		t.Fatalf("what it wrote is what it checks: %v", err)
	}
}

func TestRefusesAGoTypeThatCannotCarryTheSlots(t *testing.T) {
	cases := map[string]struct{ from, to, says string }{
		"a text argument":         {"Greet(name string)", "Greet(name int)", "crosses as string"},
		"a lent map as a value":   {"Fill(config guatiao.MapRef)", "Fill(config *guatiao.Value)", "lent tree"},
		"a taken map as a view":   {"Take(config *guatiao.Value)", "Take(config guatiao.MapRef)", "handed over"},
		"no error last":           {"Count() (int64, error)", "Count() int64", "error"},
		"a missing optional slot": {"type Counter interface{ Count() (int64, error) }", "", "no interface in the package has Count"},
		"an object without Close": {"\tClose() error\n", "", "Close() error"},
	}
	for name, c := range cases {
		src := strings.Replace(kinds, c.from, c.to, 1)
		err := run(pkg(t, src, header), "probe.h", "guatiao_kinds", false)
		if err == nil || !strings.Contains(err.Error(), c.says) {
			t.Errorf("%s: %v", name, err)
		}
	}
}
