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
	"strconv"
	"time"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// decodeInto reads node into what dst points at; an absent node leaves it.
func decodeInto(node unsafe.Pointer, dst any, at string) error {
	rv := reflect.ValueOf(dst)
	if rv.Kind() != reflect.Pointer || rv.IsNil() {
		return &Error{Status: NullArgument, Message: fmt.Sprintf("Decode needs a non-nil pointer, not %T", dst)}
	}
	return decode(node, rv.Elem(), at, 0)
}

func decodeError(at string, node unsafe.Pointer, want string) error {
	msg := fmt.Sprintf("want %s, got %s", want, Tag(native.Tag(node)))
	if at != "" {
		msg = "at " + at + ": " + msg
	}
	return &Error{Status: BadValue, Message: msg}
}

func decodeFailed(at, format string, args ...any) error {
	msg := fmt.Sprintf(format, args...)
	if at != "" {
		msg = "at " + at + ": " + msg
	}
	return &Error{Status: BadValue, Message: msg}
}

func decode(node unsafe.Pointer, v reflect.Value, at string, depth int) error {
	tag := native.Tag(node)
	if tag == native.TagAbsent {
		return nil
	}
	if depth > native.MaxDepth {
		return decodeFailed(at, "the value nests deeper than %d", native.MaxDepth)
	}
	t := v.Type()
	switch t {
	case typeValue, typeRef, typeMapRef, typeListRef:
		return decodeFailed(at, "a %s borrows; decode into a *guatiao.Value", t)
	case typeNumber:
		if tag != native.TagNumber {
			return decodeError(at, node, "a number")
		}
		v.SetString(native.TextOf(node))
		return nil
	case typeTime:
		return decodeInstant(node, v, at)
	case typeDuration:
		return decodeLength(node, v, at)
	case typeBigInt:
		if tag != native.TagNumber {
			return decodeError(at, node, "an integer")
		}
		b, ok := new(big.Int).SetString(native.TextOf(node), 10)
		if !ok {
			return decodeFailed(at, "%s is not an integer", native.TextOf(node))
		}
		v.Set(reflect.ValueOf(*b))
		return nil
	}
	if t.Kind() == reflect.Pointer {
		if tag == native.TagNull {
			v.SetZero()
			return nil
		}
		if t.Elem() == typeValue {
			alloc, err := coreAlloc()
			if err != nil {
				return err
			}
			out := native.Calloc(native.SizeValue)
			if err := check(native.ValueClone(alloc, node, out), ""); err != nil {
				native.Free(out)
				return err
			}
			v.Set(reflect.ValueOf(adopt(out, alloc)))
			return nil
		}
		if v.IsNil() {
			v.Set(reflect.New(t.Elem()))
		}
		return decode(node, v.Elem(), at, depth)
	}
	if v.CanAddr() && reflect.PointerTo(t).Implements(typeUnmarshaler) && t.Kind() != reflect.Interface {
		here, lent := node, true
		r := Ref{o: &borrowed{find: func() unsafe.Pointer { return here }, open: func() bool { return lent }}}
		err := v.Addr().Interface().(Unmarshaler).UnmarshalGuatiao(r)
		lent = false
		if err != nil {
			return decodeFailed(at, "%v", err)
		}
		return nil
	}
	if v.CanAddr() && reflect.PointerTo(t).Implements(typeTextUnmarshaler) && t.Kind() != reflect.Interface {
		if tag != native.TagString {
			return decodeError(at, node, "text")
		}
		u := v.Addr().Interface().(encoding.TextUnmarshaler)
		if err := u.UnmarshalText([]byte(native.TextOf(node))); err != nil {
			return decodeFailed(at, "%v", err)
		}
		return nil
	}
	switch t.Kind() {
	case reflect.Interface:
		if t.NumMethod() != 0 {
			return decodeFailed(at, "cannot decode into a %s", t)
		}
		plain, err := plainOf(node, at, depth)
		if err != nil {
			return err
		}
		if plain == nil {
			v.SetZero()
		} else {
			v.Set(reflect.ValueOf(plain))
		}
		return nil
	case reflect.Bool:
		if tag != native.TagBool {
			return decodeError(at, node, "a bool")
		}
		v.SetBool(native.BoolOf(node))
		return nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		if tag != native.TagNumber {
			return decodeError(at, node, "an integer")
		}
		text := native.TextOf(node)
		i, err := strconv.ParseInt(text, 10, t.Bits())
		if err != nil {
			return decodeFailed(at, "%s is not a %s", text, t)
		}
		v.SetInt(i)
		return nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		if tag != native.TagNumber {
			return decodeError(at, node, "an integer")
		}
		text := native.TextOf(node)
		u, err := strconv.ParseUint(text, 10, t.Bits())
		if err != nil {
			return decodeFailed(at, "%s is not a %s", text, t)
		}
		v.SetUint(u)
		return nil
	case reflect.Float32, reflect.Float64:
		if tag != native.TagNumber {
			return decodeError(at, node, "a number")
		}
		text := native.TextOf(node)
		f, err := strconv.ParseFloat(text, t.Bits())
		if err != nil || math.IsInf(f, 0) {
			return decodeFailed(at, "%s does not fit a %s", text, t)
		}
		v.SetFloat(f)
		return nil
	case reflect.String:
		if tag != native.TagString {
			return decodeError(at, node, "a string")
		}
		v.SetString(native.TextOf(node))
		return nil
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 {
			if tag != native.TagBytes {
				return decodeError(at, node, "bytes")
			}
			p := native.Payload(node)
			v.SetBytes(native.Bytes(native.OwnedPtr(p), native.OwnedLen(p)))
			return nil
		}
		if tag == native.TagNull {
			v.SetZero()
			return nil
		}
		if tag != native.TagList {
			return decodeError(at, node, "a list")
		}
		n := int(native.Len(node))
		s := reflect.MakeSlice(t, n, n)
		for i := 0; i < n; i++ {
			if err := decode(native.ListElem(node, uintptr(i)), s.Index(i), at+"["+strconv.Itoa(i)+"]", depth+1); err != nil {
				return err
			}
		}
		v.Set(s)
		return nil
	case reflect.Array:
		if tag != native.TagList {
			return decodeError(at, node, "a list")
		}
		n := int(native.Len(node))
		if n != t.Len() {
			return decodeFailed(at, "want a list of exactly %d, got %d", t.Len(), n)
		}
		for i := 0; i < n; i++ {
			if err := decode(native.ListElem(node, uintptr(i)), v.Index(i), at+"["+strconv.Itoa(i)+"]", depth+1); err != nil {
				return err
			}
		}
		return nil
	case reflect.Map:
		if tag == native.TagNull {
			v.SetZero()
			return nil
		}
		if tag != native.TagMap {
			return decodeError(at, node, "a map")
		}
		if v.IsNil() {
			v.Set(reflect.MakeMap(t))
		}
		n := native.Len(node)
		for i := uintptr(0); i < n; i++ {
			e := native.EntryAt(node, i)
			key := native.EntryKey(e)
			kv := reflect.New(t.Key()).Elem()
			if err := decodeKey(key, kv); err != nil {
				return decodeFailed(at, "the key %q: %v", key, err)
			}
			ev := reflect.New(t.Elem()).Elem()
			if err := decode(native.EntryValue(e), ev, joinKey(at, key), depth+1); err != nil {
				return err
			}
			v.SetMapIndex(kv, ev)
		}
		return nil
	case reflect.Struct:
		if tag != native.TagMap {
			return decodeError(at, node, "a map")
		}
		for _, f := range fieldsOf(t) {
			child := native.Find(node, f.name)
			if child == nil {
				continue
			}
			fv, err := settableField(v, f.index)
			if err != nil {
				return decodeFailed(joinKey(at, f.name), "%v", err)
			}
			if err := decode(child, fv, joinKey(at, f.name), depth+1); err != nil {
				return err
			}
		}
		return nil
	}
	return decodeFailed(at, "cannot decode into a %s", t)
}

