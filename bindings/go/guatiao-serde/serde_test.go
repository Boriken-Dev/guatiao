// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package serde_test

import (
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde/internal/testenv"
)

func TestMain(m *testing.M) {
	testenv.Setup()
	os.Exit(m.Run())
}

func parse(t *testing.T, text string, f serde.Format) *guatiao.Value {
	t.Helper()
	v, err := serde.Parse(text, f)
	if err != nil {
		t.Fatalf("Parse(%q): %v", text, err)
	}
	t.Cleanup(func() { v.Close() })
	return v
}

func TestJSONKeepsKeyOrderAndIntegers(t *testing.T) {
	doc, err := guatiao.NewValue(map[string]any{"z": 3, "a": 1, "list": []any{1, "two", nil, true}})
	if err != nil {
		t.Fatal(err)
	}
	defer doc.Close()
	text, err := serde.Emit(doc, serde.JSON)
	if err != nil {
		t.Fatal(err)
	}
	back := parse(t, `{"z":3,"a":1,"list":[1,"two",null,true]}`, serde.JSON)
	m, _ := back.Map()
	if !reflect.DeepEqual(m.Keys(), []string{"z", "a", "list"}) {
		t.Fatalf("keys %v", m.Keys())
	}
	again, _ := serde.Emit(back, serde.JSON)
	if again != `{"z":3,"a":1,"list":[1,"two",null,true]}` {
		t.Fatalf("emitted %s", again)
	}
	if text != `{"a":1,"list":[1,"two",null,true],"z":3}` {
		t.Fatalf("a Go map is written sorted: %s", text)
	}
}

func TestKeepsTheExactTextOfANumber(t *testing.T) {
	v := parse(t, `{"n": 1.10, "big": 1180591620717411303424, "e": 1e400}`, serde.JSON)
	for key, want := range map[string]string{"n": "1.10", "big": "1180591620717411303424", "e": "1e+400"} {
		if got, _ := v.Key(key).NumberText(); string(got) != want {
			t.Errorf("%s: %q", key, got)
		}
	}
}

func TestIndentsWhenAsked(t *testing.T) {
	plain, _ := serde.Emit(map[string]any{"a": 1}, serde.JSON)
	pretty, _ := serde.Emit(map[string]any{"a": 1}, serde.JSON, serde.Pretty())
	if len(pretty) <= len(plain) || !strings.Contains(pretty, "\n") {
		t.Fatalf("%q", pretty)
	}
}

func TestTOMLAndYAMLRoundTrip(t *testing.T) {
	for _, f := range []serde.Format{serde.TOML, serde.YAML} {
		text, err := serde.Emit(map[string]any{"a": 1, "b": "x"}, f)
		if err != nil {
			t.Fatalf("%s: %v", f, err)
		}
		back := parse(t, text, f)
		got, _ := back.Any()
		if !reflect.DeepEqual(got, map[string]any{"a": guatiao.Number("1"), "b": "x"}) {
			t.Fatalf("%s: %#v", f, got)
		}
	}
}

func TestRefusesATOMLDocumentThatIsNotAMap(t *testing.T) {
	if _, err := serde.Emit([]int{1, 2}, serde.TOML); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("%v", err)
	}
	if _, err := serde.Emit(1, "xml"); err == nil {
		t.Fatal("an unknown format")
	}
}

func TestSaysWhereADocumentStoppedParsing(t *testing.T) {
	_, err := serde.Parse("{\"a\":\n 1,,}", serde.JSON)
	var pe *serde.ParseError
	if !errors.As(err, &pe) || !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	if pe.Format != serde.JSON || pe.Line != 2 || pe.Column != 4 || pe.Reason == "" {
		t.Fatalf("%#v", pe)
	}
	if !strings.Contains(pe.Error(), "line 2, column 4") {
		t.Fatalf("%s", pe)
	}
	_, err = serde.Parse("a = = 1", serde.TOML)
	if !errors.As(err, &pe) || pe.Format != serde.TOML || pe.Line != 1 || pe.Column != 5 {
		t.Fatalf("toml: %v", err)
	}
	_, err = serde.Parse("a: [1", serde.YAML)
	if !errors.As(err, &pe) || pe.Format != serde.YAML || pe.Line != 1 {
		t.Fatalf("yaml: %v", err)
	}
}

func TestEmitsPlainDataAsItIs(t *testing.T) {
	text, err := serde.Emit(map[string]any{"a": []any{1, true}}, serde.JSON)
	if err != nil || text != `{"a":[1,true]}` {
		t.Fatalf("%q %v", text, err)
	}
	type point struct {
		X, Y int
	}
	text, err = serde.Emit(point{1, 2}, serde.JSON)
	if err != nil || text != `{"X":1,"Y":2}` {
		t.Fatalf("%q %v", text, err)
	}
}
