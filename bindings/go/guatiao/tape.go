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
	"sync"
	"time"
	"unicode/utf8"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// The wire encoding written from Go data and read into it in Go, with no
// tree built: one crossing carries a whole document, where building or
// reading a tree costs a call per node.

const (
	wireAbsent byte = iota
	wireNull
	wireBool
	wireNumber
	wireString
	wireBytes
	wireList
	wireMap
)

// MarshalWire is the wire encoding of what NewValue(x) builds, written in
// Go with no tree built: the same Go types taken, the same errors. Only a
// time.Time, or a Value or view inside x, needs the guatiao library.
func MarshalWire(x any) ([]byte, error) {
	s := &tapeSink{buf: make([]byte, 0, 256)}
	e := encoder{out: s}
	if err := e.writeAny(x); err != nil {
		return nil, err
	}
	return s.buf, nil
}

// tapeSink writes events as wire bytes.
type tapeSink struct {
	buf []byte
}

func (s *tapeSink) leb(n uint64) {
	for n >= 0x80 {
		s.buf = append(s.buf, byte(n)|0x80)
		n >>= 7
	}
	s.buf = append(s.buf, byte(n))
}

func (s *tapeSink) body(tag byte, text string) {
	s.buf = append(s.buf, tag)
	s.leb(uint64(len(text)))
	s.buf = append(s.buf, text...)
}

func (s *tapeSink) null() error { s.buf = append(s.buf, wireNull); return nil }

func (s *tapeSink) boolean(b bool) error {
	v := byte(0)
	if b {
		v = 1
	}
	s.buf = append(s.buf, wireBool, v)
	return nil
}

func (s *tapeSink) number(text string) error { s.body(wireNumber, text); return nil }
func (s *tapeSink) text(t string) error      { s.body(wireString, t); return nil }

func (s *tapeSink) bytes(b []byte) error {
	s.buf = append(s.buf, wireBytes)
	s.leb(uint64(len(b)))
	s.buf = append(s.buf, b...)
	return nil
}

func (s *tapeSink) node(r Ref) error {
	data, err := r.ToWire()
	if err != nil {
		return err
	}
	s.buf = append(s.buf, data...)
	return nil
}

// instant writes t's text as the C surface's own writer spells it.
func (s *tapeSink) instant(t time.Time) error {
	alloc, err := coreAlloc()
	if err != nil {
		return err
	}
	out := native.Calloc(native.SizeValue)
	defer freeRoot(out)
	ts := native.Timestamp{Seconds: t.Unix(), Nanoseconds: uint32(t.Nanosecond())}
	if err := check(native.TimestampFormat(alloc, ts, out), ""); err != nil {
		return err
	}
	s.body(wireString, native.TextOf(out))
	return nil
}

func (s *tapeSink) list(n int) error {
	s.buf = append(s.buf, wireList)
	s.leb(uint64(n))
	return nil
}

func (s *tapeSink) mapOf(n int) error {
	s.buf = append(s.buf, wireMap)
	s.leb(uint64(n))
	return nil
}

func (s *tapeSink) key(k string) error {
	s.leb(uint64(len(k)))
	s.buf = append(s.buf, k...)
	return nil
}

func (s *tapeSink) end() error { return nil }

// UnmarshalWire reads the value the wire bytes in data hold into what
// dst points at, as FromWire(data) then Decode(dst) would, with no tree
// built. ErrBadValue for bytes cut short, bytes after the value, an
// unknown tag, a length not in its shortest form, text that is not
// UTF-8, a number outside the grammar or nesting past the limit; a
// repeated key is not looked for. Only a time.Time, a time.Duration, a
// *Value or an Unmarshaler inside dst needs the guatiao library.
func UnmarshalWire(data []byte, dst any) error {
	rv := reflect.ValueOf(dst)
	if rv.Kind() != reflect.Pointer || rv.IsNil() {
		return &Error{Status: NullArgument, Message: fmt.Sprintf("UnmarshalWire needs a non-nil pointer, not %T", dst)}
	}
	t := tape{data: data}
	if err := t.decode(rv.Elem(), 0); err != nil {
		return err
	}
	if t.at != len(data) {
		return wireError("bytes after the value, from %d", t.at)
	}
	return nil
}

