// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"encoding"
	"fmt"
	"math"
	"math/big"
	"reflect"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

var (
	typeValue           = reflect.TypeFor[Value]()
	typeRef             = reflect.TypeFor[Ref]()
	typeMapRef          = reflect.TypeFor[MapRef]()
	typeListRef         = reflect.TypeFor[ListRef]()
	typeNumber          = reflect.TypeFor[Number]()
	typeTime            = reflect.TypeFor[time.Time]()
	typeDuration        = reflect.TypeFor[time.Duration]()
	typeBigInt          = reflect.TypeFor[big.Int]()
	typeTextMarshaler   = reflect.TypeFor[encoding.TextMarshaler]()
	typeMarshaler       = reflect.TypeFor[Marshaler]()
	typeUnmarshaler     = reflect.TypeFor[Unmarshaler]()
	typeSchemer         = reflect.TypeFor[Schemer]()
	typeTextUnmarshaler = reflect.TypeFor[encoding.TextUnmarshaler]()
	typeAny             = reflect.TypeFor[any]()
	typeBytes           = reflect.TypeFor[[]byte]()
)

// Marshaler is a type that says what it is as a value: what
// MarshalGuatiao answers is converted in its place.
type Marshaler interface {
	MarshalGuatiao() (any, error)
}

// Unmarshaler is a type that reads itself from a value: Decode hands it a
// read-only Ref valid for the call.
type Unmarshaler interface {
	UnmarshalGuatiao(Ref) error
}

// Schemer is a type that states its own JSON Schema, for SchemaOf.
type Schemer interface {
	GuatiaoSchema() map[string]any
}

// writeGo writes x into the node at raw, which holds nothing. On any
// failure raw is freed and left absent, so nobody holds half a tree.
func writeGo(raw unsafe.Pointer, x any, alloc unsafe.Pointer) error {
	e := encoder{alloc: alloc, seen: map[uintptr]bool{}}
	if err := e.write(raw, reflect.ValueOf(x), ""); err != nil {
		native.ValueFree(raw)
		native.Zero(raw, native.SizeValue)
		return err
	}
	return nil
}

type encoder struct {
	alloc unsafe.Pointer
	seen  map[uintptr]bool
	depth int
}

func encodeError(at, format string, args ...any) error {
	msg := fmt.Sprintf(format, args...)
	if at != "" {
		msg = "at " + at + ": " + msg
	}
	return &Error{Status: BadValue, Message: msg}
}

func joinKey(at, key string) string {
	if at == "" {
		return key
	}
	return at + "." + key
}

