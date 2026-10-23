// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package intake_test

import (
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/intake"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/intake/internal/testenv"
)

func TestMain(m *testing.M) {
	testenv.Setup()
	var err error
	if flatSchema, err = testenv.FromJSON(flatSchemaJSON); err != nil {
		panic(err)
	}
	code := m.Run()
	flatSchema.Close()
	os.Exit(code)
}

var schema = map[string]any{
	"type":                 "object",
	"properties":           map[string]any{"name": map[string]any{"type": "string", "title": "Name"}},
	"required":             []any{"name"},
	"additionalProperties": false,
}

// A Go map literal has no order and is written sorted, so a schema whose
// declaration order matters is written as JSON.
const flatSchemaJSON = `{
  "type": "object",
  "properties": {
    "auth": {
      "type": "object",
      "x-variant-tag": "auth",
      "oneOf": [
        {"type": "object", "properties": {"auth": {"const": "sso"}}, "required": ["auth"]},
        {"type": "object", "properties": {
          "auth": {"const": "userpass"},
          "username": {"type": "string"},
          "password": {"type": "string", "x-sensitive": true}
        }, "required": ["auth"]}
      ]
    },
    "agent": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},
    "env": {"type": "object", "additionalProperties": {"type": "string"}},
    "host": {"type": "string", "title": "Host"}
  }
}`

var flatSchema *guatiao.Value

func value(t *testing.T, x any) *guatiao.Value {
	t.Helper()
	v, err := guatiao.NewValue(x)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { v.Close() })
	return v
}

func TestChecksAFormAgainstItsSchema(t *testing.T) {
	why, err := intake.Check(schema, map[string]any{})
	if err != nil || why != nil {
		t.Fatalf("%v %v", why, err)
	}
	why, err = intake.Check(schema, map[string]any{"sections": []any{}, "fields": map[string]any{"nope": map[string]any{}}})
	if err != nil || why["kind"] != "unknown_field" {
		t.Fatalf("%v %v", why, err)
	}
}