func wireError(format string, args ...any) error {
	return &Error{Status: BadValue, Message: fmt.Sprintf(format, args...)}
}

var errCutShort = &Error{Status: BadValue, Message: "the bytes end inside a node"}

// tape reads wire bytes from at; path is where the node being read sits,
// spelled only for an error.
type tape struct {
	data []byte
	at   int
	path []pathStep
}

func (t *tape) where() string { return spell(t.path) }

func (t *tape) byte() (byte, error) {
	if t.at >= len(t.data) {
		return 0, errCutShort
	}
	b := t.data[t.at]
	t.at++
	return b, nil
}

// boolean is a bool's byte, 0 or 1.
func (t *tape) boolean() (bool, error) {
	b, err := t.byte()
	if err != nil {
		return false, err
	}
	if b > 1 {
		return false, wireError("a bool that is neither 0 nor 1 at %d", t.at-1)
	}
	return b == 1, nil
}

// count is a LEB128 count, refused unless in its shortest form and
// within an int.
func (t *tape) count() (int, error) {
	var n uint64
	for shift := 0; ; shift += 7 {
		b, err := t.byte()
		if err != nil {
			return 0, err
		}
		if shift == 63 && b > 1 || shift > 63 {
			return 0, wireError("a length past 64 bits at %d", t.at-1)
		}
		n |= uint64(b&0x7f) << shift
		if b < 0x80 {
			if b == 0 && shift > 0 {
				return 0, wireError("a length not in its shortest form at %d", t.at-1)
			}
			if n > math.MaxInt {
				return 0, wireError("a length past what this platform holds at %d", t.at-1)
			}
			return int(n), nil
		}
	}
}

// body is the next n bytes, in place.
func (t *tape) body(n int) ([]byte, error) {
	if n > len(t.data)-t.at {
		return nil, errCutShort
	}
	b := t.data[t.at : t.at+n]
	t.at += n
	return b, nil
}

// text is a key or a string's body, checked UTF-8 and copied.
func (t *tape) text() (string, error) {
	n, err := t.count()
	if err != nil {
		return "", err
	}
	b, err := t.body(n)
	if err != nil {
		return "", err
	}
	if !utf8.Valid(b) {
		return "", wireError("text that is not UTF-8 before %d", t.at)
	}
	return string(b), nil
}

// scalar reads the body of a number, string or bytes node whose tag was
// just read: the number's or string's text, or a copy of the bytes.
func (t *tape) scalar(tag byte) (string, []byte, error) {
	if tag == wireString {
		s, err := t.text()
		return s, nil, err
	}
	n, err := t.count()
	if err != nil {
		return "", nil, err
	}
	b, err := t.body(n)
	if err != nil {
		return "", nil, err
	}
	if tag == wireBytes {
		return "", append([]byte{}, b...), nil
	}
	text := string(b)
	if !isNumber(text) {
		return "", nil, wireError("a number outside the grammar before %d", t.at)
	}
	return text, nil, nil
}

// skip passes over one whole node whose tag was just read.
func (t *tape) skip(tag byte, depth int) error {
	switch tag {
	case wireAbsent, wireNull:
		return nil
	case wireBool:
		_, err := t.boolean()
		return err
	case wireNumber, wireString, wireBytes:
		_, _, err := t.scalar(tag)
		return err
	case wireList, wireMap:
		n, err := t.items()
		if err != nil {
			return err
		}
		if n > 0 && depth >= native.MaxDepth {
			return wireError("the value nests deeper than %d", native.MaxDepth)
		}
		for i := 0; i < n; i++ {
			if tag == wireMap {
				if _, err := t.text(); err != nil {
					return err
				}
			}
			child, err := t.byte()
			if err != nil {
				return err
			}
			if err := t.skip(child, depth+1); err != nil {
				return err
			}
		}
		return nil
	}
	return wireError("unknown tag %d at %d", tag, t.at-1)
}

