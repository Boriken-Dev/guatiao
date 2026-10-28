// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strconv"
	"strings"
	"testing"
)

func repoFile(t *testing.T, parts ...string) []byte {
	t.Helper()
	text, err := os.ReadFile(filepath.Join(append([]string{"..", "..", "..", "..", ".."}, parts...)...))
	if err != nil {
		t.Fatal(err)
	}
	return text
}

func TestDescribesKindsBornInGoAsTheTraitDoes(t *testing.T) {
	p, err := load(filepath.Join("testdata", "greeter"), "guatiao_kinds")
	if err != nil {
		t.Fatal(err)
	}
	got, err := p.describe()
	if err != nil {
		t.Fatal(err)
	}
	want := repoFile(t, "examples", "greeter_kind", "include", "greeter_kind.kinds.json")
	if !bytes.Equal(got, want) {
		t.Fatalf("the greeter kinds described from Go:\n%s", got)
	}
}

func TestComputesTheFloorHashOfTheSharedVectors(t *testing.T) {
	var file struct {
		Vectors []struct {
			Name      string    `json:"name"`
			Object    bool      `json:"object"`
			Slots     []outSlot `json:"slots"`
			FloorHash uint32    `json:"floor_hash"`
		} `json:"vectors"`
	}
	if err := json.Unmarshal(repoFile(t, "crates", "guatiao-derive", "tests", "vectors", "floor_hash.json"), &file); err != nil {
		t.Fatal(err)
	}
	if len(file.Vectors) < 6 {
		t.Fatalf("%d vectors", len(file.Vectors))
	}
	for _, v := range file.Vectors {
		if got, err := floorHashOf(v.Object, v.Slots); err != nil || got != v.FloorHash {
			t.Errorf("%s: %d (%v), Rust %d", v.Name, got, err, v.FloorHash)
		}
	}
	// What is not a description is refused by the library, naming where.
	_, err := floorHashOf(false, []outSlot{{Name: "x", Required: true, Args: []outType{{Name: "a", Type: "u256"}}}})
	if err == nil || !strings.Contains(err.Error(), "slots[0].args[0].type") {
		t.Fatalf("u256 hashed: %v", err)
	}
}

const bornKinds = `package probe

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

// Hears.
//
//guatiao:kind ear
type Ear interface {
	//guatiao:infallible
	Heard(what string) error
	Close() error
}

// Probes.
//
//guatiao:kind probe
//guatiao:floorhash 42
//guatiao:optional Filler
type Probe interface {
	// Reads ` + "`v`" + `, which may be null.
	//
	//guatiao:nullable v
	//guatiao:taken items
	Read(v guatiao.Ref, items []string, ear Ear) (*guatiao.Value, error)
}

// Fills in place.
type Filler interface {
	//guatiao:lent config ear
	Fill(config guatiao.ListRef, ear Ear) (bool, error)
}
`

