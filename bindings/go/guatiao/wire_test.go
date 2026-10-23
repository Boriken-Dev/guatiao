// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// The wire encoding and channels against the shared vectors every
// implementation reads: each value both ways, each malformed input
// refused, the frames the builders write, and a conversation with every
// refusal.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/testenv"
)

type wireVectors struct {
	Statuses map[string]string `json:"statuses"`
	Values   []struct {
		Name string `json:"name"`
		Tree any    `json:"tree"`
		Hex  string `json:"hex"`
	} `json:"values"`
	Malformed []struct {
		Name string      `json:"name"`
		Hex  string      `json:"hex"`
		At   json.Number `json:"at"`
	} `json:"malformed"`
	Frames []struct {
		Build   string      `json:"build"`
		Channel json.Number `json:"channel"`
		Tree    any         `json:"tree"`
		Hex     string      `json:"hex"`
	} `json:"frames"`
	Conversation []map[string]any `json:"conversation"`
}

func readWireVectors(t *testing.T) wireVectors {
	t.Helper()
	repo := testenv.Repo()
	if repo == "" {
		t.Skip("the shared vectors live in the repository")
	}
	text, err := os.ReadFile(filepath.Join(repo, "crates", "guatiao", "tests", "vectors", "wire.json"))
	if err != nil {
		t.Fatal(err)
	}
	dec := json.NewDecoder(bytes.NewReader(text))
	dec.UseNumber()
	var v wireVectors
	if err := dec.Decode(&v); err != nil {
		t.Fatal(err)
	}
	return v
}

func unhex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// describe is a node as the vector file writes a tree.
func describe(t *testing.T, r guatiao.Ref) any {
	t.Helper()
	switch r.Tag() {
	case guatiao.TagAbsent:
		return []any{"absent"}
	case guatiao.TagNull:
		return []any{"null"}
	case guatiao.TagBool:
		return []any{"bool", r.BoolOr(false)}
	case guatiao.TagNumber:
		n, _ := r.NumberText()
		return []any{"number", string(n)}
	case guatiao.TagString:
		s, _ := r.Text()
		return []any{"string", s}
	case guatiao.TagBytes:
		b, _ := r.Bytes()
		return []any{"bytes", hex.EncodeToString(b)}
	case guatiao.TagList:
		l, _ := r.List()
		items := []any{}
		for i := 0; i < l.Len(); i++ {
			item, _ := l.At(i)
			items = append(items, describe(t, item))
		}
		return []any{"list", items}
	default:
		m, err := r.Map()
		if err != nil {
			t.Fatal(err)
		}
		pairs := []any{}
		for k, item := range m.All() {
			pairs = append(pairs, []any{k, describe(t, item)})
		}
		return []any{"map", pairs}
	}
}

// build is the value a vector tree names.
func build(t *testing.T, tree any) *guatiao.Value {
	t.Helper()
	parts := tree.([]any)
	var v *guatiao.Value
	var err error
	switch parts[0] {
	case "absent":
		v = guatiao.Absent()
	case "null":
		v, err = guatiao.Null()
	case "bool":
		v, err = guatiao.NewValue(parts[1].(bool))
	case "number":
		v, err = guatiao.NewValue(guatiao.Number(parts[1].(string)))
	case "string":
		v, err = guatiao.NewValue(parts[1].(string))
	case "bytes":
		v, err = guatiao.NewValue(unhex(t, parts[1].(string)))
	case "list":
		v, err = guatiao.NewList()
		if err == nil {
			l, _ := v.List()
			for _, item := range parts[1].([]any) {
				child := build(t, item)
				if err := l.Push(child); err != nil {
					t.Fatal(err)
				}
				child.Close()
			}
		}
	default:
		v, err = guatiao.NewMap()
		if err == nil {
			m, _ := v.Map()
			for _, pair := range parts[1].([]any) {
				kv := pair.([]any)
				child := build(t, kv[1])
				if err := m.Set(kv[0].(string), child); err != nil {
					t.Fatal(err)
				}
				child.Close()
			}
		}
	}
	if err != nil {
		t.Fatalf("building %v: %v", tree, err)
	}
	t.Cleanup(func() { v.Close() })
	return v
}

