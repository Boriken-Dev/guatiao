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
	"unicode/utf8"
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
	s := &treeSink{alloc: alloc, root: raw}
	e := encoder{out: s}
	if err := e.writeAny(x); err != nil {
		s.abandon()
		native.ValueFree(raw)
		native.Zero(raw, native.SizeValue)
		return err
	}
	return nil
}

// sink takes one value's events from the encoder, in order: a list's or
// map's count, then its items (each of a map's after its key), then end.
// Text and keys arrive as UTF-8 and numbers in the JSON number grammar:
// the encoder checks both. A number's text may be the encoder's scratch,
// valid for the call only.
type sink interface {
	null() error
	boolean(b bool) error
	number(text string) error
	text(s string) error
	bytes(b []byte) error
	// node writes the tree a Ref names, as it is.
	node(r Ref) error
	instant(t time.Time) error
	list(n int) error
	mapOf(n int) error
	key(k string) error
	end() error
}

// treeSink builds a tree in C memory: each item is built in its
// container's scratch node, then moved in.
type treeSink struct {
	alloc unsafe.Pointer
	root  unsafe.Pointer
	open  []treeFrame
}

type treeFrame struct {
	node, tmp unsafe.Pointer
	isMap     bool
	key       string
}

// target is the node the next item is written into.
func (s *treeSink) target() unsafe.Pointer {
	if len(s.open) == 0 {
		return s.root
	}
	return s.open[len(s.open)-1].tmp
}

// placed moves what target holds into the innermost open container.
func (s *treeSink) placed(st uint32) error {
	if err := check(st, ""); err != nil {
		return err
	}
	if len(s.open) == 0 {
		return nil
	}
	top := &s.open[len(s.open)-1]
	if top.isMap {
		st = native.MapSet(s.alloc, top.node, top.key, top.tmp)
	} else {
		st = native.ListPush(s.alloc, top.node, top.tmp)
	}
	native.ValueFree(top.tmp)
	return check(st, "")
}

func (s *treeSink) null() error          { return s.placed(native.ValueNull(s.target())) }
func (s *treeSink) boolean(b bool) error { return s.placed(native.ValueBool(b, s.target())) }
func (s *treeSink) number(text string) error {
	return s.placed(native.ValueNumber(s.alloc, text, s.target()))
}
func (s *treeSink) text(t string) error  { return s.placed(native.ValueString(s.alloc, t, s.target())) }
func (s *treeSink) bytes(b []byte) error { return s.placed(native.ValueBytes(s.alloc, b, s.target())) }

func (s *treeSink) node(r Ref) error {
	node, err := r.requireNode()
	if err != nil {
		return err
	}
	return s.placed(native.ValueClone(s.alloc, node, s.target()))
}

func (s *treeSink) instant(t time.Time) error {
	ts := native.Timestamp{Seconds: t.Unix(), Nanoseconds: uint32(t.Nanosecond())}
	return s.placed(native.TimestampFormat(s.alloc, ts, s.target()))
}

func (s *treeSink) list(int) error { return s.opened(native.ValueList(s.alloc, s.target()), false) }

func (s *treeSink) mapOf(int) error { return s.opened(native.ValueMap(s.alloc, s.target()), true) }

func (s *treeSink) opened(st uint32, isMap bool) error {
	if err := check(st, ""); err != nil {
		return err
	}
	s.open = append(s.open, treeFrame{node: s.target(), tmp: native.Calloc(native.SizeValue), isMap: isMap})
	return nil
}

func (s *treeSink) key(k string) error {
	s.open[len(s.open)-1].key = k
	return nil
}

func (s *treeSink) end() error {
	top := s.open[len(s.open)-1]
	s.open = s.open[:len(s.open)-1]
	native.Free(top.tmp)
	return s.placed(native.OK)
}

// abandon frees every item still in a scratch node, innermost first; the
// root, which holds what was moved in, is the caller's to free.
func (s *treeSink) abandon() {
	for i := len(s.open) - 1; i >= 0; i-- {
		native.ValueFree(s.open[i].tmp)
		native.Free(s.open[i].tmp)
	}
	s.open = nil
}

