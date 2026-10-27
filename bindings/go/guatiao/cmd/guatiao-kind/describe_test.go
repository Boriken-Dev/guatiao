// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
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
//guatiao:appended Filler
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
		"a borrowed list":            {"\t//guatiao:taken items\n", "", "a list is not borrowed: mark it taken"},
		"capitals in a row":          {"Fill(config", "FillIO(config", "capitals in a row"},
		"an appended non-interface":  {"//guatiao:appended Filler", "//guatiao:appended Filer", "Filer, which is no interface"},
		"an appended kind":           {"//guatiao:appended Filler", "//guatiao:appended Ear", "a kind of its own"},
		"another package's type":     {"items []string", "items time.Duration", "another package's type"},
		"no error last":              {"Heard(what string) error", "Heard(what string)", "returns an error last"},
		"a taken view":               {"//guatiao:nullable v\n\t//guatiao:taken items", "//guatiao:taken v items", "a view is borrowed"},
		"a lent string":              {"//guatiao:infallible\n\tHeard", "//guatiao:lent what\n\tHeard", "cannot be marked lent"},
		"a provider kind passed":     {"ear Ear) (*", "ear Probe) (*", "no object kind"},
		"a floor hash not a number":  {"//guatiao:floorhash 42", "//guatiao:floorhash x", "32-bit unsigned integer"},
		"an appended slot twice":     {"//guatiao:appended Filler", "//guatiao:appended Filler Filler", "two slots of probe are fill"},
		"an appended kind method":    {"\tFill(config", "\tRead(config", "Filler.Read is a method of Probe already"},
		"an appended destroy":        {"// Hears.\n//\n//guatiao:kind ear", "type Closer interface{ Close() error }\n\n// Hears.\n//\n//guatiao:kind ear\n//guatiao:appended Closer", "Closer.Close is a method of Ear already"},
		"a method declared twice":    {"// Fills in place.", "type Other interface{ Fill(config guatiao.ListRef, ear Ear) (bool, error) }\n\n// Fills in place.", "Filler and Other both have Fill"},
		"a kind named twice":         {"//guatiao:kind probe", "//guatiao:kind ear", "both the kind ear"},
		"a slot directive misplaced": {"// Fills in place.", "// Fills in place.\n//\n//guatiao:infallible", "a directive of a slot goes on its method"},
		"a map keyed by numbers":     {"items []string", "items map[int]string", "a map's keys are text"},
		"an argument with no name":   {"Heard(what string) error", "Heard(string) error", "every argument has a name"},
		"an underscore in a method":  {"Fill(config", "Fill_it(config", "no slot's name in CamelCase"},
		"an empty mark":              {"//guatiao:lent config ear", "//guatiao:lent", "names what it marks"},
		"a destroy slot":             {"\tHeard(what string) error\n", "\tHeard(what string) error\n\tDestroy() error\n", "destroy is the slot"},
		"a remote's own method":      {"Fill(config", "Has(config", "a remote has Has of its own"},
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