func (e *encoder) write(raw unsafe.Pointer, v reflect.Value, at string) error {
	if !v.IsValid() {
		return check(native.ValueNull(raw), "")
	}
	e.depth++
	defer func() { e.depth-- }()
	if e.depth > native.MaxDepth {
		return encodeError(at, "the value nests deeper than %d", native.MaxDepth)
	}
	t := v.Type()
	switch t {
	case typeValue, typeRef, typeMapRef, typeListRef:
		return e.clone(raw, v, at)
	case typeNumber:
		return e.number(raw, v.String(), at)
	case typeTime:
		return e.instant(raw, v.Interface().(time.Time), at)
	case typeDuration:
		return e.length(raw, time.Duration(v.Int()), at)
	case typeBigInt:
		b := v.Interface().(big.Int)
		return e.number(raw, b.String(), at)
	case typeOrderedMap:
		return e.ordered(raw, v.Interface().(orderedMap), at)
	}
	if t.Kind() == reflect.Pointer {
		if v.IsNil() {
			return check(native.ValueNull(raw), "")
		}
		switch t.Elem() {
		case typeValue, typeBigInt:
			return e.write(raw, v.Elem(), at)
		}
	}
	if t.Kind() != reflect.Interface && t.Implements(typeMarshaler) && !(t.Kind() == reflect.Pointer && v.IsNil()) {
		x, err := v.Interface().(Marshaler).MarshalGuatiao()
		if err != nil {
			return encodeError(at, "%v", err)
		}
		return e.write(raw, reflect.ValueOf(x), at)
	}
	if t.Kind() != reflect.Pointer && t.Kind() != reflect.Interface && t.Implements(typeTextMarshaler) {
		return e.text(raw, v.Interface().(encoding.TextMarshaler), at)
	}
	if t.Kind() == reflect.Pointer && t.Implements(typeTextMarshaler) && !v.IsNil() {
		return e.text(raw, v.Interface().(encoding.TextMarshaler), at)
	}
	switch t.Kind() {
	case reflect.Bool:
		return check(native.ValueBool(v.Bool(), raw), "")
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return e.number(raw, strconv.FormatInt(v.Int(), 10), at)
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return e.number(raw, strconv.FormatUint(v.Uint(), 10), at)
	case reflect.Float32, reflect.Float64:
		f := v.Float()
		if math.IsInf(f, 0) || math.IsNaN(f) {
			return encodeError(at, "%v is not a finite number", f)
		}
		bits := 64
		if t.Kind() == reflect.Float32 {
			bits = 32
		}
		text := strconv.FormatFloat(f, 'g', -1, bits)
		if f == 0 && math.Signbit(f) {
			text = "-0"
		}
		return e.number(raw, text, at)
	case reflect.String:
		return e.str(raw, v.String(), at)
	case reflect.Interface:
		if v.IsNil() {
			return check(native.ValueNull(raw), "")
		}
		return e.write(raw, v.Elem(), at)
	case reflect.Pointer:
		if v.IsNil() {
			return check(native.ValueNull(raw), "")
		}
		ptr := v.Pointer()
		if e.seen[ptr] {
			return encodeError(at, "the value refers to itself")
		}
		e.seen[ptr] = true
		defer delete(e.seen, ptr)
		return e.write(raw, v.Elem(), at)
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 && !t.Elem().Implements(typeTextMarshaler) {
			return check(native.ValueBytes(e.alloc, v.Bytes(), raw), "")
		}
		if !v.IsNil() {
			ptr := v.Pointer()
			if v.Len() > 0 && e.seen[ptr] {
				return encodeError(at, "the value refers to itself")
			}
			e.seen[ptr] = true
			defer delete(e.seen, ptr)
		}
		return e.list(raw, v, at)
	case reflect.Array:
		return e.list(raw, v, at)
	case reflect.Map:
		if v.IsNil() {
			return check(native.ValueMap(e.alloc, raw), "")
		}
		ptr := v.Pointer()
		if e.seen[ptr] {
			return encodeError(at, "the value refers to itself")
		}
		e.seen[ptr] = true
		defer delete(e.seen, ptr)
		return e.mapOf(raw, v, at)
	case reflect.Struct:
		return e.record(raw, v, at)
	}
	return encodeError(at, "a %s has no value form", t)
}

func (e *encoder) clone(raw unsafe.Pointer, v reflect.Value, at string) error {
	var r Ref
	switch x := v.Interface().(type) {
	case Value:
		r = x.Ref
	case Ref:
		r = x
	case MapRef:
		r = x.Ref
	case ListRef:
		r = x.Ref
	}
	node, err := r.requireNode()
	if err != nil {
		return err
	}
	return check(native.ValueClone(e.alloc, node, raw), "")
}

func (e *encoder) number(raw unsafe.Pointer, text, at string) error {
	if err := check(native.ValueNumber(e.alloc, text, raw), ""); err != nil {
		return encodeError(at, "%q is not a number", text)
	}
	return nil
}

func (e *encoder) text(raw unsafe.Pointer, m encoding.TextMarshaler, at string) error {
	text, err := m.MarshalText()
	if err != nil {
		return encodeError(at, "%v", err)
	}
	return e.str(raw, string(text), at)
}

func (e *encoder) str(raw unsafe.Pointer, text, at string) error {
	st := native.ValueString(e.alloc, text, raw)
	if st == native.ErrBadValue {
		return encodeError(at, "the text is not UTF-8")
	}
	return check(st, "")
}

