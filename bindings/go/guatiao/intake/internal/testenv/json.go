// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package testenv

import (
	"encoding/json"
	"fmt"
	"strings"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// FromJSON builds a Value from JSON text, each object's keys in the order
// written, which a Go map literal cannot keep.
func FromJSON(text string) (*guatiao.Value, error) {
	dec := json.NewDecoder(strings.NewReader(text))
	dec.UseNumber()
	return build(dec)
}

func build(dec *json.Decoder) (*guatiao.Value, error) {
	tok, err := dec.Token()
	if err != nil {
		return nil, err
	}
	switch t := tok.(type) {
	case json.Delim:
		switch t {
		case '{':
			v, err := guatiao.NewMap()
			if err != nil {
				return nil, err
			}
			m, _ := v.Map()
			for dec.More() {
				key, err := dec.Token()
				if err != nil {
					v.Close()
					return nil, err
				}
				if err := add(dec, v, func(child *guatiao.Value) error { return m.Set(key.(string), child) }); err != nil {
					return nil, err
				}
			}
			return closing(dec, v)
		case '[':
			v, err := guatiao.NewList()
			if err != nil {
				return nil, err
			}
			l, _ := v.List()
			for dec.More() {
				if err := add(dec, v, func(child *guatiao.Value) error { return l.Push(child) }); err != nil {
					return nil, err
				}
			}
			return closing(dec, v)
		}
		return nil, fmt.Errorf("unexpected %v", t)
	case json.Number:
		return guatiao.NewValue(guatiao.Number(t))
	default:
		return guatiao.NewValue(t)
	}
}

// closing reads the delimiter that ends v.
func closing(dec *json.Decoder, v *guatiao.Value) (*guatiao.Value, error) {
	if _, err := dec.Token(); err != nil {
		v.Close()
		return nil, err
	}
	return v, nil
}

// add builds the next value and copies it into parent through put,
// closing parent when either fails.
func add(dec *json.Decoder, parent *guatiao.Value, put func(*guatiao.Value) error) error {
	child, err := build(dec)
	if err == nil {
		err = put(child)
		child.Close()
	}
	if err != nil {
		parent.Close()
	}
	return err
}
