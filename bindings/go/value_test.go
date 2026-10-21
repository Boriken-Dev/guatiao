// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

import (
	"errors"
	"math"
	"os"
	"reflect"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/testenv"
)

func TestMain(m *testing.M) {
	testenv.Setup()
	os.Exit(m.Run())
}

func mustValue(t *testing.T, x any) *guatiao.Value {
	t.Helper()
	v, err := guatiao.NewValue(x)
	if err != nil {
		t.Fatalf("NewValue(%#v): %v", x, err)
	}
	t.Cleanup(func() { v.Close() })
	return v
}

func plain(t *testing.T, r interface{ Any() (any, error) }) any {
	t.Helper()
	x, err := r.Any()
	if err != nil {
		t.Fatalf("Any: %v", err)
	}
	return x
}

func TestEveryKindRoundTrips(t *testing.T) {
	v := mustValue(t, map[string]any{
		"null": nil, "bool": true, "int": 5900, "neg": -7, "float": 1.5,
		"text": "héllo", "bytes": []byte{0, 1, 255}, "list": []any{1, "two", nil},
		"map": map[string]any{"k": []any{}},
	})
	got := plain(t, v)
	want := map[string]any{
		"null": nil, "bool": true, "int": guatiao.Number("5900"), "neg": guatiao.Number("-7"),
		"float": guatiao.Number("1.5"), "text": "héllo", "bytes": []byte{0, 1, 255},
		"list": []any{guatiao.Number("1"), "two", nil}, "map": map[string]any{"k": []any{}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v\nwant %#v", got, want)
	}
	if v.Tag() != guatiao.TagMap {
		t.Fatalf("tag %v", v.Tag())
	}
}

func TestRefusesWhatHasNoValueForm(t *testing.T) {
	for _, bad := range []any{
		make(chan int), func() {}, complex(1, 2), math.NaN(), math.Inf(1),
		map[[2]int]int{{1, 2}: 3}, "\xff\xfe", map[string]any{"\xff": 1},
	} {
		v, err := guatiao.NewValue(bad)
		if err == nil {
			v.Close()
			t.Errorf("NewValue(%#v) took it", bad)
			continue
		}
		if !errors.Is(err, guatiao.ErrBadValue) {
			t.Errorf("NewValue(%#v): %v is not a bad value", bad, err)
		}
	}
	type self struct{ Next *self }
	loop := &self{}
	loop.Next = loop
	if _, err := guatiao.NewValue(loop); err == nil || !strings.Contains(err.Error(), "itself") {
		t.Errorf("a cycle: %v", err)
	}
}

func TestKeepsANumberExact(t *testing.T) {
	v := mustValue(t, map[string]any{
		"exact":   guatiao.Number("1.10"),
		"u64":     uint64(math.MaxUint64),
		"i64":     int64(math.MinInt64),
		"huge":    guatiao.Number("123456789012345678901234567890.5"),
		"zero":    math.Copysign(0, -1),
		"tiny":    1e-7,
		"float32": float32(0.1),
	})
	m, _ := v.Map()
	for key, want := range map[string]string{
		"exact": "1.10", "u64": "18446744073709551615", "i64": "-9223372036854775808",
		"huge": "123456789012345678901234567890.5", "zero": "-0", "tiny": "1e-07", "float32": "0.1",
	} {
		ref, _ := m.Get(key)
		got, ok := ref.NumberText()
		if !ok || string(got) != want {
			t.Errorf("%s: %q, want %q", key, got, want)
		}
	}
	if _, err := guatiao.NewValue(guatiao.Number("1.2.3")); !errors.Is(err, guatiao.ErrBadValue) {
		t.Errorf("a number that is not one: %v", err)
	}
}

func TestReadsOneFieldWithTheHeaderRules(t *testing.T) {
	v := mustValue(t, map[string]any{"n": 42, "f": 2.5, "e": guatiao.Number("1e2"), "big": guatiao.Number("9223372036854775808"),
		"s": "x", "b": true})
	if got := v.Key("n").IntOr(-1); got != 42 {
		t.Errorf("IntOr n %d", got)
	}
	for _, key := range []string{"f", "e", "big", "s", "missing"} {
		if got := v.Key(key).IntOr(-1); got != -1 {
			t.Errorf("IntOr %s answered %d, not the fallback", key, got)
		}
	}
	if got := v.Key("f").FloatOr(0); got != 2.5 {
		t.Errorf("FloatOr %v", got)
	}
	if got := v.Key("s").StringOr("?"); got != "x" {
		t.Errorf("StringOr %q", got)
	}
	if got := v.Key("n").StringOr("?"); got != "?" {
		t.Errorf("a number is no string: %q", got)
	}
	if !v.Key("b").BoolOr(false) || v.Key("n").BoolOr(false) {
		t.Errorf("BoolOr")
	}
	if !v.Key("missing").IsAbsent() || v.Key("n").Index(0).Tag() != guatiao.TagAbsent {
		t.Errorf("a path naming nothing is absent")
	}
}

func TestARefStaysValidWhileTheMapGrows(t *testing.T) {
	v := mustValue(t, map[string]any{"first": map[string]any{"k": "v"}})
	inner := v.Key("first").Key("k")
	m, _ := v.Map()
	for i := 0; i < 200; i++ {
		if err := m.Set("k"+strings.Repeat("x", i), i); err != nil {
			t.Fatal(err)
		}
	}
	if got := inner.StringOr(""); got != "v" {
		t.Fatalf("after growth %q", got)
	}
}

func TestUseAfterCloseIsRefused(t *testing.T) {
	v, err := guatiao.NewValue(map[string]any{"a": 1})
	if err != nil {
		t.Fatal(err)
	}
	ref := v.Key("a")
	m, _ := v.Map()
	v.Close()
	v.Close()
	if !v.Closed() || ref.Tag() != guatiao.TagAbsent || ref.IntOr(-1) != -1 {
		t.Fatalf("a closed value reads as absent")
	}
	if err := m.Set("b", 2); !errors.Is(err, guatiao.ErrClosed) {
		t.Fatalf("Set after close: %v", err)
	}
	if _, err := ref.Any(); !errors.Is(err, guatiao.ErrClosed) {
		t.Fatalf("Any after close: %v", err)
	}
	if _, err := v.Clone(); !errors.Is(err, guatiao.ErrClosed) {
		t.Fatalf("Clone after close: %v", err)
	}
}

func TestTheMapProtocol(t *testing.T) {
	v, err := guatiao.NewMap()
	if err != nil {
		t.Fatal(err)
	}
	defer v.Close()
	m, err := v.Map()
	if err != nil {
		t.Fatal(err)
	}
	for _, kv := range []struct {
		k string
		v any
	}{{"host", "10.0.0.1"}, {"port", 5900}, {"tags", []string{"a", "b"}}} {
		if err := m.Set(kv.k, kv.v); err != nil {
			t.Fatal(err)
		}
	}
	if err := m.Set("port", 5901); err != nil {
		t.Fatal(err)
	}
	if got := m.Keys(); !reflect.DeepEqual(got, []string{"host", "port", "tags"}) {
		t.Fatalf("keys %v", got)
	}
	if m.Len() != 3 || !m.Has("port") || m.Has("nope") {
		t.Fatalf("len %d", m.Len())
	}
	port, ok := m.Get("port")
	if !ok || port.IntOr(0) != 5901 {
		t.Fatalf("port")
	}
	tags, err := v.Key("tags").List()
	if err != nil {
		t.Fatal(err)
	}
	if err := tags.Push("c"); err != nil {
		t.Fatal(err)
	}
	var seen []string
	for k, r := range m.All() {
		seen = append(seen, k+"="+r.Tag().String())
	}
	if strings.Join(seen, ",") != "host=string,port=number,tags=list" {
		t.Fatalf("All %v", seen)
	}
	if gone, err := m.Delete("host"); err != nil || !gone {
		t.Fatalf("Delete %v %v", gone, err)
	}
	if gone, err := m.Delete("host"); err != nil || gone {
		t.Fatalf("Delete twice %v %v", gone, err)
	}
	if err := m.Clear(); err != nil || m.Len() != 0 {
		t.Fatalf("Clear %v", err)
	}
}

func TestCopiesOneMapIntoAnother(t *testing.T) {
	a := mustValue(t, map[string]any{"x": 1, "y": 2})
	b := mustValue(t, map[string]any{"y": 20, "z": 30})
	m, _ := a.Map()
	if err := m.CopyFrom(b.Ref); err != nil {
		t.Fatal(err)
	}
	if got := m.Keys(); !reflect.DeepEqual(got, []string{"x", "y", "z"}) || m.Key("y").IntOr(0) != 20 {
		t.Fatalf("keys %v", got)
	}
}

func TestTheListProtocol(t *testing.T) {
	v := mustValue(t, []any{"a"})
	l, err := v.List()
	if err != nil {
		t.Fatal(err)
	}
	if err := l.Push("b", 3, map[string]any{"k": 1}); err != nil {
		t.Fatal(err)
	}
	if l.Len() != 4 {
		t.Fatalf("len %d", l.Len())
	}
	last, ok := l.At(-1)
	if !ok || last.Tag() != guatiao.TagMap {
		t.Fatalf("At(-1)")
	}
	if _, ok := l.At(9); ok {
		t.Fatalf("At past the end")
	}
	if err := l.Set(0, "A"); err != nil {
		t.Fatal(err)
	}
	if err := l.Insert(1, "mid"); err != nil {
		t.Fatal(err)
	}
	if err := l.RemoveAt(-1); err != nil {
		t.Fatal(err)
	}
	got := plain(t, v)
	want := []any{"A", "mid", "b", guatiao.Number("3")}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v", got)
	}
	if err := l.RemoveAt(10); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("RemoveAt past the end: %v", err)
	}
	n := 0
	for i, r := range l.All() {
		if r.Tag() == guatiao.TagAbsent || i != n {
			t.Fatalf("All")
		}
		n++
	}
	if err := l.Clear(); err != nil || l.Len() != 0 {
		t.Fatalf("Clear")
	}
}