func TestWritesTheDocumentAndTheGlueOfKindsBornInGo(t *testing.T) {
	src := strings.Replace(bornKinds, "items []string", "items Items", 1) + "\ntype Items []string\n"
	dir := pkg(t, src, "")
	document := filepath.Join("include", "probe.kinds.json")
	if err := runBorn(dir, document, "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	text, err := os.ReadFile(filepath.Join(dir, document))
	if err != nil {
		t.Fatal(err)
	}
	doc, err := readDocument(text)
	if err != nil {
		t.Fatal(err)
	}
	if len(doc.Kinds) != 2 || doc.Kinds[0].Name != "ear" || doc.Kinds[1].Name != "probe" || doc.Kinds[1].FloorHash != 42 {
		t.Fatalf("%s", text)
	}
	earHash, err := floorHashOf(true, []outSlot{{Name: "heard", Required: true, Args: []outType{{Name: "what", Type: "str"}}}})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"\"name\": \"v\",\n              \"type\": \"value\",\n              \"pass\": \"borrowed\",\n              \"nullable\": true",
		"\"name\": \"items\",\n              \"type\": \"list\",\n              \"pass\": \"taken\"",
		"\"type\": \"list\",\n              \"pass\": \"lent\"",
		"\"kind\": \"ear\",\n              \"pass\": \"lent\"",
		"\"floor_hash\": " + strconv.FormatUint(uint64(earHash), 10),
		"\"doc\": \"Reads `v`, which may be null.\"",
		"\"doc\": \"Fills in place.\"",
	} {
		if !strings.Contains(string(text), want) {
			t.Errorf("no %q in\n%s", want, text)
		}
	}
	goSrc, _ := os.ReadFile(filepath.Join(dir, "guatiao_kinds.go"))
	for _, want := range []string{"var ProbeKind = guatiao.NewKind", "FloorHash: 42", "c.Lend(config, guatiao.ListClass)"} {
		if !strings.Contains(string(goSrc), want) {
			t.Errorf("no %q in\n%s", want, goSrc)
		}
	}
	if err := runBorn(dir, document, "guatiao_kinds", true); err != nil {
		t.Fatalf("what it wrote is what it checks: %v", err)
	}
	stale := strings.Replace(src, "// Hears.", "// Listens.", 1)
	if err := os.WriteFile(filepath.Join(dir, "kinds.go"), []byte(stale), 0o644); err != nil {
		t.Fatal(err)
	}
	err = runBorn(dir, document, "guatiao_kinds", true)
	if err == nil || !strings.Contains(err.Error(), "not the document") {
		t.Fatalf("a document the interfaces no longer write: %v", err)
	}
}

func TestRefusesAKindBornInGoItCannotDescribe(t *testing.T) {
	cases := map[string]struct{ from, to, says string }{
		"an unknown directive":       {"//guatiao:infallible", "//guatiao:infalible", "is not one of"},
		"a mark on no argument":      {"//guatiao:lent config ear", "//guatiao:lent config ear cup", "//guatiao:lent cup: no argument has that name"},
		"a borrowed list view":       {"//guatiao:lent config ear", "//guatiao:lent ear", "a list is not borrowed"},
		"a borrowed list":            {"\t//guatiao:taken items\n", "", "args[1].pass` is borrowed: a list is not borrowed"},
		"capitals in a row":          {"Fill(config", "FillIO(config", "capitals in a row"},
		"an optional non-interface":  {"//guatiao:optional Filler", "//guatiao:optional Filer", "Filer, which is no interface"},
		"an optional kind":           {"//guatiao:optional Filler", "//guatiao:optional Ear", "a kind of its own"},
		"another package's type":     {"items []string", "items time.Duration", "another package's type"},
		"no error last":              {"Heard(what string) error", "Heard(what string)", "returns an error last"},
		"a taken view":               {"//guatiao:nullable v\n\t//guatiao:taken items", "//guatiao:taken v items", "a view is borrowed"},
		"a lent string":              {"//guatiao:infallible\n\tHeard", "//guatiao:lent what\n\tHeard", "cannot be marked lent"},
		"a provider kind passed":     {"ear Ear) (*", "ear Probe) (*", "no object kind"},
		"a floor hash not a number":  {"//guatiao:floorhash 42", "//guatiao:floorhash x", "32-bit unsigned integer"},
		"an optional slot twice":     {"//guatiao:optional Filler", "//guatiao:optional Filler Filler", "two slots of probe are fill"},
		"an optional kind method":    {"\tFill(config", "\tRead(config", "Filler.Read is a method of Probe already"},
		"an optional destroy":        {"// Hears.\n//\n//guatiao:kind ear", "type Closer interface{ Close() error }\n\n// Hears.\n//\n//guatiao:kind ear\n//guatiao:optional Closer", "Closer.Close is a method of Ear already"},
		"a method declared twice":    {"// Fills in place.", "type Other interface{ Fill(config guatiao.ListRef, ear Ear) (bool, error) }\n\n// Fills in place.", "Filler and Other both have Fill"},
		"a kind named twice":         {"//guatiao:kind probe", "//guatiao:kind ear", "both the kind ear"},
		"a slot directive misplaced": {"// Fills in place.", "// Fills in place.\n//\n//guatiao:infallible", "a directive of a slot goes on its method"},
		"a map keyed by numbers":     {"items []string", "items map[int]string", "a map's keys are text"},
		"an argument with no name":   {"Heard(what string) error", "Heard(string) error", "every argument has a name"},
		"an underscore in a method":  {"Fill(config", "Fill_it(config", "no slot's name in CamelCase"},
		"an empty mark":              {"//guatiao:lent config ear", "//guatiao:lent", "names what it marks"},
		"a destroy slot":             {"\tHeard(what string) error\n", "\tHeard(what string) error\n\tDestroy() error\n", "destroy is the slot"},
		"a remote's own method":      {"Fill(config", "Has(config", "Has is a remote's own method"},
		"bytes_mut on a provider": {"//guatiao:taken items\n\tRead(v guatiao.Ref, items []string, ear Ear)",
			"//guatiao:taken items\n\t//guatiao:mut buf\n\tRead(v guatiao.Ref, items []string, ear Ear, buf []byte)",
			"on an object kind only"},
	}
	for name, c := range cases {
		src := strings.Replace(bornKinds, c.from, c.to, 1)
		if !strings.Contains(bornKinds, c.from) {
			t.Fatalf("%s: %q is not in the source", name, c.from)
		}
		dir := pkg(t, src, "")
		err := runBorn(dir, "probe.kinds.json", "guatiao_kinds", false)
		if err == nil || !strings.Contains(err.Error(), c.says) {
			t.Errorf("%s: %v", name, err)
		}
		if _, err := os.Stat(filepath.Join(dir, "probe.kinds.json")); err == nil {
			t.Errorf("%s: a document was written for kinds that were refused", name)
		}
	}
}