// sameTree compares a described node with a vector tree, the JSON reader
// having read neither numbers nor anything else differently.
func sameTree(got, want any) bool {
	g, _ := json.Marshal(got)
	w, _ := json.Marshal(want)
	return bytes.Equal(g, w)
}

func TestEveryWireValueRoundTrips(t *testing.T) {
	v := readWireVectors(t)
	if len(v.Values) == 0 {
		t.Fatal("no values")
	}
	for _, c := range v.Values {
		data := unhex(t, c.Hex)
		got, err := guatiao.FromWire(data)
		if err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		if d := describe(t, got.Ref); !sameTree(d, c.Tree) {
			t.Errorf("%s: decoded %v, want %v", c.Name, d, c.Tree)
		}
		if back, err := got.ToWire(); err != nil || !bytes.Equal(back, data) {
			t.Errorf("%s: re-encoded %x (%v)", c.Name, back, err)
		}
		got.Close()
		if back, err := build(t, c.Tree).ToWire(); err != nil || !bytes.Equal(back, data) {
			t.Errorf("%s: built and encoded %x (%v)", c.Name, back, err)
		}
	}
}

func TestEveryMalformedWireInputIsRefused(t *testing.T) {
	for _, c := range readWireVectors(t).Malformed {
		v, err := guatiao.FromWire(unhex(t, c.Hex))
		if !errors.Is(err, guatiao.ErrBadValue) {
			t.Errorf("%s: %v, %v", c.Name, v, err)
		}
	}
}

func TestAChildEncodesAlone(t *testing.T) {
	v := mustValue(t, map[string]any{"outer": map[string]any{"inner": []any{1, "two"}}})
	data, err := v.Key("outer").ToWire()
	if err != nil {
		t.Fatal(err)
	}
	back, err := guatiao.FromWire(data)
	if err != nil {
		t.Fatal(err)
	}
	defer back.Close()
	if got := plain(t, back); !reflect.DeepEqual(got, map[string]any{"inner": []any{guatiao.Number("1"), "two"}}) {
		t.Errorf("%#v", got)
	}
}

func TestTheFrameBuildersWriteTheDocumentedBytes(t *testing.T) {
	v := readWireVectors(t)
	for _, f := range v.Frames {
		var channel uint64
		if _, err := fmt.Sscan(f.Channel.String(), &channel); err != nil {
			t.Fatal(err)
		}
		var got []byte
		var err error
		switch f.Build {
		case "announce":
			got, err = guatiao.WireAnnounce(channel, build(t, f.Tree))
		case "send":
			got, err = guatiao.WireSend(channel, build(t, f.Tree))
		default:
			got, err = guatiao.WireClose(channel)
		}
		if err != nil || hex.EncodeToString(got) != f.Hex {
			t.Errorf("%s %d: %x (%v), want %s", f.Build, channel, got, err, f.Hex)
		}
	}
	got, err := guatiao.WireSend(7, map[string]any{"greeting": "hello"})
	if err != nil || hex.EncodeToString(got) != v.Frames[1].Hex {
		t.Errorf("plain data: %x (%v)", got, err)
	}
}