// instant writes t as an instant's text, through the C surface's own
// writer.
func (e *encoder) instant(raw unsafe.Pointer, t time.Time, at string) error {
	ts := native.Timestamp{Seconds: t.Unix(), Nanoseconds: uint32(t.Nanosecond())}
	if err := check(native.TimestampFormat(e.alloc, ts, raw), ""); err != nil {
		return encodeError(at, "%s has no instant text: %v", t, err)
	}
	return nil
}

// length writes d as an exact length, [seconds, nanoseconds].
func (e *encoder) length(raw unsafe.Pointer, d time.Duration, at string) error {
	if d < 0 {
		return encodeError(at, "an exact length is never negative, and %s is", d)
	}
	secs := uint64(d / time.Second)
	nanos := uint64(d % time.Second)
	return e.write(raw, reflect.ValueOf([2]uint64{secs, nanos}), at)
}

func (e *encoder) list(raw unsafe.Pointer, v reflect.Value, at string) error {
	if err := check(native.ValueList(e.alloc, raw), ""); err != nil {
		return err
	}
	return withScratch(func(tmp unsafe.Pointer) error {
		for i := 0; i < v.Len(); i++ {
			if err := e.write(tmp, v.Index(i), at+"["+strconv.Itoa(i)+"]"); err != nil {
				native.ValueFree(tmp)
				return err
			}
			st := native.ListPush(e.alloc, raw, tmp)
			native.ValueFree(tmp)
			if err := check(st, ""); err != nil {
				return err
			}
		}
		return nil
	})
}

// keyText is a map key's text: a string as it is, an integer in decimal,
// a TextMarshaler's text.
func keyText(k reflect.Value) (string, error) {
	if k.Type().Implements(typeTextMarshaler) && k.Kind() != reflect.String {
		b, err := k.Interface().(encoding.TextMarshaler).MarshalText()
		return string(b), err
	}
	switch k.Kind() {
	case reflect.String:
		return k.String(), nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return strconv.FormatInt(k.Int(), 10), nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return strconv.FormatUint(k.Uint(), 10), nil
	case reflect.Bool:
		return strconv.FormatBool(k.Bool()), nil
	}
	return "", fmt.Errorf("a map keyed by %s has no value form", k.Type())
}

func (e *encoder) mapOf(raw unsafe.Pointer, v reflect.Value, at string) error {
	type kv struct {
		key string
		num bool
		n   int64
		u   uint64
		val reflect.Value
	}
	entries := make([]kv, 0, v.Len())
	iter := v.MapRange()
	for iter.Next() {
		text, err := keyText(iter.Key())
		if err != nil {
			return encodeError(at, "%v", err)
		}
		item := kv{key: text, val: iter.Value()}
		switch iter.Key().Kind() {
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			item.num, item.n = true, iter.Key().Int()
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
			item.num, item.u = true, iter.Key().Uint()
		}
		entries = append(entries, item)
	}
	// A Go map has no order; keys are written sorted, integers by value.
	slices.SortFunc(entries, func(a, b kv) int {
		if a.num && b.num {
			if a.n != b.n {
				return compare(a.n, b.n)
			}
			return compare(a.u, b.u)
		}
		return strings.Compare(a.key, b.key)
	})
	if err := check(native.ValueMap(e.alloc, raw), ""); err != nil {
		return err
	}
	return withScratch(func(tmp unsafe.Pointer) error {
		for _, item := range entries {
			if err := e.set(raw, tmp, item.key, item.val, at); err != nil {
				return err
			}
		}
		return nil
	})
}

func compare[T int64 | uint64](a, b T) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}

func (e *encoder) set(raw, tmp unsafe.Pointer, key string, v reflect.Value, at string) error {
	if err := e.write(tmp, v, joinKey(at, key)); err != nil {
		native.ValueFree(tmp)
		return err
	}
	st := native.MapSet(e.alloc, raw, key, tmp)
	native.ValueFree(tmp)
	if st == native.ErrBadValue {
		return encodeError(at, "the key %q is not UTF-8", key)
	}
	return check(st, "")
}