// An optional slot comes where a //guatiao:optional puts it: before the
// method whose doc comment names it, as a trait declares it between two
// required methods.
func TestPutsAnOptionalSlotBetweenRequiredOnes(t *testing.T) {
	var file struct {
		Vectors []struct {
			Name      string          `json:"name"`
			Slots     json.RawMessage `json:"slots"`
			FloorHash uint32          `json:"floor_hash"`
		} `json:"vectors"`
	}
	if err := json.Unmarshal(repoFile(t, "crates", "guatiao-derive", "tests", "vectors", "floor_hash.json"), &file); err != nil {
		t.Fatal(err)
	}
	src := `package probe

//guatiao:kind between
type Between interface {
	First(name string) (map[string]any, error)
	//guatiao:optional Middle
	//guatiao:infallible
	Last(n uint32) (uint64, error)
}

type Middle interface {
	//guatiao:infallible
	Middle(name string) (string, error)
}
`
	dir := pkg(t, src, "")
	if err := runBorn(dir, "between.kinds.json", "guatiao_kinds", false); err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Kinds []struct {
			Slots     json.RawMessage `json:"slots"`
			FloorHash uint32          `json:"floor_hash"`
		} `json:"kinds"`
	}
	text, err := os.ReadFile(filepath.Join(dir, "between.kinds.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(text, &doc); err != nil {
		t.Fatal(err)
	}
	for _, v := range file.Vectors {
		if v.Name != "between" {
			continue
		}
		var got, want any
		_ = json.Unmarshal(doc.Kinds[0].Slots, &got)
		_ = json.Unmarshal(v.Slots, &want)
		if !reflect.DeepEqual(got, want) || doc.Kinds[0].FloorHash != v.FloorHash {
			t.Fatalf("described:\n%s", text)
		}
		return
	}
	t.Fatal("no between vector")
}