func TestTheConversationRunsThroughOneReceiver(t *testing.T) {
	v := readWireVectors(t)
	sentinels := map[string]error{
		"GUATIAO_ERR_BAD_VALUE":  guatiao.ErrBadValue,
		"GUATIAO_ERR_NOT_FOUND":  guatiao.ErrNotFound,
		"GUATIAO_ERR_WRONG_KIND": guatiao.ErrWrongKind,
	}
	refusals := map[string]guatiao.Refusal{
		"malformed":    guatiao.RefusalMalformed,
		"no_schema":    guatiao.RefusalNoSchema,
		"not_a_schema": guatiao.RefusalNotASchema,
		"invalid":      guatiao.RefusalInvalid,
	}
	number := func(step map[string]any, key string) (uint64, bool) {
		n, ok := step[key].(json.Number)
		if !ok {
			return 0, false
		}
		var u uint64
		fmt.Sscan(n.String(), &u)
		return u, true
	}
	r, err := guatiao.NewReceiver()
	if err != nil {
		t.Fatal(err)
	}
	for i, step := range v.Conversation {
		frame := unhex(t, step["frame"].(string))
		channel, _ := number(step, "channel")
		event, err := r.Accept(frame)
		if name, ok := step["event"].(string); ok {
			if err != nil {
				t.Fatalf("step %d: %v", i, err)
			}
			if event.Kind.String() != name || event.Channel != channel {
				t.Errorf("step %d: %v on %d", i, event.Kind, event.Channel)
			}
			if tree, ok := step["tree"]; ok {
				if d := describe(t, event.Value.Ref); !sameTree(d, tree) {
					t.Errorf("step %d: %v, want %v", i, d, tree)
				}
				event.Value.Close()
			} else if event.Value != nil {
				t.Errorf("step %d: a value on a %v", i, event.Kind)
			}
			continue
		}
		name := step["refusal"].(string)
		var refused *guatiao.Refused
		if !errors.As(err, &refused) {
			t.Fatalf("step %d: %v, want a refusal", i, err)
		}
		if !errors.Is(err, sentinels[v.Statuses[name]]) || refused.Refusal != refusals[name] {
			t.Errorf("step %d: %v (%v), want %s", i, err, refused.Refusal, name)
		}
		var plainErr *guatiao.Error
		if !errors.As(err, &plainErr) || plainErr.Status != refused.Status {
			t.Errorf("step %d: not an *Error of its status", i)
		}
		if refused.Channel != channel || refused.Message == "" || strings.Contains(err.Error(), "hunter2") {
			t.Errorf("step %d: %+v", i, refused)
		}
		if kind, ok := number(step, "kind"); ok && uint64(refused.Kind) != kind {
			t.Errorf("step %d: kind %d", i, refused.Kind)
		}
		if at, ok := number(step, "at"); ok && uint64(refused.At) != at {
			t.Errorf("step %d: at %d, want %d", i, refused.At, at)
		}
		if key, ok := step["key"].(string); ok && refused.Key != key {
			t.Errorf("step %d: key %q, want %q", i, refused.Key, key)
		}
	}
	r.Close()
	r.Close()
	if _, err := r.Accept([]byte{2, 1}); !errors.Is(err, guatiao.ErrClosed) {
		t.Errorf("a closed receiver answered %v", err)
	}
}

func TestTheFrameAndRefusalCodesAreTheHeaders(t *testing.T) {
	text, err := os.ReadFile("internal/native/include/guatiao.h")
	if err != nil {
		t.Fatal(err)
	}
	for name, code := range map[string]uint32{
		"FRAME_SCHEMA":      uint32(guatiao.FrameSchema),
		"FRAME_VALUE":       uint32(guatiao.FrameValue),
		"FRAME_CLOSE":       uint32(guatiao.FrameClose),
		"WIRE_MALFORMED":    uint32(guatiao.RefusalMalformed),
		"WIRE_NO_SCHEMA":    uint32(guatiao.RefusalNoSchema),
		"WIRE_NOT_A_SCHEMA": uint32(guatiao.RefusalNotASchema),
		"WIRE_INVALID":      uint32(guatiao.RefusalInvalid),
	} {
		if line := fmt.Sprintf("#define GUATIAO_%s %d\n", name, code); !strings.Contains(string(text), line) {
			t.Errorf("the header has no %q", line)
		}
	}
}
