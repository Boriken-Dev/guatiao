// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guest

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// The shared vectors every implementation of the wire encoding reads.
type vectors struct {
	Values []struct {
		Name string `json:"name"`
		Tree any    `json:"tree"`
		Hex  string `json:"hex"`
	} `json:"values"`
	Malformed []struct {
		Name string `json:"name"`
		Hex  string `json:"hex"`
		At   int    `json:"at"`
	} `json:"malformed"`
}

func readVectors(t *testing.T) vectors {
	t.Helper()
	path := filepath.Join("..", "..", "guatiao", "tests", "vectors", "wire.json")
	text, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("the shared vectors: %v", err)
	}
	var v vectors
	if err := json.Unmarshal(text, &v); err != nil {
		t.Fatal(err)
	}
	return v
}

// tree is a value in the vectors' notation.
func tree(v any) any {
	switch x := v.(type) {
	case AbsentValue:
		return []any{"absent"}
	case nil:
		return []any{"null"}
	case bool:
		return []any{"bool", x}
	case Number:
		return []any{"number", string(x)}
	case string:
		return []any{"string", x}
	case []byte:
		return []any{"bytes", hex.EncodeToString(x)}
	case []any:
		items := []any{}
		for _, item := range x {
			items = append(items, tree(item))
		}
		return []any{"list", items}
	case *Map:
		pairs := []any{}
		for _, k := range x.Keys() {
			v, _ := x.Get(k)
			pairs = append(pairs, []any{k, tree(v)})
		}
		return []any{"map", pairs}
	}
	return []any{"unknown"}
}

func TestEveryVectorDecodesToItsTreeAndEncodesToItsBytes(t *testing.T) {
	v := readVectors(t)
	if len(v.Values) < 20 {
		t.Fatalf("only %d values", len(v.Values))
	}
	for _, c := range v.Values {
		raw, _ := hex.DecodeString(c.Hex)
		value, err := Decode(raw)
		if err != nil {
			t.Errorf("%s: %v", c.Name, err)
			continue
		}
		if got := tree(value); !reflect.DeepEqual(got, c.Tree) {
			t.Errorf("%s: %v, not %v", c.Name, got, c.Tree)
		}
		back, err := Encode(value)
		if err != nil || !bytes.Equal(back, raw) {
			t.Errorf("%s: encoded %x (%v), not %s", c.Name, back, err, c.Hex)
		}
	}
}

func TestEveryMalformationIsRefusedAtItsByte(t *testing.T) {
	for _, c := range readVectors(t).Malformed {
		raw, _ := hex.DecodeString(c.Hex)
		_, err := Decode(raw)
		var we *WireError
		if !errors.As(err, &we) {
			t.Errorf("%s: %v", c.Name, err)
			continue
		}
		if we.At != c.At {
			t.Errorf("%s: refused at %d (%s), not %d", c.Name, we.At, we.Why, c.At)
		}
	}
}

func TestGoValuesEncodeAsTheirNumbersAndMapsInOrder(t *testing.T) {
	m := NewMap().Set("z", 1).Set("a", uint64(18446744073709551615)).Set("f", 1.5)
	out, err := Encode(m)
	if err != nil {
		t.Fatal(err)
	}
	back, err := Decode(out)
	if err != nil {
		t.Fatal(err)
	}
	want := []any{"map", []any{
		[]any{"z", []any{"number", "1"}},
		[]any{"a", []any{"number", "18446744073709551615"}},
		[]any{"f", []any{"number", "1.5"}},
	}}
	if got := tree(back); !reflect.DeepEqual(got, want) {
		t.Errorf("%v", got)
	}
	if _, err := Encode(map[string]any{"x": []any{Number("01")}}); err == nil {
		t.Error("a number outside the grammar was encoded")
	}
}
