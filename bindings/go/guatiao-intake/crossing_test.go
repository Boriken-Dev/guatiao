// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package intake_test

// A tree crosses between guatiao_intake and guatiao: built by one library,
// read and freed through the other, every block accounted for.

import (
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake/internal/testenv"
)

func TestATreeTheCoreBuiltIsReadByIntake(t *testing.T) {
	c := testenv.NewCounting()
	defer c.Close()
	v, err := guatiao.NewValue(map[string]any{"agent": []any{map[string]any{"name": "a"}, map[string]any{"name": "b"}}},
		guatiao.InAllocator(c.Alloc()))
	if err != nil {
		t.Fatal(err)
	}
	found, ok, err := intake.At(v, "agent[1].name")
	if err != nil || !ok || found.StringOr("") != "b" {
		t.Fatalf("%v %v", ok, err)
	}
	v.Close()
	if _, err := found.Any(); err == nil {
		t.Fatal("a view read its value after it closed")
	}
	if live := c.Live(); live != 0 {
		t.Fatalf("%d blocks still out", live)
	}
}

func TestATreeIntakeBuiltIsChangedAndFreedThroughTheCore(t *testing.T) {
	schema, err := guatiao.SchemaOf[struct {
		Name string `guatiao:"name" schema:"x-section=who"`
	}]()
	if err != nil {
		t.Fatal(err)
	}
	defer schema.Close()
	form, err := intake.ForSchema(schema)
	if err != nil {
		t.Fatal(err)
	}
	m, err := form.Map()
	if err != nil {
		t.Fatal(err)
	}
	if err := m.Set("note", "added through the core"); err != nil {
		t.Fatal(err)
	}
	form.Close()
}

type listen struct {
	Host string `guatiao:"host" schema:"minLength=1,title=Host"`
	Port uint16 `guatiao:"port" schema:"default=5900"`
	Mode string `guatiao:"mode" schema:"enum=[\"fast\",\"safe\"]"`
}

func TestWhatAStructsSchemaSaysIsWhatIntakeChecks(t *testing.T) {
	s, err := guatiao.SchemaOf[listen]()
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	sections, err := intake.Layout(s, map[string]any{})
	if err != nil || len(sections) != 1 || len(sections[0].Fields) != 3 {
		t.Fatalf("%v %v", sections, err)
	}
}

func TestAProvidersFormFromItsMetaFitsItsSchema(t *testing.T) {
	reg, err := guatiao.NewRegistry("go-intake-tests", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	defer reg.Close()
	if _, err := reg.ScanDir(testenv.Workspace(), guatiao.Rules("kind=greeter")); err != nil {
		t.Fatal(err)
	}
	meta, ok, err := reg.ProviderMeta("derived_greeter_shouter")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	defer meta.Close()
	form := meta.Key(intake.ProviderFormKey)
	schema, ok, err := reg.ProviderConfig("derived_greeter_shouter")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	if why, err := intake.Check(schema, form); why != nil || err != nil {
		t.Fatalf("%v %v", why, err)
	}
	var fields map[string]map[string]any
	if err := form.Key("fields").Decode(&fields); err != nil {
		t.Fatal(err)
	}
	if fields["prefix"]["placeholder"] != "hey" || fields["marks"]["unit"] != "marks" {
		t.Fatalf("%v", fields)
	}
	for exclaim, want := range map[bool]bool{false: false, true: true} {
		shown, err := intake.IsVisible(schema, form, "marks", map[string]any{"exclaim": exclaim})
		if err != nil || shown != want {
			t.Fatalf("exclaim %v: %v %v", exclaim, shown, err)
		}
	}
}
