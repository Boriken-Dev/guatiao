// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guest

import (
	"strings"
	"testing"
)

// word is an instance greeting with the word it was configured with.
type word struct {
	word     string
	released *int
}

func (w *word) Call(c *Call) (any, error) {
	name, err := c.Text(0)
	if err != nil {
		return nil, err
	}
	return w.word + ", " + name, nil
}

func (w *word) Release() { *w.released++ }

func configuration(t *testing.T, w string) []byte {
	t.Helper()
	b, err := Encode(NewMap().Set("word", w))
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func lastText(t *testing.T, got *told) string {
	t.Helper()
	v, err := Decode(got.answers[len(got.answers)-1])
	if err != nil {
		t.Fatal(err)
	}
	s, _ := v.(string)
	return s
}

func TestInstancesAreBuiltCalledAndDestroyedAndAvailabilityIsAnswered(t *testing.T) {
	got := standIn(t)
	released := 0
	builder := func(config any) (Handler, error) {
		m, _ := config.(*Map)
		w, _ := m.Get("word")
		if s, _ := w.(string); s != "" {
			return &word{word: s, released: &released}, nil
		}
		return nil, Refuse(StatusBadValue, "no word")
	}
	schema := NewMap().Set("type", "object")
	Serve(&Library{Providers: []Provider{
		{ID: "counter", Handler: &counter{}},
		{ID: "tuned", Handler: &counter{}, Config: schema, Build: builder, Available: func() error { return nil }},
		{ID: "only", Config: schema, Build: builder, InstancesOnly: true,
			Available: func() error { return Refuse(StatusUnsupported, "asleep") }},
	}})
	if declare() != 0 {
		t.Fatal("declared")
	}
	d, _ := Decode(got.answers[0])
	providers, _ := d.(*Map).Get("providers")
	for i, want := range [][]string{
		{"id", "name", "serves"},
		{"id", "name", "serves", "config", "may_be_unavailable"},
		{"id", "name", "serves", "config", "instances_only", "may_be_unavailable"},
	} {
		keys := providers.([]any)[i].(*Map).Keys()
		if strings.Join(keys, ",") != strings.Join(want, ",") {
			t.Errorf("provider %d declares %v", i, keys)
		}
	}

	ids := []int64{}
	for _, c := range []struct {
		provider int64
		word     string
	}{{1, "howdy"}, {2, "hey"}} {
		if s := create(c.provider, configuration(t, c.word)); s != 0 {
			t.Fatalf("create: status %d", s)
		}
		v, _ := Decode(got.answers[len(got.answers)-1])
		n, _ := v.(Number).Int()
		ids = append(ids, n)
	}
	if ids[0] == ids[1] {
		t.Fatalf("one id for two instances: %v", ids)
	}
	for i, want := range []string{"howdy, ana", "hey, ana"} {
		s := call(targetInstance, ids[i], request(t, "greeter", "greet", "ana"))
		if s != 0 || lastText(t, got) != want {
			t.Errorf("status %d, %q", s, lastText(t, got))
		}
	}
	destroy(ids[0])
	if released != 1 {
		t.Errorf("released %d", released)
	}
	if s := call(targetInstance, ids[0], request(t, "greeter", "greet", "bo")); s != uint32(StatusGone) {
		t.Errorf("destroyed, then %d", s)
	}
	s := call(targetInstance, ids[1], request(t, "greeter", "greet", "bo"))
	if s != 0 || lastText(t, got) != "hey, bo" {
		t.Errorf("the other instance: %d", s)
	}
	if s := call(targetProvider, 2, request(t, "greeter", "greet", "bo")); s != uint32(StatusUnsupported) {
		t.Errorf("instances only, called as itself: %d", s)
	}

	s = create(1, configuration(t, ""))
	if s != uint32(StatusBadValue) || string(got.answers[len(got.answers)-1]) != "no word" {
		t.Errorf("refused: %d", s)
	}
	if s := create(0, configuration(t, "x")); s != uint32(StatusUnsupported) {
		t.Errorf("builds no instances: %d", s)
	}
	if s := create(9, configuration(t, "x")); s != uint32(StatusNotFound) {
		t.Errorf("no provider 9: %d", s)
	}

	before := len(got.answers)
	if available(0) != 0 || available(1) != 0 || len(got.answers) != before {
		t.Errorf("available: nothing answered")
	}
	if s := available(2); s != 0 || lastText(t, got) != "asleep" {
		t.Errorf("unavailable: %d", s)
	}
	if s := available(9); s != uint32(StatusNotFound) {
		t.Errorf("no provider 9: %d", s)
	}
}