func (e *encoder) record(raw unsafe.Pointer, v reflect.Value, at string) error {
	fields := fieldsOf(v.Type())
	if err := check(native.ValueMap(e.alloc, raw), ""); err != nil {
		return err
	}
	return withScratch(func(tmp unsafe.Pointer) error {
		for _, f := range fields {
			fv, ok := fieldByIndex(v, f.index)
			if !ok {
				continue
			}
			if omitted(fv, f.omitEmpty) {
				continue
			}
			if err := e.set(raw, tmp, f.name, fv, at); err != nil {
				return err
			}
		}
		return nil
	})
}

// omitted reports whether a field is left out: a nil pointer always (an
// absent field is omitted), a zero value under omitempty.
func omitted(v reflect.Value, omitEmpty bool) bool {
	if v.Kind() == reflect.Pointer && v.IsNil() {
		return true
	}
	if !omitEmpty {
		return false
	}
	switch v.Kind() {
	case reflect.Slice, reflect.Map, reflect.String, reflect.Array:
		return v.Len() == 0
	case reflect.Interface:
		return v.IsNil()
	}
	return v.IsZero()
}

// fieldByIndex is the field at index, or false when an embedded pointer
// on the way is nil.
func fieldByIndex(v reflect.Value, index []int) (reflect.Value, bool) {
	for i, x := range index {
		if i > 0 && v.Kind() == reflect.Pointer {
			if v.IsNil() {
				return reflect.Value{}, false
			}
			v = v.Elem()
		}
		v = v.Field(x)
	}
	return v, true
}

// field is one struct field as a map key.
type field struct {
	name      string
	index     []int
	typ       reflect.Type
	omitEmpty bool
	schema    string
}

var fieldCache sync.Map // reflect.Type -> []field

// fieldsOf are a struct's fields as map keys, in declaration order: a
// `guatiao` tag names one (a `json` tag when there is none), "-" leaves it
// out, an untagged embedded struct lends its fields, and an unexported
// field is never one.
func fieldsOf(t reflect.Type) []field {
	if cached, ok := fieldCache.Load(t); ok {
		return cached.([]field)
	}
	var out []field
	seen := map[string]int{}
	var walk func(t reflect.Type, index []int, depth int)
	walk = func(t reflect.Type, index []int, depth int) {
		for i := 0; i < t.NumField(); i++ {
			sf := t.Field(i)
			tag, has := sf.Tag.Lookup("guatiao")
			if !has {
				tag, has = sf.Tag.Lookup("json")
			}
			if tag == "-" {
				continue
			}
			name, opts, _ := strings.Cut(tag, ",")
			idx := append(slices.Clone(index), i)
			ft := sf.Type
			if sf.Anonymous && name == "" {
				et := ft
				if et.Kind() == reflect.Pointer {
					et = et.Elem()
				}
				if et.Kind() == reflect.Struct && depth < 8 {
					walk(et, idx, depth+1)
					continue
				}
			}
			if !sf.IsExported() {
				continue
			}
			if name == "" {
				name = sf.Name
			}
			f := field{name: name, index: idx, typ: ft, omitEmpty: hasOpt(opts, "omitempty"), schema: sf.Tag.Get("schema")}
			if at, dup := seen[name]; dup {
				// The shallower field wins, as encoding/json has it.
				if len(out[at].index) > len(idx) {
					out[at] = f
				}
				continue
			}
			seen[name] = len(out)
			out = append(out, f)
		}
	}
	walk(t, nil, 0)
	fieldCache.Store(t, out)
	return out
}

func hasOpt(opts, want string) bool {
	for opts != "" {
		var o string
		o, opts, _ = strings.Cut(opts, ",")
		if o == want {
			return true
		}
	}
	return false
}