func TestLaysTheFieldsOutAndSaysWhichAreShown(t *testing.T) {
	s := value(t, schema)
	sections, err := intake.Layout(s, map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	var fields []string
	for _, sec := range sections {
		fields = append(fields, sec.Fields...)
	}
	if !reflect.DeepEqual(fields, []string{"name"}) {
		t.Fatalf("%v", fields)
	}
	shown, err := intake.IsVisible(s, map[string]any{}, "name", map[string]any{"name": "ana"})
	if err != nil || !shown {
		t.Fatalf("%v %v", shown, err)
	}
	if _, err := intake.IsVisible(s, map[string]any{}, "nope", map[string]any{}); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestNamesOnePlaceInsideAValueWithAPath(t *testing.T) {
	v := value(t, map[string]any{
		"agent": []any{map[string]any{"name": "one"}, map[string]any{"name": "two"}},
		"env":   map[string]any{"PATH": "/bin", "a.b": "dotted", "1": "keyed"},
	})
	for path, want := range map[string]string{
		"agent[0].name": "one", "agent[1].name": "two", "env[PATH]": "/bin", "env[a.b]": "dotted", `env["1"]`: "keyed",
	} {
		r, ok, err := intake.At(v, path)
		if err != nil || !ok || r.StringOr("") != want {
			t.Errorf("%s: %v %v %q", path, ok, err, r.StringOr(""))
		}
	}
	for _, path := range []string{"agent[9].name", "nonesuch", "agent["} {
		if _, ok, err := intake.At(v, path); ok || err != nil {
			t.Errorf("%s names something: %v", path, err)
		}
	}
	if _, _, err := intake.At(map[string]any{}, "a"); err == nil {
		t.Errorf("At takes a value, not plain data")
	}
}

func TestMakesAFormOutOfASchema(t *testing.T) {
	s := value(t, map[string]any{
		"type": "object",
		"properties": map[string]any{
			"host": map[string]any{"type": "string", "x-section": "net"},
			"tls": map[string]any{"type": "object", "properties": map[string]any{
				"verify": map[string]any{"type": "boolean", "x-section": "trust"}}, "additionalProperties": false},
			"label": map[string]any{"type": "string"},
		},
		"additionalProperties": false,
	})
	made, err := intake.ForSchema(s)
	if err != nil {
		t.Fatal(err)
	}
	defer made.Close()
	if id := made.Key("sections").Index(0).Key("id").StringOr(""); id != "net" {
		t.Fatalf("section %q", id)
	}
	if why, err := intake.Check(s, made); why != nil || err != nil {
		t.Fatalf("%v %v", why, err)
	}
	implied, err := intake.FormFor(map[string]any{}, s, "tls")
	if err != nil || implied == nil {
		t.Fatalf("%v", err)
	}
	implied.Close()
	for _, path := range []string{"label", "nonesuch"} {
		if f, err := intake.FormFor(map[string]any{}, s, path); f != nil || err != nil {
			t.Fatalf("%s: %v", path, err)
		}
	}
}

func TestResolvesTheFieldThatGovernsAFlatKey(t *testing.T) {
	s := value(t, flatSchema)
	host, ok, err := intake.Resolve(s, "host")
	if err != nil || !ok || host.Key("title").StringOr("") != "Host" {
		t.Fatalf("%v %v", ok, err)
	}
	pw, ok, _ := intake.Resolve(s, "auth.password")
	if !ok || !pw.Key("x-sensitive").BoolOr(false) {
		t.Fatal("auth.password")
	}
	if _, ok, _ := intake.Resolve(s, "nonesuch"); ok {
		t.Fatal("nonesuch")
	}
}

func TestFlatKeysFlattenAndUnflatten(t *testing.T) {
	keys, err := intake.FlatKeys(flatSchema, "auth")
	if err != nil || !reflect.DeepEqual(keys, []string{"auth", "auth.username", "auth.password"}) {
		t.Fatalf("%v %v", keys, err)
	}
	if _, err := intake.FlatKeys(flatSchema, "nonesuch"); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("%v", err)
	}
	cases := []struct {
		key   string
		value any
		flat  map[string]string
	}{
		{"auth", map[string]any{"auth": "userpass", "username": "alice", "password": "s3cret"},
			map[string]string{"auth": "userpass", "auth.username": "alice", "auth.password": "s3cret"}},
		{"auth", map[string]any{"auth": "sso"}, map[string]string{"auth": "sso"}},
		{"agent", []any{map[string]any{"name": "one"}, map[string]any{"name": "two"}},
			map[string]string{"agent[0].name": "one", "agent[1].name": "two"}},
		{"env", map[string]any{"PATH": "/bin", "a.b": "dotted"}, map[string]string{"env[PATH]": "/bin", "env[a.b]": "dotted"}},
		{"host", "example.org", map[string]string{"host": "example.org"}},
	}
	for _, c := range cases {
		flat, err := intake.Flatten(flatSchema, c.key, c.value)
		if err != nil || !reflect.DeepEqual(flat, c.flat) {
			t.Errorf("Flatten %s: %v %v", c.key, flat, err)
			continue
		}
		back, err := intake.Unflatten(flatSchema, c.key, flat)
		if err != nil || back == nil {
			t.Errorf("Unflatten %s: %v", c.key, err)
			continue
		}
		got, _ := back.Any()
		back.Close()
		want := value(t, c.value)
		wantPlain, _ := want.Any()
		if !reflect.DeepEqual(got, wantPlain) {
			t.Errorf("Unflatten %s: %#v", c.key, got)
		}
	}
}

func TestUnflattenAnswersNilWhereAStoreDoesNotApply(t *testing.T) {
	store := value(t, map[string]any{"env[PATH]": "/bin"})
	env, err := intake.Unflatten(flatSchema, "env", store)
	if err != nil || env == nil || env.Key("PATH").StringOr("") != "/bin" {
		t.Fatalf("%v", err)
	}
	env.Close()
	for _, c := range []struct {
		key   string
		store map[string]string
	}{{"auth", map[string]string{"auth": "nonesuch"}}, {"nonesuch", map[string]string{}}, {"host", map[string]string{}}} {
		if v, err := intake.Unflatten(flatSchema, c.key, c.store); v != nil || err != nil {
			t.Errorf("%s: %v %v", c.key, v, err)
		}
	}
	numbers := value(t, map[string]any{"host": 1})
	for _, bad := range []any{map[string]any{"host": 1}, []string{"host"}, numbers} {
		if _, err := intake.Unflatten(flatSchema, "host", bad); !errors.Is(err, guatiao.ErrWrongKind) {
			t.Errorf("%#v: %v", bad, err)
		}
	}
	if _, err := intake.Flatten(flatSchema, "auth", []int{1}); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Errorf("%v", err)
	}
}