func TestClonesIntoAnIndependentValue(t *testing.T) {
	v := mustValue(t, map[string]any{"a": []any{1, 2}})
	c, err := v.Key("a").Clone()
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	l, _ := v.Key("a").List()
	l.Push(3)
	cl, _ := c.List()
	if cl.Len() != 2 {
		t.Fatalf("the clone changed with its source")
	}
	nested := mustValue(t, map[string]any{"copy": v, "part": v.Key("a")})
	if nested.Key("copy").Key("a").Index(2).IntOr(0) != 3 || nested.Key("part").Index(0).IntOr(0) != 1 {
		t.Fatalf("a value inside another is copied in")
	}
}

func TestRefusesTheWrongContainer(t *testing.T) {
	v := mustValue(t, map[string]any{"s": "x", "l": []int{1}})
	if _, err := v.Key("s").Map(); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("a string as a map: %v", err)
	}
	if _, err := v.Key("l").Map(); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("a list as a map: %v", err)
	}
	if _, err := v.Key("nope").List(); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("nothing as a list: %v", err)
	}
}

func TestAbsentAndNull(t *testing.T) {
	a := guatiao.Absent()
	defer a.Close()
	if !a.IsAbsent() {
		t.Fatal("Absent")
	}
	n, err := guatiao.Null()
	if err != nil {
		t.Fatal(err)
	}
	defer n.Close()
	if !n.IsNull() {
		t.Fatal("Null")
	}
	if got := plain(t, n); got != nil {
		t.Fatalf("null reads as nil, not %#v", got)
	}
}

func TestPathsAreSpelledInTheIntakeGrammar(t *testing.T) {
	r := (guatiao.Ref{}).Key("agent").Index(1).Key("name").Key("a.b")
	if got := r.Path(); got != `agent[1].name["a.b"]` {
		t.Fatalf("path %q", got)
	}
}

func TestReadingStopsAtTheDepthTheOtherWalksStopAt(t *testing.T) {
	v := mustValue(t, []any{})
	at := v.Ref
	for i := 0; i < 140; i++ {
		l, err := at.List()
		if err != nil {
			t.Fatal(err)
		}
		if err := l.Push([]any{}); err != nil {
			t.Fatal(err)
		}
		at = at.Index(0)
	}
	if _, err := v.Any(); !errors.Is(err, guatiao.ErrBadValue) || !strings.Contains(err.Error(), "deeper") {
		t.Fatalf("%v", err)
	}
}
