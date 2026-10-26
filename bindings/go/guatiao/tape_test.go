// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// MarshalWire and UnmarshalWire against the tree they stand in for: the
// same bytes as NewValue then ToWire, the same Go data as FromWire then
// Decode, and the same refusals.

import (
	"bytes"
	"errors"
	"math"
	"math/big"
	"net/netip"
	"reflect"
	"strings"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// treeWire is the wire encoding of x as a tree writes it.
func treeWire(t *testing.T, x any) []byte {
	t.Helper()
	v := mustValue(t, x)
	data, err := v.ToWire()
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestWireBytesReadAndWrittenInGo(t *testing.T) {
	vectors := readWireVectors(t)
	for _, c := range vectors.Values {
		data := unhex(t, c.Hex)
		v, err := guatiao.FromWire(data)
		if err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		want, err := v.Any()
		v.Close()
		if err != nil {
			t.Fatal(err)
		}
		var got any
		if err := guatiao.UnmarshalWire(data, &got); err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("%s: read %#v, the tree reads %#v", c.Name, got, want)
		}
		written, err := guatiao.MarshalWire(got)
		if err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		if tree := treeWire(t, got); !bytes.Equal(written, tree) {
			t.Fatalf("%s: wrote %x, the tree writes %x", c.Name, written, tree)
		}
	}
}

func TestTheTapeRefusesMalformedBytes(t *testing.T) {
	vectors := readWireVectors(t)
	for _, c := range vectors.Malformed {
		var x any
		err := guatiao.UnmarshalWire(unhex(t, c.Hex), &x)
		if c.Name == "a key repeated" {
			// Not looked for: the last one is kept.
			if err != nil {
				t.Errorf("%s: %v", c.Name, err)
			}
			continue
		}
		if !errors.Is(err, guatiao.ErrBadValue) {
			t.Errorf("%s: %v", c.Name, err)
		}
	}
	data := treeWire(t, map[string]any{"a": []any{1, "two", []byte{0}}, "ключ": true})
	for end := 0; end < len(data); end++ {
		var x any
		if err := guatiao.UnmarshalWire(data[:end], &x); !errors.Is(err, guatiao.ErrBadValue) {
			t.Fatalf("cut at %d: %v", end, err)
		}
	}
	var x any
	if err := guatiao.UnmarshalWire(append(data, 1), &x); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("a byte after the value: %v", err)
	}
	// A count no remaining bytes could hold is refused before anything that large is made.
	if err := guatiao.UnmarshalWire([]byte{6, 0xff, 0xff, 0xff, 0xff, 0x0f}, &x); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("a huge count: %v", err)
	}
	if err := guatiao.UnmarshalWire(data, x); !errors.Is(err, guatiao.ErrNullArgument) {
		t.Fatalf("into a non-pointer: %v", err)
	}
}

