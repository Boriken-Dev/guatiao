// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"encoding/json"
	"fmt"
	"math"
	"reflect"
	"strconv"
	"strings"
)

// SchemaOf is the JSON Schema (2020-12) of the values a T encodes to and
// decodes from, as a Value to close: what a configuration struct's
// provider declares, and what a host checks a configuration against. A
// struct's properties keep its fields' order, which is the order a form
// shows them in.
//
// A struct is an object whose properties are its fields, named as
// NewValue names them, refusing keys it does not declare; a field is
// required unless it is a pointer (which may also be null), is
// `omitempty`, or has a default. A `schema` tag adds keywords to the
// field's schema, comma-separated `key=value` pairs whose value is read as
// JSON when it is JSON and as text otherwise:
//
//	Prefix string `guatiao:"prefix" schema:"minLength=1,default=hey"`
//
// The integer types state their bounds; time.Time, time.Duration and the
// civil types are their encodings' forms; a type with a GuatiaoSchema
// method states its own.
func SchemaOf[T any]() (*Value, error) {
	return SchemaFor(reflect.TypeFor[T]())
}

// SchemaFor is SchemaOf for a reflect.Type.
func SchemaFor(t reflect.Type) (*Value, error) {
	s, err := schemaFor(t, map[reflect.Type]bool{})
	if err != nil {
		return nil, err
	}
	return NewValue(s)
}

func intBounds(t reflect.Type) (lo, hi any) {
	bits := t.Bits()
	switch t.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		if bits == 64 {
			return int64(math.MinInt64), int64(math.MaxInt64)
		}
		return -(int64(1) << (bits - 1)), int64(1)<<(bits-1) - 1
	}
	if bits == 64 {
		return 0, Number(strconv.FormatUint(math.MaxUint64, 10))
	}
	return 0, uint64(1)<<bits - 1
}

func schemaFor(t reflect.Type, visiting map[reflect.Type]bool) (map[string]any, error) {
	if s, ok := ownSchema(t); ok {
		return s, nil
	}
	switch t {
	case typeTime:
		return map[string]any{"type": "string", "format": "date-time"}, nil
	case typeDuration:
		return map[string]any{
			"type": "array",
			"prefixItems": []any{
				map[string]any{"type": "integer", "minimum": 0, "maximum": Number("18446744073709551615")},
				map[string]any{"type": "integer", "minimum": 0, "maximum": 999999999},
			},
			"items": false, "minItems": 2, "maxItems": 2,
		}, nil
	case typeNumber:
		return map[string]any{"type": "number"}, nil
	case typeBigInt:
		return map[string]any{"type": "integer"}, nil
	case typeValue, typeRef, typeMapRef, typeListRef, typeAny:
		return map[string]any{}, nil
	}
	if t.Kind() != reflect.Pointer && t.Kind() != reflect.Interface && t.Implements(typeTextMarshaler) {
		return map[string]any{"type": "string"}, nil
	}
	switch t.Kind() {
	case reflect.Bool:
		return map[string]any{"type": "boolean"}, nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		lo, hi := intBounds(t)
		return map[string]any{"type": "integer", "minimum": lo, "maximum": hi}, nil
	case reflect.Float32, reflect.Float64:
		return map[string]any{"type": "number"}, nil
	case reflect.String:
		return map[string]any{"type": "string"}, nil
	case reflect.Interface:
		return map[string]any{}, nil
	case reflect.Pointer:
		inner, err := schemaFor(t.Elem(), visiting)
		if err != nil {
			return nil, err
		}
		return map[string]any{"anyOf": []any{inner, map[string]any{"type": "null"}}}, nil
	case reflect.Slice, reflect.Array:
		if t.Kind() == reflect.Slice && t.Elem().Kind() == reflect.Uint8 {
			return map[string]any{"type": "bytes"}, nil
		}
		item, err := schemaFor(t.Elem(), visiting)
		if err != nil {
			return nil, err
		}
		s := map[string]any{"type": "array", "items": item}
		if t.Kind() == reflect.Array {
			s["minItems"], s["maxItems"] = t.Len(), t.Len()
		}
		return s, nil
	case reflect.Map:
		value, err := schemaFor(t.Elem(), visiting)
		if err != nil {
			return nil, err
		}
		s := map[string]any{"type": "object", "additionalProperties": value}
		switch t.Key().Kind() {
		case reflect.String:
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			s["propertyNames"] = map[string]any{"type": "string", "pattern": "^-?(0|[1-9][0-9]*)$"}
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
			s["propertyNames"] = map[string]any{"type": "string", "pattern": "^(0|[1-9][0-9]*)$"}
		case reflect.Bool:
			s["propertyNames"] = map[string]any{"type": "string", "enum": []any{"false", "true"}}
		default:
			if !t.Key().Implements(typeTextMarshaler) {
				return nil, fmt.Errorf("guatiao: a map keyed by %s has no schema", t.Key())
			}
		}
		return s, nil
	case reflect.Struct:
		if visiting[t] {
			return nil, fmt.Errorf("guatiao: %s refers to itself, and SchemaOf writes no $ref", t)
		}
		visiting[t] = true
		defer delete(visiting, t)
		properties := map[string]any{}
		var required []any
		var order []any
		for _, f := range fieldsOf(t) {
			fs, err := schemaFor(f.typ, visiting)
			if err != nil {
				return nil, fmt.Errorf("guatiao: %s.%s: %w", t, f.name, err)
			}
			extra, err := keywords(f.schema)
			if err != nil {
				return nil, fmt.Errorf("guatiao: %s.%s: %w", t, f.name, err)
			}
			for k, v := range extra {
				fs[k] = v
			}
			properties[f.name] = fs
			order = append(order, f.name)
			_, defaulted := extra["default"]
			if f.typ.Kind() != reflect.Pointer && !f.omitEmpty && !defaulted {
				required = append(required, f.name)
			}
		}
		s := map[string]any{"type": "object", "properties": orderedMap{keys: order, values: properties}, "additionalProperties": false}
		if len(required) > 0 {
			s["required"] = required
		}
		return s, nil
	}
	return nil, fmt.Errorf("guatiao: a %s has no schema", t)
}