// pathStep is a key into a map, or an index into a list when index >= 0.
type pathStep struct {
	key   string
	index int
}

// spell is a path in the intake grammar's spelling: keys joined by dots,
// indices in brackets.
func spell(path []pathStep) string {
	at := ""
	for _, s := range path {
		if s.index >= 0 {
			at += "[" + strconv.Itoa(s.index) + "]"
		} else {
			at = joinKey(at, s.key)
		}
	}
	return at
}

// encoder walks Go data with reflection and hands its events to out. The
// path to the value being written is spelled only for an error.
type encoder struct {
	out     sink
	seen    map[uintptr]bool
	depth   int
	path    []pathStep
	scratch []byte
}

func (e *encoder) fail(format string, args ...any) error {
	return encodeError(spell(e.path), format, args...)
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

// typeFlags are the interfaces a type implements, looked up once.
type typeFlags struct {
	marshaler, textMarshaler bool
}

var typeFlagCache sync.Map // reflect.Type -> typeFlags

func flagsOf(t reflect.Type) typeFlags {
	if f, ok := typeFlagCache.Load(t); ok {
		return f.(typeFlags)
	}
	f := typeFlags{marshaler: t.Implements(typeMarshaler), textMarshaler: t.Implements(typeTextMarshaler)}
	typeFlagCache.Store(t, f)
	return f
}

// mark records a container about to be written, refusing one already open
// on the way here.
func (e *encoder) mark(ptr uintptr) error {
	if e.seen == nil {
		e.seen = map[uintptr]bool{}
	}
	if e.seen[ptr] {
		return e.fail("the value refers to itself")
	}
	e.seen[ptr] = true
	return nil
}

// writeAny writes x, taking the plain data a document holds (types with
// no methods) without reflection: what it does with each is what write
// does.
func (e *encoder) writeAny(x any) error {
	switch x.(type) {
	case string, bool, float64, int, []any, map[string]any:
	default:
		if x == nil {
			return e.out.null()
		}
		return e.write(reflect.ValueOf(x))
	}
	e.depth++
	defer func() { e.depth-- }()
	if e.depth > native.MaxDepth {
		return e.fail("the value nests deeper than %d", native.MaxDepth)
	}
	switch v := x.(type) {
	case string:
		return e.str(v)
	case bool:
		return e.out.boolean(v)
	case float64:
		return e.float(v, 64)
	case int:
		e.scratch = strconv.AppendInt(e.scratch[:0], int64(v), 10)
		return e.out.number(unsafe.String(unsafe.SliceData(e.scratch), len(e.scratch)))
	case []any:
		if len(v) > 0 {
			ptr := uintptr(unsafe.Pointer(unsafe.SliceData(v)))
			if err := e.mark(ptr); err != nil {
				return err
			}
			defer delete(e.seen, ptr)
		}
		if err := e.out.list(len(v)); err != nil {
			return err
		}
		e.path = append(e.path, pathStep{})
		for i, item := range v {
			e.path[len(e.path)-1].index = i
			if err := e.writeAny(item); err != nil {
				return err
			}
		}
		e.path = e.path[:len(e.path)-1]
		return e.out.end()
	}
	m := x.(map[string]any)
	if m == nil {
		if err := e.out.mapOf(0); err != nil {
			return err
		}
		return e.out.end()
	}
	ptr := uintptr(reflect.ValueOf(m).UnsafePointer())
	if err := e.mark(ptr); err != nil {
		return err
	}
	defer delete(e.seen, ptr)
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	// A Go map has no order; keys are written sorted.
	slices.Sort(keys)
	if err := e.out.mapOf(len(keys)); err != nil {
		return err
	}
	for _, k := range keys {
		if err := e.entry(k, func() error { return e.writeAny(m[k]) }); err != nil {
			return err
		}
	}
	return e.out.end()
}

func (e *encoder) write(v reflect.Value) error {
	if !v.IsValid() {
		return e.out.null()
	}
	e.depth++
	defer func() { e.depth-- }()
	if e.depth > native.MaxDepth {
		return e.fail("the value nests deeper than %d", native.MaxDepth)
	}
	t := v.Type()
	switch t {
	case typeValue, typeRef, typeMapRef, typeListRef:
		return e.node(v)
	case typeNumber:
		return e.number(v.String())
	case typeTime:
		return e.out.instant(v.Interface().(time.Time))
	case typeDuration:
		return e.length(time.Duration(v.Int()))
	case typeBigInt:
		b := v.Interface().(big.Int)
		return e.number(b.String())
	case typeOrderedMap:
		return e.ordered(v.Interface().(orderedMap))
	}
	kind := t.Kind()
	if kind == reflect.Pointer {
		if v.IsNil() {
			return e.out.null()
		}
		switch t.Elem() {
		case typeValue, typeBigInt:
			return e.write(v.Elem())
		}
	}
	if kind != reflect.Interface {
		flags := flagsOf(t)
		if flags.marshaler {
			x, err := v.Interface().(Marshaler).MarshalGuatiao()
			if err != nil {
				return e.fail("%v", err)
			}
			e.depth--
			defer func() { e.depth++ }()
			return e.writeAny(x)
		}
		if flags.textMarshaler {
			return e.text(v.Interface().(encoding.TextMarshaler))
		}
	}
	switch kind {
	case reflect.Bool:
		return e.out.boolean(v.Bool())
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		e.scratch = strconv.AppendInt(e.scratch[:0], v.Int(), 10)
		return e.out.number(unsafe.String(unsafe.SliceData(e.scratch), len(e.scratch)))
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		e.scratch = strconv.AppendUint(e.scratch[:0], v.Uint(), 10)
		return e.out.number(unsafe.String(unsafe.SliceData(e.scratch), len(e.scratch)))
	case reflect.Float32:
		return e.float(v.Float(), 32)
	case reflect.Float64:
		return e.float(v.Float(), 64)
	case reflect.String:
		return e.str(v.String())
	case reflect.Interface:
		if v.IsNil() {
			return e.out.null()
		}
		e.depth--
		defer func() { e.depth++ }()
		return e.writeAny(v.Elem().Interface())
	case reflect.Pointer:
		ptr := v.Pointer()
		if err := e.mark(ptr); err != nil {
			return err
		}
		defer delete(e.seen, ptr)
		e.depth--
		defer func() { e.depth++ }()
		return e.write(v.Elem())
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 && !flagsOf(t.Elem()).textMarshaler {
			return e.out.bytes(v.Bytes())
		}
		if !v.IsNil() && v.Len() > 0 {
			ptr := v.Pointer()
			if err := e.mark(ptr); err != nil {
				return err
			}
			defer delete(e.seen, ptr)
		}
		return e.list(v)
	case reflect.Array:
		return e.list(v)
	case reflect.Map:
		if v.IsNil() {
			if err := e.out.mapOf(0); err != nil {
				return err
			}
			return e.out.end()
		}
		ptr := v.Pointer()
		if err := e.mark(ptr); err != nil {
			return err
		}
		defer delete(e.seen, ptr)
		return e.mapOf(v)
	case reflect.Struct:
		return e.record(v)
	}
	return e.fail("a %s has no value form", t)
}

func (e *encoder) float(f float64, bits int) error {
	if math.IsInf(f, 0) || math.IsNaN(f) {
		return e.fail("%v is not a finite number", f)
	}
	if f == 0 && math.Signbit(f) {
		return e.out.number("-0")
	}
	e.scratch = strconv.AppendFloat(e.scratch[:0], f, 'g', -1, bits)
	return e.out.number(unsafe.String(unsafe.SliceData(e.scratch), len(e.scratch)))
}

func (e *encoder) node(v reflect.Value) error {
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
	return e.out.node(r)
}

func (e *encoder) number(text string) error {
	if !isNumber(text) {
		return e.fail("%q is not a number", text)
	}
	return e.out.number(text)
}

func (e *encoder) text(m encoding.TextMarshaler) error {
	text, err := m.MarshalText()
	if err != nil {
		return e.fail("%v", err)
	}
	return e.str(string(text))
}

func (e *encoder) str(text string) error {
	if !utf8.ValidString(text) {
		return e.fail("the text is not UTF-8")
	}
	return e.out.text(text)
}

// length writes d as an exact length, [seconds, nanoseconds].
func (e *encoder) length(d time.Duration) error {
	if d < 0 {
		return e.fail("an exact length is never negative, and %s is", d)
	}
	secs := uint64(d / time.Second)
	nanos := uint64(d % time.Second)
	e.depth--
	defer func() { e.depth++ }()
	return e.write(reflect.ValueOf([2]uint64{secs, nanos}))
}

func (e *encoder) list(v reflect.Value) error {
	n := v.Len()
	if err := e.out.list(n); err != nil {
		return err
	}
	e.path = append(e.path, pathStep{})
	for i := 0; i < n; i++ {
		e.path[len(e.path)-1].index = i
		if err := e.write(v.Index(i)); err != nil {
			return err
		}
	}
	e.path = e.path[:len(e.path)-1]
	return e.out.end()
}

// keyText is a map key's text: a string as it is, an integer in decimal,
// a TextMarshaler's text.
func keyText(k reflect.Value) (string, error) {
	if k.Kind() != reflect.String && flagsOf(k.Type()).textMarshaler {
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

func (e *encoder) mapOf(v reflect.Value) error {
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
			return e.fail("%v", err)
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
	for i := 1; i < len(entries); i++ {
		if entries[i].key == entries[i-1].key {
			return e.fail("two keys are both written %q", entries[i].key)
		}
	}
	if err := e.out.mapOf(len(entries)); err != nil {
		return err
	}
	for _, item := range entries {
		if err := e.entry(item.key, func() error { return e.write(item.val) }); err != nil {
			return err
		}
	}
	return e.out.end()
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

// entry writes one map entry: its key, checked, then what write writes
// under it.
func (e *encoder) entry(key string, write func() error) error {
	if !utf8.ValidString(key) {
		return e.fail("the key %q is not UTF-8", key)
	}
	if err := e.out.key(key); err != nil {
		return err
	}
	e.path = append(e.path, pathStep{key: key, index: -1})
	if err := write(); err != nil {
		return err
	}
	e.path = e.path[:len(e.path)-1]
	return nil
}

func (e *encoder) record(v reflect.Value) error {
	fields := fieldsOf(v.Type())
	n := 0
	for _, f := range fields {
		if fv, ok := fieldByIndex(v, f.index); ok && !omitted(fv, f.omitEmpty) {
			n++
		}
	}
	if err := e.out.mapOf(n); err != nil {
		return err
	}
	for _, f := range fields {
		fv, ok := fieldByIndex(v, f.index)
		if !ok || omitted(fv, f.omitEmpty) {
			continue
		}
		if err := e.entry(f.name, func() error { return e.write(fv) }); err != nil {
			return err
		}
	}
	return e.out.end()
}

// isNumber reports whether text is in the JSON number grammar (RFC 8259
// section 6), which every number's text keeps.
func isNumber(text string) bool {
	i, n := 0, len(text)
	if i < n && text[i] == '-' {
		i++
	}
	switch {
	case i < n && text[i] == '0':
		i++
	case i < n && text[i] >= '1' && text[i] <= '9':
		for i < n && text[i] >= '0' && text[i] <= '9' {
			i++
		}
	default:
		return false
	}
	if i < n && text[i] == '.' {
		i++
		start := i
		for i < n && text[i] >= '0' && text[i] <= '9' {
			i++
		}
		if i == start {
			return false
		}
	}
	if i < n && (text[i] == 'e' || text[i] == 'E') {
		i++
		if i < n && (text[i] == '+' || text[i] == '-') {
			i++
		}
		start := i
		for i < n && text[i] >= '0' && text[i] <= '9' {
			i++
		}
		if i == start {
			return false
		}
	}
	return i == n
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
