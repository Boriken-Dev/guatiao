// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
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

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

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

// The probe kinds of the header above, as their description.
const document = `{"kinds": [
  {"name": "probe", "floor_hash": 42, "object": false, "slots": [
    {"name": "greet", "required": true, "fallible": true, "args": [{"name": "name", "type": "str"}], "answer": {"type": "map"}},
    {"name": "fill", "required": false, "fallible": true, "args": [{"name": "config", "type": "map", "pass": "lent"}], "answer": {"type": "bool"}},
    {"name": "take", "required": false, "fallible": true, "args": [{"name": "config", "type": "map", "pass": "taken"}]},
    {"name": "count", "required": false, "fallible": false, "args": [], "answer": {"type": "i64"}}]},
  {"name": "ear", "floor_hash": 7, "object": true, "slots": [
    {"name": "heard", "required": true, "fallible": false, "args": [{"name": "what", "type": "str"}]}]}]}`

// A kind read from its description is the table its header declares, so
// the glue is the same whichever it was written from: for the greeter
// kinds, whose header and document the kind crate commits side by side.
func TestReadsADescriptionAsTheHeaderDeclaresIt(t *testing.T) {
	include := filepath.Join("..", "..", "..", "..", "..", "examples", "greeter_kind", "include")
	hdr, err := os.ReadFile(filepath.Join(include, "greeter_kind.h"))
	if err != nil {
		t.Fatal(err)
	}
	text, err := os.ReadFile(filepath.Join(include, "greeter_kind.kinds.json"))
	if err != nil {
		t.Fatal(err)
	}
	doc, err := readDocument(text)
	if err != nil {
		t.Fatal(err)
	}
	for _, k := range doc.Kinds {
		fromHeader, err := readTable(string(hdr), k.Name+"_vtable")
		if err != nil {
			t.Fatal(err)
		}
		described, err := describedTable(doc, k.Name, k.Name+"_vtable")
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(fromHeader, described) {
			t.Errorf("%s:\nheader      %+v\ndescription %+v", k.Name, fromHeader, described)
		}
	}
}

func TestWritesTheSameGlueFromADescription(t *testing.T) {
	fromHeader := pkg(t, kinds, header)
	if err := run(fromHeader, "probe.h", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	described := pkg(t, strings.ReplaceAll(kinds, " probe_vtable", ""), header)
	if err := os.WriteFile(filepath.Join(described, "probe.kinds.json"), []byte(document), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := run(described, "probe.kinds.json", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"guatiao_kinds.go", "guatiao_kinds.h", "guatiao_kinds.c"} {
		a, _ := os.ReadFile(filepath.Join(fromHeader, name))
		b, _ := os.ReadFile(filepath.Join(described, name))
		a = bytes.ReplaceAll(a, []byte("probe.h"), []byte("SOURCE"))
		b = bytes.ReplaceAll(b, []byte("probe.kinds.json"), []byte("SOURCE"))
		if !bytes.Equal(a, b) {
			t.Errorf("%s differs:\n%s\n---\n%s", name, a, b)
		}
	}
	if err := run(described, "probe.kinds.json", "guatiao_kinds", true); err != nil {
		t.Fatalf("what it wrote is what it checks: %v", err)
	}
}

func TestRefusesADescriptionItCannotRead(t *testing.T) {
	cases := map[string]struct{ from, to, says string }{
		"no kinds":          {`{"kinds": [`, `{"kindz": [`, "a kinds document is"},
		"an unknown type":   {`"type": "i64"`, `"type": "u128"`, "an answer cannot be"},
		"a borrowed list":   {`"type": "map", "pass": "taken"`, `"type": "list", "pass": "borrowed"`, "a list cannot be passed borrowed"},
		"a kind not in it":  {`"name": "ear"`, `"name": "lug"`, "describes no kind ear"},
		"a pass it ignores": {`"pass": "lent"`, `"pass": "kept"`, "borrowed, taken or lent"},
	}
	for name, c := range cases {
		dir := pkg(t, strings.ReplaceAll(kinds, " probe_vtable", ""), header)
		if err := os.WriteFile(filepath.Join(dir, "probe.kinds.json"), []byte(strings.Replace(document, c.from, c.to, 1)), 0o644); err != nil {
			t.Fatal(err)
		}
		err := run(dir, "probe.kinds.json", "guatiao_kinds", false)
		if err == nil || !strings.Contains(err.Error(), c.says) {
			t.Errorf("%s: %v", name, err)
		}
	}
}

// A kind born in TypeScript has no header: its description is all there
// is, and the marker names no C table.
const tallyKinds = `package tally

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

//guatiao:kind watcher
type Watcher interface {
	Changed(name string, total int64) error
	Close() error
}

//guatiao:kind tally
type Tally interface {
	Add(name string, by uint32) (int64, error)
	Total(name string) (int64, error)
}

type Aller interface{ All() (map[string]int64, error) }
type Knower interface{ Known(name string) (bool, error) }
type Checksummer interface{ Checksum(data []byte) (uint32, error) }
type Reporter interface{ Report(watcher Watcher) error }
type Loader interface{ Load(counts *guatiao.Value) (uint32, error) }
`

func TestWritesGlueForAKindWithNoHeader(t *testing.T) {
	text, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "..", "examples", "tally_kind", "include", "tally_kind.kinds.json"))
	if err != nil {
		t.Fatal(err)
	}
	dir := pkg(t, tallyKinds, "")
	if err := os.WriteFile(filepath.Join(dir, "tally_kind.kinds.json"), text, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := run(dir, "tally_kind.kinds.json", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	goSrc, _ := os.ReadFile(filepath.Join(dir, "guatiao_kinds.go"))
	for _, want := range []string{
		"var TallyKind = guatiao.NewKind", "FloorHash: 2688319391", "var WatcherKind = guatiao.NewKind",
		"func (r *TallyRemote) Report(watcher Watcher) error", "c.Take(counts, guatiao.MapClass)",
	} {
		if !strings.Contains(string(goSrc), want) {
			t.Errorf("no %q in\n%s", want, goSrc)
		}
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

func TestRenamesAParameterTheGeneratedMethodUsesItself(t *testing.T) {
	src := strings.Replace(kinds, "Fill(config guatiao.MapRef)", "Fill(out guatiao.MapRef)", 1)
	src = strings.Replace(src, "Greet(name string)", "Greet(c string)", 1)
	dir := pkg(t, src, header)
	if err := run(dir, "probe.h", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	goSrc, _ := os.ReadFile(filepath.Join(dir, "guatiao_kinds.go"))
	for _, want := range []string{"Fill(out_ guatiao.MapRef)", "c.Lend(out_, guatiao.MapClass)", "Greet(c_ string)", "c.Str(c_)"} {
		if !strings.Contains(string(goSrc), want) {
			t.Errorf("no %q in\n%s", want, goSrc)
		}
	}
}