// settableField is the field at index, allocating embedded pointers on
// the way.
func settableField(v reflect.Value, index []int) (reflect.Value, error) {
	for i, x := range index {
		if i > 0 && v.Kind() == reflect.Pointer {
			if v.IsNil() {
				if !v.CanSet() {
					return reflect.Value{}, fmt.Errorf("an embedded %s is unexported", v.Type().Elem())
				}
				v.Set(reflect.New(v.Type().Elem()))
			}
			v = v.Elem()
		}
		v = v.Field(x)
	}
	return v, nil
}

func decodeKey(text string, kv reflect.Value) error {
	if reflect.PointerTo(kv.Type()).Implements(typeTextUnmarshaler) && kv.Kind() != reflect.String {
		return kv.Addr().Interface().(encoding.TextUnmarshaler).UnmarshalText([]byte(text))
	}
	switch kv.Kind() {
	case reflect.String:
		kv.SetString(text)
		return nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		i, err := strconv.ParseInt(text, 10, kv.Type().Bits())
		if err != nil || strconv.FormatInt(i, 10) != text {
			return fmt.Errorf("not a %s", kv.Type())
		}
		kv.SetInt(i)
		return nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		u, err := strconv.ParseUint(text, 10, kv.Type().Bits())
		if err != nil || strconv.FormatUint(u, 10) != text {
			return fmt.Errorf("not a %s", kv.Type())
		}
		kv.SetUint(u)
		return nil
	case reflect.Bool:
		switch text {
		case "true":
			kv.SetBool(true)
		case "false":
			kv.SetBool(false)
		default:
			return fmt.Errorf("not a bool")
		}
		return nil
	}
	return fmt.Errorf("a %s cannot key a map", kv.Type())
}

