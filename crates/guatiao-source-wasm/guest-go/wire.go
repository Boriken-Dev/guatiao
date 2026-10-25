// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guest

import (
	"errors"
	"fmt"
	"math"
	"sort"
	"strconv"
	"unicode/utf8"
)

// A value is one of: Absent, nil (null), bool, Number, string, []byte,
// []any (a list) or *Map. Encoding also takes Go's integers and floats
// (as a Number), and map[string]any (its keys sorted).

// AbsentValue is the type of Absent.
type AbsentValue struct{}

// Absent is no value: what a lookup that found nothing answers.
var Absent = AbsentValue{}

// Number is a number as its exact text, in the JSON number grammar:
// "1.10" stays "1.10", and a 40-digit integer loses nothing.
type Number string

// Int is the number as an int64, when it is one exactly.
func (n Number) Int() (int64, bool) {
	i, err := strconv.ParseInt(string(n), 10, 64)
	return i, err == nil
}

// Uint is the number as a uint64, when it is one exactly.
func (n Number) Uint() (uint64, bool) {
	u, err := strconv.ParseUint(string(n), 10, 64)
	return u, err == nil
}

// Float is the number as a float64.
func (n Number) Float() (float64, bool) {
	f, err := strconv.ParseFloat(string(n), 64)
	return f, err == nil
}

// Map is a map that keeps its keys' order.
type Map struct {
	keys   []string
	values map[string]any
}

// NewMap is an empty map.
func NewMap() *Map {
	return &Map{values: map[string]any{}}
}

// Set sets key to value: a new key goes last. It answers the map.
func (m *Map) Set(key string, value any) *Map {
	if _, ok := m.values[key]; !ok {
		m.keys = append(m.keys, key)
	}
	m.values[key] = value
	return m
}

// Get is the value under key, and whether there is one.
func (m *Map) Get(key string) (any, bool) {
	v, ok := m.values[key]
	return v, ok
}

// Keys are its keys, in order.
func (m *Map) Keys() []string {
	return append([]string(nil), m.keys...)
}

// Len is how many entries it has.
func (m *Map) Len() int {
	return len(m.keys)
}

// WireError says why bytes are not a value, and at which byte.
type WireError struct {
	At  int
	Why string
}

func (e *WireError) Error() string {
	return fmt.Sprintf("%s at byte %d", e.Why, e.At)
}

const (
	tagAbsent = 0
	tagNull   = 1
	tagBool   = 2
	tagNumber = 3
	tagString = 4
	tagBytes  = 5
	tagList   = 6
	tagMap    = 7
)

// ValidNumber is whether text is in the JSON number grammar.
func ValidNumber(text string) bool {
	i, n := 0, len(text)
	digits := func() int {
		start := i
		for i < n && text[i] >= '0' && text[i] <= '9' {
			i++
		}
		return i - start
	}
	if i < n && text[i] == '-' {
		i++
	}
	if i < n && text[i] == '0' {
		i++
	} else if i < n && text[i] >= '1' && text[i] <= '9' {
		digits()
	} else {
		return false
	}
	if i < n && text[i] == '.' {
		i++
		if digits() == 0 {
			return false
		}
	}
	if i < n && (text[i] == 'e' || text[i] == 'E') {
		i++
		if i < n && (text[i] == '+' || text[i] == '-') {
			i++
		}
		if digits() == 0 {
			return false
		}
	}
	return i == n
}

func putLen(out []byte, n uint64) []byte {
	for {
		b := byte(n & 0x7f)
		n >>= 7
		if n == 0 {
			return append(out, b)
		}
		out = append(out, b|0x80)
	}
}

func putText(out []byte, tag byte, text string) []byte {
	out = append(out, tag)
	out = putLen(out, uint64(len(text)))
	return append(out, text...)
}

// Encode is the wire bytes of value.
func Encode(value any) ([]byte, error) {
	return encode(nil, value, 0)
}

// The deepest tree Encode walks: deeper is refused, not a stack overflow.
const deepest = 512