// What the shared vectors refuse, the glue refuses: through the library
// for a description, by guatiao.Remote's names for a slot.
func TestTheGlueRefusesWhatTheSharedVectorsRefuse(t *testing.T) {
	var file struct {
		Refused []struct {
			Name   string            `json:"name"`
			Object bool              `json:"object"`
			Slots  json.RawMessage   `json:"slots"`
			With   []json.RawMessage `json:"with"`
			Why    string            `json:"why"`
			By     any               `json:"by"`
		} `json:"refused"`
	}
	if err := json.Unmarshal(repoFile(t, "crates", "guatiao-derive", "tests", "vectors", "floor_hash.json"), &file); err != nil {
		t.Fatal(err)
	}
	// The remote's own names are said in Go's spelling.
	why := map[string]string{"a remote's own": "a remote's own method"}
	ran := 0
	for _, v := range file.Refused {
		by, _ := v.By.([]any)
		if v.By != "every reader" && !slices.Contains(by, any("go")) {
			continue
		}
		doc := `{"kinds": [{"name": "` + v.Name + `", "floor_hash": 1, "object": ` + strconv.FormatBool(v.Object) +
			`, "slots": ` + string(v.Slots) + `}`
		for _, k := range v.With {
			doc += ", " + string(k)
		}
		doc += "]}"
		src := "package probe\n\n//guatiao:kind " + v.Name + "\ntype K interface {\n\tHas(name string) (bool, error)\n}\n"
		dir := pkg(t, src, "")
		if err := os.WriteFile(filepath.Join(dir, "k.kinds.json"), []byte(doc), 0o644); err != nil {
			t.Fatal(err)
		}
		says := v.Why
		if w, ok := why[says]; ok {
			says = w
		}
		if err := run(dir, "k.kinds.json", "guatiao_kinds", false); err == nil || !strings.Contains(err.Error(), says) {
			t.Errorf("%s: %v", v.Name, err)
		}
		ran++
	}
	if ran < 6 {
		t.Fatalf("%d refusals", ran)
	}
}

// A header is no description: what no trait declares is refused where the
// table is read, and the same table that can fail is read.
func TestAHeaderTableNoTraitDeclaresIsRefused(t *testing.T) {
	table := func(slot string) string {
		return "typedef struct w_vtable {\n  guatiao_kind_header header;\n" + slot +
			"\n} w_vtable;\n#define w_vtable_FLOOR_HASH 1\n"
	}
	for slot, says := range map[string]string{
		"  guatiao_status (*header)(void *ctx);":                                          "the table's first field",
		"  guatiao_status (*sum)(void *ctx, const guatiao_list *items, int64_t *out);":    "a list is not borrowed",
		"  guatiao_status (*read)(void *ctx, const guatiao_object *memo, uint64_t *out);": "on a fallible slot only",
		"  guatiao_status (*give)(void *ctx, guatiao_object memo);":                       "on a fallible slot only",
		"  guatiao_status (*open)(void *ctx, guatiao_object *out);":                       "on a fallible slot only",
	} {
		if _, err := readTable(table(slot), "w_vtable"); err == nil || !strings.Contains(err.Error(), says) {
			t.Errorf("%s: %v", slot, err)
		}
	}
	fine := "  guatiao_status (*open)(void *ctx, const guatiao_object *memo, guatiao_object *out, guatiao_provider_error *err);"
	if _, err := readTable(table(fine), "w_vtable"); err != nil {
		t.Fatal(err)
	}
}

// A header is no description: bytes written in place on a table with no
// destroy are refused where it is read.
func TestAHeaderTableWithBytesMutAndNoDestroyIsRefused(t *testing.T) {
	hdr := "typedef struct w_vtable {\n  guatiao_kind_header header;\n" +
		"  guatiao_status (*fill)(void *ctx, guatiao_bytes_mut dst, int64_t *out);\n} w_vtable;\n#define w_vtable_FLOOR_HASH 1\n"
	if _, err := readTable(hdr, "w_vtable"); err == nil || !strings.Contains(err.Error(), "on an object kind only") {
		t.Fatalf("%v", err)
	}
	object := strings.Replace(hdr, "header;\n", "header;\n  void (*destroy)(void *ctx);\n", 1)
	if _, err := readTable(object, "w_vtable"); err != nil {
		t.Fatal(err)
	}
}
