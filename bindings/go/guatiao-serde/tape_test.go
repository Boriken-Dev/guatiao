// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package serde_test

// Unmarshal and Emit of Go data cross as wire bytes: they must answer as
// Parse then Decode, and as Emit of the tree, do.

import (
	"errors"
	"reflect"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
)

type server struct {
	Host   string            `guatiao:"host"`
	Port   uint16            `guatiao:"port"`
	Weight float64           `guatiao:"weight"`
	Tags   []string          `guatiao:"tags"`
	Limits map[string]any    `guatiao:"limits"`
	Note   *string           `guatiao:"note"`
	Extra  map[string]string `guatiao:"extra,omitempty"`
}

func TestUnmarshalReadsAsParseThenDecode(t *testing.T) {
	data := map[string]any{
		"servers": []server{{Host: "a", Port: 1, Weight: 0.5, Tags: []string{"x"}, Limits: map[string]any{"rps": 10, "burst": 2.5}}},
		"n":       guatiao.Number("1.10"),
		"big":     guatiao.Number("1180591620717411303424"),
		"ü":       "€",
	}
	for _, f := range []serde.Format{serde.JSON, serde.TOML, serde.YAML} {
		text, err := serde.Emit(data, f)
		if err != nil {
			t.Fatalf("%s: %v", f, err)
		}
		var viaTape, viaTree any
		if err := serde.Unmarshal(text, f, &viaTape); err != nil {
			t.Fatalf("%s: %v", f, err)
		}
		if err := parse(t, text, f).Decode(&viaTree); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(viaTape, viaTree) {
			t.Fatalf("%s: the tape reads %#v\nthe tree reads %#v", f, viaTape, viaTree)
		}
		var typedTape, typedTree struct {
			Servers []server `guatiao:"servers"`
		}
		if err := serde.Unmarshal(text, f, &typedTape); err != nil {
			t.Fatalf("%s: %v", f, err)
		}
		if err := parse(t, text, f).Decode(&typedTree); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(typedTape, typedTree) || typedTape.Servers[0].Host != "a" {
			t.Fatalf("%s: %#v", f, typedTape)
		}
	}
}

func TestUnmarshalRefusesAsParseAndDecodeDo(t *testing.T) {
	for _, c := range []struct {
		text   string
		format serde.Format
	}{{"{\"a\":\n 1,,}", serde.JSON}, {"a = = 1", serde.TOML}, {"a: [1", serde.YAML}} {
		_, parseErr := serde.Parse(c.text, c.format)
		var x any
		err := serde.Unmarshal(c.text, c.format, &x)
		var a, b *serde.ParseError
		if !errors.As(parseErr, &a) || !errors.As(err, &b) || *a != *b {
			t.Errorf("%s: Unmarshal says %v, Parse %v", c.format, err, parseErr)
		}
	}
	var port uint8
	if err := serde.Unmarshal(`300`, serde.JSON, &port); !errors.Is(err, guatiao.ErrBadValue) {
		t.Errorf("a number that does not fit: %v", err)
	}
	if err := serde.Unmarshal(`{}`, "xml", &port); err == nil {
		t.Error("an unknown format")
	}
}

func TestEmitOfGoDataWritesWhatTheTreeWrites(t *testing.T) {
	note := "n"
	data := map[string]any{
		"servers": []server{{Host: "a", Port: 1, Weight: 0.5, Tags: []string{"x"}, Note: &note, Extra: map[string]string{"k": "v"}}},
		"bytes":   []byte{1, 2, 3},
		"n":       guatiao.Number("1.10"),
		"ü":       "€",
	}
	tree, err := guatiao.NewValue(data)
	if err != nil {
		t.Fatal(err)
	}
	defer tree.Close()
	for _, f := range []serde.Format{serde.JSON, serde.TOML, serde.YAML} {
		for _, opts := range [][]serde.Option{nil, {serde.Pretty()}, {serde.How(serde.BytesBase64)}} {
			got, err := serde.Emit(data, f, opts...)
			if err != nil {
				t.Fatalf("%s: %v", f, err)
			}
			want, err := serde.Emit(tree, f, opts...)
			if err != nil {
				t.Fatal(err)
			}
			if got != want {
				t.Fatalf("%s %d: the tape writes\n%s\nthe tree writes\n%s", f, len(opts), got, want)
			}
		}
	}
	if _, err := serde.Emit(map[string]any{"b": []byte{1}}, serde.JSON, serde.How(serde.BytesRefuse)); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("bytes refused: %v", err)
	}
	if _, err := serde.Emit(map[string]any{"bad": "\xff"}, serde.JSON); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("text that is not UTF-8: %v", err)
	}
}