func encode(out []byte, value any, depth int) ([]byte, error) {
	if depth > deepest {
		return nil, errors.New("a tree deeper than 512")
	}
	switch v := value.(type) {
	case AbsentValue:
		return append(out, tagAbsent), nil
	case nil:
		return append(out, tagNull), nil
	case bool:
		if v {
			return append(out, tagBool, 1), nil
		}
		return append(out, tagBool, 0), nil
	case Number:
		if !ValidNumber(string(v)) {
			return nil, fmt.Errorf("%q is not a number", string(v))
		}
		return putText(out, tagNumber, string(v)), nil
	case int:
		return putText(out, tagNumber, strconv.FormatInt(int64(v), 10)), nil
	case int8, int16, int32, int64:
		return putText(out, tagNumber, fmt.Sprint(v)), nil
	case uint, uint8, uint16, uint32, uint64:
		return putText(out, tagNumber, fmt.Sprint(v)), nil
	case float32:
		return encode(out, float64(v), depth)
	case float64:
		if math.IsNaN(v) || math.IsInf(v, 0) {
			return nil, errors.New("a float that is not finite has no value")
		}
		return putText(out, tagNumber, strconv.FormatFloat(v, 'g', -1, 64)), nil
	case string:
		if !utf8.ValidString(v) {
			return nil, errors.New("text that is not UTF-8")
		}
		return putText(out, tagString, v), nil
	case []byte:
		out = append(out, tagBytes)
		out = putLen(out, uint64(len(v)))
		return append(out, v...), nil
	case []any:
		out = append(out, tagList)
		out = putLen(out, uint64(len(v)))
		var err error
		for _, item := range v {
			if out, err = encode(out, item, depth+1); err != nil {
				return nil, err
			}
		}
		return out, nil
	case *Map:
		out = append(out, tagMap)
		out = putLen(out, uint64(len(v.keys)))
		var err error
		for _, key := range v.keys {
			if !utf8.ValidString(key) {
				return nil, errors.New("a key that is not UTF-8")
			}
			out = putLen(out, uint64(len(key)))
			out = append(out, key...)
			if out, err = encode(out, v.values[key], depth+1); err != nil {
				return nil, err
			}
		}
		return out, nil
	case map[string]any:
		keys := make([]string, 0, len(v))
		for k := range v {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		m := NewMap()
		for _, k := range keys {
			m.Set(k, v[k])
		}
		return encode(out, m, depth)
	default:
		return nil, fmt.Errorf("a %T has no wire form", value)
	}
}

type reader struct {
	bytes []byte
	at    int
}

func (r *reader) byte() (byte, error) {
	if r.at >= len(r.bytes) {
		return 0, &WireError{r.at, "the bytes end inside a value"}
	}
	b := r.bytes[r.at]
	r.at++
	return b, nil
}

// leb is an unsigned LEB128 of at most 64 bits, in its shortest form.
func (r *reader) leb() (uint64, error) {
	at := r.at
	var n uint64
	shift := uint(0)
	for {
		b, err := r.byte()
		if err != nil {
			return 0, err
		}
		bits := uint64(b & 0x7f)
		if (shift == 63 && bits > 1) || shift > 63 {
			return 0, &WireError{at, "a length past 64 bits"}
		}
		n |= bits << shift
		if b&0x80 == 0 {
			if b == 0 && shift > 0 {
				return 0, &WireError{at, "a length not in its shortest form"}
			}
			return n, nil
		}
		shift += 7
	}
}

// count is a length or a count no larger than the bytes that remain.
func (r *reader) count() (int, error) {
	n, err := r.leb()
	if err != nil {
		return 0, err
	}
	if n > uint64(len(r.bytes)-r.at) {
		return 0, &WireError{r.at, "the bytes end inside a value"}
	}
	return int(n), nil
}

func (r *reader) slice() ([]byte, error) {
	n, err := r.count()
	if err != nil {
		return nil, err
	}
	s := r.bytes[r.at : r.at+n]
	r.at += n
	return s, nil
}

func (r *reader) text() (string, error) {
	at := r.at
	s, err := r.slice()
	if err != nil {
		return "", err
	}
	if !utf8.Valid(s) {
		return "", &WireError{at, "text that is not UTF-8"}
	}
	return string(s), nil
}

// A container being read.
type frame struct {
	list  []any
	m     *Map
	left  int
	key   string
	keyAt int
}

// Decode is the value bytes hold. Every malformation is refused, naming
// the byte: an unknown tag, a body cut short, a length not in its
// shortest form or past 64 bits, text that is not UTF-8, a number
// outside the grammar, a bool byte other than 0 or 1, a key repeated,
// anything after the value.
func Decode(bytes []byte) (any, error) {
	r := &reader{bytes: bytes}
	var stack []*frame
	for {
		var value any
		if top := len(stack) - 1; top >= 0 && stack[top].left == 0 {
			done := stack[top]
			stack = stack[:top]
			if done.m != nil {
				value = done.m
			} else {
				value = done.list
			}
		} else {
			if top >= 0 && stack[top].m != nil {
				at := r.at
				key, err := r.text()
				if err != nil {
					return nil, err
				}
				stack[top].key, stack[top].keyAt = key, at
			}
			at := r.at
			tag, err := r.byte()
			if err != nil {
				return nil, err
			}
			switch tag {
			case tagAbsent:
				value = Absent
			case tagNull:
				value = nil
			case tagBool:
				bat := r.at
				b, err := r.byte()
				if err != nil {
					return nil, err
				}
				if b > 1 {
					return nil, &WireError{bat, "a bool byte other than 0 or 1"}
				}
				value = b == 1
			case tagNumber:
				body := r.at
				text, err := r.text()
				if err != nil {
					return nil, err
				}
				if !ValidNumber(text) {
					return nil, &WireError{body, "a number outside the grammar"}
				}
				value = Number(text)
			case tagString:
				text, err := r.text()
				if err != nil {
					return nil, err
				}
				value = text
			case tagBytes:
				s, err := r.slice()
				if err != nil {
					return nil, err
				}
				value = append([]byte{}, s...)
			case tagList, tagMap:
				n, err := r.count()
				if err != nil {
					return nil, err
				}
				f := &frame{left: n}
				if tag == tagMap {
					f.m = NewMap()
				} else {
					f.list = make([]any, 0, n)
				}
				stack = append(stack, f)
				continue
			default:
				return nil, &WireError{at, "an unknown tag"}
			}
		}
		top := len(stack) - 1
		if top < 0 {
			if r.at != len(r.bytes) {
				return nil, &WireError{r.at, "bytes after the value"}
			}
			return value, nil
		}
		f := stack[top]
		if f.m != nil {
			if _, dup := f.m.values[f.key]; dup {
				return nil, &WireError{f.keyAt, "a key repeated"}
			}
			f.m.Set(f.key, value)
		} else {
			f.list = append(f.list, value)
		}
		f.left--
	}
}