// subtree is the whole node whose tag is at start, read past, as a Value
// built from its bytes: for what needs a tree.
func (t *tape) subtree(tag byte, depth int) (*Value, error) {
	start := t.at - 1
	if err := t.skip(tag, depth); err != nil {
		return nil, err
	}
	return FromWire(t.data[start:t.at])
}

var tagOfWire = [...]uint32{
	wireAbsent: native.TagAbsent, wireNull: native.TagNull, wireBool: native.TagBool,
	wireNumber: native.TagNumber, wireString: native.TagString, wireBytes: native.TagBytes,
	wireList: native.TagList, wireMap: native.TagMap,
}

func tapeError(at string, tag byte, want string) error {
	got := "tag " + strconv.Itoa(int(tag))
	if int(tag) < len(tagOfWire) {
		got = Tag(tagOfWire[tag]).String()
	}
	msg := fmt.Sprintf("want %s, got %s", want, got)
	if at != "" {
		msg = "at " + at + ": " + msg
	}
	return &Error{Status: BadValue, Message: msg}
}

// decode reads the next node into v, by the rules decode (the tree
// reader) follows, and in the same order of checks.
func (t *tape) decode(v reflect.Value, depth int) error {
	tag, err := t.byte()
	if err != nil {
		return err
	}
	if tag > wireMap {
		return wireError("unknown tag %d at %d", tag, t.at-1)
	}
	if tag == wireAbsent {
		return nil
	}
	if depth > native.MaxDepth {
		return decodeFailed(t.where(), "the value nests deeper than %d", native.MaxDepth)
	}
	ty := v.Type()
	switch ty {
	case typeValue, typeRef, typeMapRef, typeListRef:
		return decodeFailed(t.where(), "a %s borrows; decode into a *guatiao.Value", ty)
	case typeNumber:
		if tag != wireNumber {
			return tapeError(t.where(), tag, "a number")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		v.SetString(text)
		return nil
	case typeTime:
		if tag != wireString {
			return tapeError(t.where(), tag, "an instant's text")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		return instantInto(text, v, t.where())
	case typeDuration:
		return t.viaTree(tag, depth, func(node unsafe.Pointer) error { return decodeLength(node, v, t.where()) })
	case typeBigInt:
		if tag != wireNumber {
			return tapeError(t.where(), tag, "an integer")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		b, ok := new(big.Int).SetString(text, 10)
		if !ok {
			return decodeFailed(t.where(), "%s is not an integer", text)
		}
		v.Set(reflect.ValueOf(*b))
		return nil
	}
	if ty.Kind() == reflect.Pointer {
		if tag == wireNull {
			v.SetZero()
			return nil
		}
		if ty.Elem() == typeValue {
			sub, err := t.subtree(tag, depth)
			if err != nil {
				return err
			}
			v.Set(reflect.ValueOf(sub))
			return nil
		}
		if v.IsNil() {
			v.Set(reflect.New(ty.Elem()))
		}
		t.at--
		return t.decode(v.Elem(), depth)
	}
	if v.CanAddr() && reflect.PointerTo(ty).Implements(typeUnmarshaler) && ty.Kind() != reflect.Interface {
		return t.viaTree(tag, depth, func(node unsafe.Pointer) error { return decode(node, v, t.where(), depth) })
	}
	if v.CanAddr() && reflect.PointerTo(ty).Implements(typeTextUnmarshaler) && ty.Kind() != reflect.Interface {
		if tag != wireString {
			return tapeError(t.where(), tag, "text")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		if err := v.Addr().Interface().(encoding.TextUnmarshaler).UnmarshalText([]byte(text)); err != nil {
			return decodeFailed(t.where(), "%v", err)
		}
		return nil
	}
	switch ty.Kind() {
	case reflect.Interface:
		if ty.NumMethod() != 0 {
			return decodeFailed(t.where(), "cannot decode into a %s", ty)
		}
		plain, err := t.plain(tag, depth)
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
		if tag != wireBool {
			return tapeError(t.where(), tag, "a bool")
		}
		b, err := t.boolean()
		if err != nil {
			return err
		}
		v.SetBool(b)
		return nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		if tag != wireNumber {
			return tapeError(t.where(), tag, "an integer")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		i, err := strconv.ParseInt(text, 10, ty.Bits())
		if err != nil {
			return decodeFailed(t.where(), "%s is not a %s", text, ty)
		}
		v.SetInt(i)
		return nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		if tag != wireNumber {
			return tapeError(t.where(), tag, "an integer")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		u, err := strconv.ParseUint(text, 10, ty.Bits())
		if err != nil {
			return decodeFailed(t.where(), "%s is not a %s", text, ty)
		}
		v.SetUint(u)
		return nil
	case reflect.Float32, reflect.Float64:
		if tag != wireNumber {
			return tapeError(t.where(), tag, "a number")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		f, err := strconv.ParseFloat(text, ty.Bits())
		if err != nil || math.IsInf(f, 0) {
			return decodeFailed(t.where(), "%s does not fit a %s", text, ty)
		}
		v.SetFloat(f)
		return nil
	case reflect.String:
		if tag != wireString {
			return tapeError(t.where(), tag, "a string")
		}
		text, _, err := t.scalar(tag)
		if err != nil {
			return err
		}
		v.SetString(text)
		return nil
	case reflect.Slice:
		if ty.Elem().Kind() == reflect.Uint8 {
			if tag != wireBytes {
				return tapeError(t.where(), tag, "bytes")
			}
			_, b, err := t.scalar(tag)
			if err != nil {
				return err
			}
			v.SetBytes(b)
			return nil
		}
		if tag == wireNull {
			v.SetZero()
			return nil
		}
		if tag != wireList {
			return tapeError(t.where(), tag, "a list")
		}
		n, err := t.items()
		if err != nil {
			return err
		}
		s := reflect.MakeSlice(ty, n, n)
		t.path = append(t.path, pathStep{})
		for i := 0; i < n; i++ {
			t.path[len(t.path)-1].index = i
			if err := t.decode(s.Index(i), depth+1); err != nil {
				return err
			}
		}
		t.path = t.path[:len(t.path)-1]
		v.Set(s)
		return nil
	case reflect.Array:
		if tag != wireList {
			return tapeError(t.where(), tag, "a list")
		}
		n, err := t.items()
		if err != nil {
			return err
		}
		if n != ty.Len() {
			return decodeFailed(t.where(), "want a list of exactly %d, got %d", ty.Len(), n)
		}
		t.path = append(t.path, pathStep{})
		for i := 0; i < n; i++ {
			t.path[len(t.path)-1].index = i
			if err := t.decode(v.Index(i), depth+1); err != nil {
				return err
			}
		}
		t.path = t.path[:len(t.path)-1]
		return nil
	case reflect.Map:
		if tag == wireNull {
			v.SetZero()
			return nil
		}
		if tag != wireMap {
			return tapeError(t.where(), tag, "a map")
		}
		n, err := t.items()
		if err != nil {
			return err
		}
		if v.IsNil() {
			v.Set(reflect.MakeMapWithSize(ty, n))
		}
		for i := 0; i < n; i++ {
			key, err := t.text()
			if err != nil {
				return err
			}
			kv := reflect.New(ty.Key()).Elem()
			if err := decodeKey(key, kv); err != nil {
				return decodeFailed(t.where(), "the key %q: %v", key, err)
			}
			ev := reflect.New(ty.Elem()).Elem()
			t.path = append(t.path, pathStep{key: key, index: -1})
			if err := t.decode(ev, depth+1); err != nil {
				return err
			}
			t.path = t.path[:len(t.path)-1]
			v.SetMapIndex(kv, ev)
		}
		return nil
	case reflect.Struct:
		if tag != wireMap {
			return tapeError(t.where(), tag, "a map")
		}
		n, err := t.items()
		if err != nil {
			return err
		}
		fields := fieldIndex(ty)
		for i := 0; i < n; i++ {
			key, err := t.text()
			if err != nil {
				return err
			}
			f, ok := fields[key]
			if !ok {
				child, err := t.byte()
				if err != nil {
					return err
				}
				if err := t.skip(child, depth+1); err != nil {
					return err
				}
				continue
			}
			t.path = append(t.path, pathStep{key: key, index: -1})
			fv, err := settableField(v, f.index)
			if err != nil {
				return decodeFailed(t.where(), "%v", err)
			}
			if err := t.decode(fv, depth+1); err != nil {
				return err
			}
			t.path = t.path[:len(t.path)-1]
		}
		return nil
	}
	return decodeFailed(t.where(), "cannot decode into a %s", ty)
}

// items is a list's or map's count, refused when more items than bytes
// remain (before anything is made that large); each item checks the
// depth itself.
func (t *tape) items() (int, error) {
	n, err := t.count()
	if err == nil && n > len(t.data)-t.at {
		return 0, wireError("a count past what remains at %d", t.at)
	}
	return n, err
}

// viaTree builds the node whose tag was just read as a tree and hands it
// to read: for what reads a tree (an Unmarshaler, an exact length).
func (t *tape) viaTree(tag byte, depth int, read func(node unsafe.Pointer) error) error {
	sub, err := t.subtree(tag, depth)
	if err != nil {
		return err
	}
	defer sub.Close()
	node, err := sub.node()
	if err != nil {
		return err
	}
	return read(node)
}

// plain is the node whose tag was just read as plain Go data, as plainOf
// reads a tree.
func (t *tape) plain(tag byte, depth int) (any, error) {
	if depth > native.MaxDepth {
		return nil, decodeFailed(t.where(), "the value nests deeper than %d", native.MaxDepth)
	}
	switch tag {
	case wireAbsent, wireNull:
		return nil, nil
	case wireBool:
		return t.boolean()
	case wireNumber:
		text, _, err := t.scalar(tag)
		return Number(text), err
	case wireString:
		text, _, err := t.scalar(tag)
		return text, err
	case wireBytes:
		_, b, err := t.scalar(tag)
		return b, err
	case wireList:
		n, err := t.items()
		if err != nil {
			return nil, err
		}
		out := make([]any, n)
		t.path = append(t.path, pathStep{})
		for i := 0; i < n; i++ {
			t.path[len(t.path)-1].index = i
			child, err := t.byte()
			if err != nil {
				return nil, err
			}
			if out[i], err = t.plain(child, depth+1); err != nil {
				return nil, err
			}
		}
		t.path = t.path[:len(t.path)-1]
		return out, nil
	case wireMap:
		n, err := t.items()
		if err != nil {
			return nil, err
		}
		out := make(map[string]any, n)
		for i := 0; i < n; i++ {
			key, err := t.text()
			if err != nil {
				return nil, err
			}
			child, err := t.byte()
			if err != nil {
				return nil, err
			}
			t.path = append(t.path, pathStep{key: key, index: -1})
			if out[key], err = t.plain(child, depth+1); err != nil {
				return nil, err
			}
			t.path = t.path[:len(t.path)-1]
		}
		return out, nil
	}
	return nil, wireError("unknown tag %d at %d", tag, t.at-1)
}

var fieldIndexByType sync.Map // reflect.Type -> map[string]field

// fieldIndex is fieldsOf(t) by name.
func fieldIndex(t reflect.Type) map[string]field {
	if cached, ok := fieldIndexByType.Load(t); ok {
		return cached.(map[string]field)
	}
	fields := fieldsOf(t)
	byName := make(map[string]field, len(fields))
	for _, f := range fields {
		byName[f.name] = f
	}
	fieldIndexByType.Store(t, byName)
	return byName
}
