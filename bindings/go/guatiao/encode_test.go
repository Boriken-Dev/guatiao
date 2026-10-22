// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

import (
	"errors"
	"math/big"
	"net/netip"
	"reflect"
	"strings"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

type Inner struct {
	Ca string `guatiao:"ca"`
}

type Base struct {
	Kind string `guatiao:"kind"`
}

type Config struct {
	Base
	Host    string            `guatiao:"host"`
	Port    uint16            `guatiao:"port"`
	Tags    []string          `guatiao:"tags,omitempty"`
	TLS     *Inner            `guatiao:"tls"`
	Env     map[string]string `json:"env"`
	Timeout time.Duration     `guatiao:"timeout"`
	At      time.Time         `guatiao:"at"`
	Addr    netip.Addr        `guatiao:"addr"`
	Skip    string            `guatiao:"-"`
	hidden  int
	Ratio   float64
	Big     *big.Int        `guatiao:"big"`
	ByPort  map[uint16]bool `guatiao:"by_port"`
}

func TestAStructIsAMapByItsTags(t *testing.T) {
	at := time.Date(2026, 10, 21, 13, 45, 0, 500, time.UTC)
	c := Config{
		Base: Base{Kind: "vnc"}, Host: "a", Port: 5900, Env: map[string]string{"B": "2", "A": "1"},
		Timeout: 90*time.Second + 5, At: at, Addr: netip.MustParseAddr("::1"), Skip: "x", hidden: 1,
		Ratio: 0.5, Big: new(big.Int).Lsh(big.NewInt(1), 70), ByPort: map[uint16]bool{443: true, 80: false},
	}
	v := mustValue(t, c)
	m, _ := v.Map()
	want := []string{"kind", "host", "port", "env", "timeout", "at", "addr", "Ratio", "big", "by_port"}
	if got := m.Keys(); !reflect.DeepEqual(got, want) {
		t.Fatalf("keys %v", got)
	}
	if got, _ := v.Key("env").Map(); !reflect.DeepEqual(got.Keys(), []string{"A", "B"}) {
		t.Fatalf("a Go map is written with its keys sorted: %v", got.Keys())
	}
	if got, _ := v.Key("by_port").Map(); !reflect.DeepEqual(got.Keys(), []string{"80", "443"}) {
		t.Fatalf("integer keys sort by value: %v", got.Keys())
	}
	if got := v.Key("at").StringOr(""); got != "2026-10-21T13:45:00.0000005Z" {
		t.Fatalf("an instant is its text: %q", got)
	}
	if got := plain(t, v.Key("timeout")); !reflect.DeepEqual(got, []any{guatiao.Number("90"), guatiao.Number("5")}) {
		t.Fatalf("a duration is [seconds, nanoseconds]: %#v", got)
	}
	if got := v.Key("addr").StringOr(""); got != "::1" {
		t.Fatalf("a TextMarshaler is its text: %q", got)
	}
	if got, _ := v.Key("big").NumberText(); got != "1180591620717411303424" {
		t.Fatalf("a big.Int is a number: %q", got)
	}

	var back Config
	if err := v.Decode(&back); err != nil {
		t.Fatal(err)
	}
	c.Skip, c.hidden = "", 0
	if !reflect.DeepEqual(back, c) {
		t.Fatalf("round trip\n got %#v\nwant %#v", back, c)
	}
}

func TestANilPointerFieldIsOmitted(t *testing.T) {
	v := mustValue(t, Config{Host: "a"})
	if v.Key("tls").Tag() != guatiao.TagAbsent || v.Key("big").Tag() != guatiao.TagAbsent {
		t.Fatalf("a nil pointer field is absent")
	}
	if v.Key("tags").Tag() != guatiao.TagAbsent {
		t.Fatalf("omitempty leaves an empty slice out")
	}
}

func TestDecodeRefusesWhatDoesNotFit(t *testing.T) {
	v := mustValue(t, map[string]any{"port": 70000, "host": 5, "tags": "x", "f": guatiao.Number("1e400"),
		"neg": -1, "frac": 1.5})
	cases := []struct {
		key string
		dst any
		in  string
	}{
		{"port", new(uint16), "port"},
		{"host", new(string), "want a string, got number"},
		{"tags", new([]string), "want a list"},
		{"f", new(float64), "does not fit"},
		{"neg", new(uint), "is not a uint"},
		{"frac", new(int), "is not a int"},
	}
	for _, c := range cases {
		err := v.Key(c.key).Decode(c.dst)
		if !errors.Is(err, guatiao.ErrBadValue) || !strings.Contains(err.Error(), c.in) {
			t.Errorf("%s into %T: %v", c.key, c.dst, err)
		}
	}
	if err := v.Decode(Config{}); !errors.Is(err, guatiao.ErrNullArgument) {
		t.Errorf("Decode into a non-pointer: %v", err)
	}
}

func TestDecodeIntoAnyIsPlainData(t *testing.T) {
	v := mustValue(t, map[string]any{"n": guatiao.Number("18446744073709551616"), "b": []byte("x")})
	var x any
	if err := v.Decode(&x); err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"n": guatiao.Number("18446744073709551616"), "b": []byte("x")}
	if !reflect.DeepEqual(x, want) {
		t.Fatalf("%#v", x)
	}
	var into *guatiao.Value
	if err := v.Key("n").Decode(&into); err != nil {
		t.Fatal(err)
	}
	defer into.Close()
	if got, _ := into.NumberText(); got != "18446744073709551616" {
		t.Fatalf("a *Value takes a copy: %q", got)
	}
}