func TestABorrowedViewIsGoneOnceItsSourceIsClosed(t *testing.T) {
	s, _ := guatiao.NewValue(flatSchema)
	host, _, _ := intake.Resolve(s, "host")
	v, _ := guatiao.NewValue(map[string]any{"a": map[string]any{"b": strings.Repeat("c", 50)}})
	view, _, _ := intake.At(v, "a")
	s.Close()
	v.Close()
	var junk []*guatiao.Value
	for i := 0; i < 200; i++ {
		j, _ := guatiao.NewValue(map[string]any{strings.Repeat("k", 40): strings.Repeat("x", 64)})
		junk = append(junk, j)
	}
	if _, err := host.Any(); !errors.Is(err, guatiao.ErrClosed) {
		t.Errorf("host: %v", err)
	}
	if _, err := view.Any(); !errors.Is(err, guatiao.ErrClosed) {
		t.Errorf("view: %v", err)
	}
	for _, j := range junk {
		j.Close()
	}
}

func TestFollowsAChangeToItsSourceAndChangesNothingThroughIt(t *testing.T) {
	s := value(t, flatSchema)
	host, _, _ := intake.Resolve(s, "host")
	props, _ := s.Key("properties").Map()
	if err := props.Set("host", map[string]any{"title": "Moved"}); err != nil {
		t.Fatal(err)
	}
	if got := host.Key("title").StringOr(""); got != "Moved" {
		t.Fatalf("%q", got)
	}
	m, _ := host.Map()
	for _, err := range []error{m.Set("title", "x"), m.Clear(), m.CopyFrom(host)} {
		if !errors.Is(err, guatiao.ErrReadOnly) {
			t.Errorf("%v", err)
		}
	}
	if _, err := m.Delete("title"); !errors.Is(err, guatiao.ErrReadOnly) {
		t.Errorf("%v", err)
	}
	v := value(t, map[string]any{"tags": []any{"a", "b"}})
	tagsRef, _, _ := intake.At(v, "tags")
	tags, _ := tagsRef.List()
	for _, err := range []error{tags.Push("c"), tags.RemoveAt(0), tags.Set(0, "z"), tags.Insert(0, "z"), tags.Clear()} {
		if !errors.Is(err, guatiao.ErrReadOnly) {
			t.Errorf("%v", err)
		}
	}
	c, err := tags.Clone()
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	if got, _ := c.Any(); !reflect.DeepEqual(got, []any{"a", "b"}) {
		t.Fatalf("%#v", got)
	}
}

func TestTakesAViewWhereItTakesAValue(t *testing.T) {
	config := value(t, map[string]any{"host": "h", "env": map[string]any{"PATH": "/bin"}})
	flat, err := intake.Flatten(flatSchema, "env", config.Key("env"))
	if err != nil || !reflect.DeepEqual(flat, map[string]string{"env[PATH]": "/bin"}) {
		t.Fatalf("%v %v", flat, err)
	}
	holder := value(t, map[string]any{"flat": map[string]any{"env[PATH]": "/bin"}})
	back, err := intake.Unflatten(flatSchema, "env", holder.Key("flat"))
	if err != nil || back == nil {
		t.Fatalf("%v", err)
	}
	defer back.Close()
	if back.Key("PATH").StringOr("") != "/bin" {
		t.Fatal("PATH")
	}
}