func TestATypedValueCrossesTheTapeAsTheTree(t *testing.T) {
	at := time.Date(2026, 10, 21, 13, 45, 0, 500, time.UTC)
	c := Config{
		Base: Base{Kind: "vnc"}, Host: "a", Port: 5900, Env: map[string]string{"B": "2", "A": "1"},
		Timeout: 90*time.Second + 5, At: at, Addr: netip.MustParseAddr("::1"),
		Ratio: 0.5, Big: new(big.Int).Lsh(big.NewInt(1), 70), ByPort: map[uint16]bool{443: true, 80: false},
		TLS: &Inner{Ca: "x"}, Tags: []string{"t"},
	}
	data, err := guatiao.MarshalWire(c)
	if err != nil {
		t.Fatal(err)
	}
	if tree := treeWire(t, c); !bytes.Equal(data, tree) {
		t.Fatalf("wrote %x\nthe tree writes %x", data, tree)
	}
	var back Config
	if err := guatiao.UnmarshalWire(data, &back); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(back, c) {
		t.Fatalf("round trip\n got %#v\nwant %#v", back, c)
	}

	temps := []celsius{21.5, -3}
	if data, err = guatiao.MarshalWire(temps); err != nil || !bytes.Equal(data, treeWire(t, temps)) {
		t.Fatalf("a Marshaler: %x %v", data, err)
	}
	var temps2 []celsius
	if err := guatiao.UnmarshalWire(data, &temps2); err != nil || !reflect.DeepEqual(temps2, temps) {
		t.Fatalf("an Unmarshaler: %v %v", temps2, err)
	}

	lengths := map[string]any{"list": []int{90, 5}, "secs": guatiao.Number("1.5"), "serde": map[string]any{"secs": 2, "nanos": 3}}
	var ds map[string]time.Duration
	if err := guatiao.UnmarshalWire(treeWire(t, lengths), &ds); err != nil {
		t.Fatal(err)
	}
	if want := map[string]time.Duration{"list": 90*time.Second + 5, "secs": 1500 * time.Millisecond, "serde": 2*time.Second + 3}; !reflect.DeepEqual(ds, want) {
		t.Fatalf("an exact length in every spelling: %v", ds)
	}
	var instant time.Time
	if err := guatiao.UnmarshalWire(treeWire(t, "2026-10-21T19:15:00.5+05:30"), &instant); err != nil ||
		!instant.Equal(time.Date(2026, 10, 21, 13, 45, 0, 500_000_000, time.UTC)) || instant.Location() != time.UTC {
		t.Fatalf("an instant: %v %v", instant, err)
	}

	var kept struct {
		V    *guatiao.Value `guatiao:"v"`
		N    guatiao.Number `guatiao:"n"`
		Any  any            `guatiao:"any"`
		Rest map[string]int `guatiao:"rest"`
		Ptr  **int          `guatiao:"ptr"`
		Arr  [2]bool        `guatiao:"arr"`
		Null *int           `guatiao:"null"`
		Skip []int          `guatiao:"skip"`
	}
	src := map[string]any{"v": map[string]any{"deep": []any{1}}, "n": guatiao.Number("1.10"), "any": []any{nil, "s"},
		"rest": map[string]int{"a": 1}, "ptr": 7, "arr": []bool{true, false}, "null": nil, "skip": nil, "unknown": map[string]any{"x": []any{1, 2}}}
	if err := guatiao.UnmarshalWire(treeWire(t, src), &kept); err != nil {
		t.Fatal(err)
	}
	defer kept.V.Close()
	if got, _ := kept.V.Any(); !reflect.DeepEqual(got, map[string]any{"deep": []any{guatiao.Number("1")}}) {
		t.Fatalf("a *Value: %#v", got)
	}
	if kept.N != "1.10" || !reflect.DeepEqual(kept.Any, []any{nil, "s"}) || kept.Rest["a"] != 1 || **kept.Ptr != 7 ||
		kept.Arr != [2]bool{true, false} || kept.Null != nil || kept.Skip != nil {
		t.Fatalf("%#v", kept)
	}
}

func TestTheTapeDecodeRefusesAsTheTreeDoes(t *testing.T) {
	v := mustValue(t, map[string]any{"port": 70000, "host": 5, "tags": "x", "f": guatiao.Number("1e400"),
		"neg": -1, "frac": 1.5, "arr": []int{1}, "when": "yesterday", "key": map[string]int{"x": 1}})
	cases := []struct {
		key string
		dst func() any
	}{
		{"port", func() any { return new(uint16) }},
		{"host", func() any { return new(string) }},
		{"tags", func() any { return new([]string) }},
		{"f", func() any { return new(float64) }},
		{"neg", func() any { return new(uint) }},
		{"frac", func() any { return new(int) }},
		{"arr", func() any { return new([2]int) }},
		{"when", func() any { return new(time.Time) }},
		{"key", func() any { return new(map[int]int) }},
		{"host", func() any { return new(chan int) }},
		{"host", func() any { return new(guatiao.Ref) }},
	}
	for _, c := range cases {
		treeErr := v.Key(c.key).Decode(c.dst())
		data, err := v.Key(c.key).ToWire()
		if err != nil {
			t.Fatal(err)
		}
		tapeErr := guatiao.UnmarshalWire(data, c.dst())
		if treeErr == nil || tapeErr == nil || tapeErr.Error() != strings.Replace(treeErr.Error(), "at "+c.key+": ", "", 1) {
			t.Errorf("%s into %T: the tape says %v, the tree %v", c.key, c.dst(), tapeErr, treeErr)
		}
	}
}