// ownSchema is the schema a type states, through either receiver.
func ownSchema(t reflect.Type) (map[string]any, bool) {
	if t.Implements(typeSchemer) && t.Kind() != reflect.Interface {
		return reflect.Zero(t).Interface().(Schemer).GuatiaoSchema(), true
	}
	if reflect.PointerTo(t).Implements(typeSchemer) {
		return reflect.New(t).Interface().(Schemer).GuatiaoSchema(), true
	}
	return nil, false
}

// keywords reads a `schema` tag.
func keywords(tag string) (map[string]any, error) {
	out := map[string]any{}
	for _, pair := range splitPairs(tag) {
		pair = strings.TrimSpace(pair)
		if pair == "" {
			continue
		}
		key, raw, ok := strings.Cut(pair, "=")
		if !ok {
			return nil, fmt.Errorf("the schema tag's %q is not key=value", pair)
		}
		var v any
		if err := json.Unmarshal([]byte(raw), &v); err != nil {
			v = raw
		} else if _, ok := v.(float64); ok {
			// The number's own text, not a float's rounding of it.
			v = Number(strings.TrimSpace(raw))
		}
		out[strings.TrimSpace(key)] = v
	}
	return out, nil
}

// splitPairs splits on commas outside brackets, braces and quotes, so a
// JSON value may hold one.
func splitPairs(tag string) []string {
	var out []string
	depth, quoted, start := 0, false, 0
	for i := 0; i < len(tag); i++ {
		switch c := tag[i]; {
		case c == '"' && (i == 0 || tag[i-1] != '\\'):
			quoted = !quoted
		case quoted:
		case c == '[' || c == '{':
			depth++
		case c == ']' || c == '}':
			depth--
		case c == ',' && depth == 0:
			out = append(out, tag[start:i])
			start = i + 1
		}
	}
	return append(out, tag[start:])
}

// orderedMap is a map written with its keys in a stated order, so a
// struct's properties keep the order its fields were declared in.
type orderedMap struct {
	keys   []any
	values map[string]any
}

func (e *encoder) ordered(m orderedMap) error {
	if err := e.out.mapOf(len(m.keys)); err != nil {
		return err
	}
	for _, k := range m.keys {
		key := k.(string)
		if err := e.entry(key, func() error { return e.writeAny(m.values[key]) }); err != nil {
			return err
		}
	}
	return e.out.end()
}

var typeOrderedMap = reflect.TypeFor[orderedMap]()