type celsius float64

func (c celsius) MarshalGuatiao() (any, error) { return map[string]any{"c": float64(c)}, nil }

func (c *celsius) UnmarshalGuatiao(r guatiao.Ref) error {
	*c = celsius(r.Key("c").FloatOr(0))
	return nil
}

func TestATypeSaysWhatItIsAsAValue(t *testing.T) {
	v := mustValue(t, []celsius{21.5})
	if got := v.Index(0).Key("c").FloatOr(0); got != 21.5 {
		t.Fatalf("MarshalGuatiao: %v", got)
	}
	var back []celsius
	if err := v.Decode(&back); err != nil || len(back) != 1 || back[0] != 21.5 {
		t.Fatalf("UnmarshalGuatiao: %v %v", back, err)
	}
}

func TestAnExactLengthIsReadInEverySpelling(t *testing.T) {
	v := mustValue(t, map[string]any{"list": []int{90, 5}, "secs": guatiao.Number("1.5"),
		"serde": map[string]any{"secs": 2, "nanos": 3}, "neg": -1, "long": guatiao.Number("9999999999999")})
	for key, want := range map[string]time.Duration{"list": 90*time.Second + 5, "secs": 1500 * time.Millisecond,
		"serde": 2*time.Second + 3} {
		var d time.Duration
		if err := v.Key(key).Decode(&d); err != nil || d != want {
			t.Errorf("%s: %v %v", key, d, err)
		}
	}
	for _, key := range []string{"neg", "long"} {
		var d time.Duration
		if err := v.Key(key).Decode(&d); !errors.Is(err, guatiao.ErrBadValue) {
			t.Errorf("%s: %v", key, err)
		}
	}
	if _, err := guatiao.NewValue(-time.Second); !errors.Is(err, guatiao.ErrBadValue) {
		t.Errorf("a negative duration: %v", err)
	}
}

func TestAnInstantIsReadWithItsOffset(t *testing.T) {
	v := mustValue(t, "2026-10-21T19:15:00.5+05:30")
	var at time.Time
	if err := v.Decode(&at); err != nil {
		t.Fatal(err)
	}
	if want := time.Date(2026, 10, 21, 13, 45, 0, 500_000_000, time.UTC); !at.Equal(want) || at.Location() != time.UTC {
		t.Fatalf("%v", at)
	}
}