func TestTheTapeRefusesWhatNewValueRefuses(t *testing.T) {
	type self struct{ Next *self }
	loop := &self{}
	loop.Next = loop
	deep := []any{}
	for i := 0; i < 200; i++ {
		deep = []any{deep}
	}
	for _, bad := range []any{
		make(chan int), func() {}, complex(1, 2), math.NaN(), math.Inf(1),
		map[[2]int]int{{1, 2}: 3}, "\xff\xfe", map[string]any{"\xff": 1}, guatiao.Number("1."),
		map[string]any{"n": guatiao.Number("0x10")}, loop, deep, -time.Second,
	} {
		v, treeErr := guatiao.NewValue(bad)
		if treeErr == nil {
			v.Close()
			t.Fatalf("NewValue(%T) took it", bad)
		}
		_, tapeErr := guatiao.MarshalWire(bad)
		if tapeErr == nil || tapeErr.Error() != treeErr.Error() {
			t.Errorf("%T: the tape says %v, the tree %v", bad, tapeErr, treeErr)
		}
	}
}

func TestTheTapeWritesTextOfEveryLength(t *testing.T) {
	for _, s := range []string{"", "a", "é", strings.Repeat("€", 43), strings.Repeat("x", 200), strings.Repeat("ü", 70_000)} {
		x := map[string]any{s: s}
		data, err := guatiao.MarshalWire(x)
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(data, treeWire(t, x)) {
			t.Fatalf("%d bytes of text", len(s))
		}
		var back map[string]string
		if err := guatiao.UnmarshalWire(data, &back); err != nil || back[s] != s {
			t.Fatalf("%d bytes of text: %v", len(s), err)
		}
	}
}

func TestTheTapeSaysWhereAsTheTreeDoes(t *testing.T) {
	type port struct {
		Port uint8 `guatiao:"port"`
	}
	var a, b struct {
		Servers []port `guatiao:"servers"`
	}
	v := mustValue(t, map[string]any{"servers": []any{map[string]any{"port": 1}, map[string]any{"port": 300}}})
	treeErr := v.Decode(&a)
	tapeErr := guatiao.UnmarshalWire(treeWire(t, v), &b)
	if treeErr == nil || tapeErr == nil || tapeErr.Error() != treeErr.Error() || !strings.Contains(tapeErr.Error(), "servers[1].port") {
		t.Fatalf("decoding: the tape says %v, the tree %v", tapeErr, treeErr)
	}
	bad := map[string]any{"a": []any{1, map[string]any{"b": math.NaN()}}}
	_, treeErr = guatiao.NewValue(bad)
	_, tapeErr = guatiao.MarshalWire(bad)
	if treeErr == nil || tapeErr == nil || tapeErr.Error() != treeErr.Error() || !strings.Contains(tapeErr.Error(), "a[1].b") {
		t.Fatalf("encoding: the tape says %v, the tree %v", tapeErr, treeErr)
	}
}

func TestAValueNestsAsDeepAsTheLibraryAllows(t *testing.T) {
	// An interface or a pointer on the way is not a level of its own.
	var deep any = "leaf"
	for i := 0; i < 120; i++ {
		deep = []any{deep}
	}
	v, err := guatiao.NewValue(deep)
	if err != nil {
		t.Fatalf("120 lists deep: %v", err)
	}
	v.Close()
	if _, err := guatiao.MarshalWire(deep); err != nil {
		t.Fatalf("120 lists deep through the tape: %v", err)
	}
}

// sameText is a map key every value of which writes one text.
type sameText int

func (sameText) MarshalText() ([]byte, error) { return []byte("k"), nil }

func TestKeysWritingOneTextAreRefused(t *testing.T) {
	m := map[sameText]int{1: 1, 2: 2}
	if v, err := guatiao.NewValue(m); err == nil || !errors.Is(err, guatiao.ErrBadValue) || !strings.Contains(err.Error(), `"k"`) {
		if v != nil {
			v.Close()
		}
		t.Fatalf("NewValue: %v", err)
	}
	if _, err := guatiao.MarshalWire(m); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("MarshalWire: %v", err)
	}
}