// plainOf is the node as plain Go data.
func plainOf(node unsafe.Pointer, at string, depth int) (any, error) {
	if depth > native.MaxDepth {
		return nil, decodeFailed(at, "the value nests deeper than %d", native.MaxDepth)
	}
	switch native.Tag(node) {
	case native.TagAbsent, native.TagNull:
		return nil, nil
	case native.TagBool:
		return native.BoolOf(node), nil
	case native.TagNumber:
		return Number(native.TextOf(node)), nil
	case native.TagString:
		return native.TextOf(node), nil
	case native.TagBytes:
		p := native.Payload(node)
		return native.Bytes(native.OwnedPtr(p), native.OwnedLen(p)), nil
	case native.TagList:
		n := int(native.Len(node))
		out := make([]any, n)
		for i := 0; i < n; i++ {
			x, err := plainOf(native.ListElem(node, uintptr(i)), at+"["+strconv.Itoa(i)+"]", depth+1)
			if err != nil {
				return nil, err
			}
			out[i] = x
		}
		return out, nil
	case native.TagMap:
		n := native.Len(node)
		out := make(map[string]any, n)
		for i := uintptr(0); i < n; i++ {
			e := native.EntryAt(node, i)
			key := native.EntryKey(e)
			x, err := plainOf(native.EntryValue(e), joinKey(at, key), depth+1)
			if err != nil {
				return nil, err
			}
			out[key] = x
		}
		return out, nil
	}
	// A tag this package does not know is skipped, as the header asks.
	return nil, nil
}

// decodeInstant reads an instant's text through the C surface's own
// reader, in UTC.
func decodeInstant(node unsafe.Pointer, v reflect.Value, at string) error {
	if native.Tag(node) != native.TagString {
		return decodeError(at, node, "an instant's text")
	}
	return instantInto(native.TextOf(node), v, at)
}

// instantInto reads an instant's text through the C surface's own reader,
// in UTC.
func instantInto(text string, v reflect.Value, at string) error {
	alloc, err := coreAlloc()
	if err != nil {
		return err
	}
	ts, refusal, st := native.TimestampParse(alloc, text)
	if st != native.OK {
		return decodeFailed(at, "not an instant: %s at byte %d", refusal.Expected, refusal.At)
	}
	v.Set(reflect.ValueOf(time.Unix(ts.Seconds, int64(ts.Nanoseconds)).UTC()))
	return nil
}

// decodeLength reads an exact length in any of its three spellings.
func decodeLength(node unsafe.Pointer, v reflect.Value, at string) error {
	if _, err := core(); err != nil {
		return err
	}
	secs, nanos, st := native.DurationRead(node)
	if st != native.OK {
		return decodeFailed(at, "not an exact length")
	}
	const most = uint64(math.MaxInt64)
	if secs > most/uint64(time.Second) || secs*uint64(time.Second) > most-uint64(nanos) {
		return decodeFailed(at, "%d.%09d seconds is longer than a time.Duration holds", secs, nanos)
	}
	v.SetInt(int64(secs*uint64(time.Second) + uint64(nanos)))
	return nil
}
