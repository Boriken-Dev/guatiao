// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"iter"
	"math"
	"runtime"
	"strconv"
	"strings"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Tag is the kind of a stored value.
type Tag uint32

// The tags, as the C surface numbers them. A tag this package does not
// know reads as what it is: a number outside this list.
const (
	TagAbsent Tag = Tag(native.TagAbsent)
	TagNull   Tag = Tag(native.TagNull)
	TagBool   Tag = Tag(native.TagBool)
	TagNumber Tag = Tag(native.TagNumber)
	TagString Tag = Tag(native.TagString)
	TagBytes  Tag = Tag(native.TagBytes)
	TagList   Tag = Tag(native.TagList)
	TagMap    Tag = Tag(native.TagMap)
)

var tagNames = [...]string{"absent", "null", "bool", "number", "string", "bytes", "list", "map"}

func (t Tag) String() string {
	if int(t) < len(tagNames) {
		return tagNames[t]
	}
	return "tag " + strconv.FormatUint(uint64(t), 10)
}

// Number is a number as the exact text that wrote it: "1.10" stays
// "1.10" and 18446744073709551615 survives. Decoding a number into any
// gives one.
type Number string

func (n Number) String() string { return string(n) }

// Int64 is the number as a signed 64-bit integer: an error for a
// fraction, an exponent spelling or a value out of range, never a
// truncation.
func (n Number) Int64() (int64, error) { return strconv.ParseInt(string(n), 10, 64) }

// Uint64 is the number as an unsigned 64-bit integer, on Int64's terms.
func (n Number) Uint64() (uint64, error) { return strconv.ParseUint(string(n), 10, 64) }

// Float64 is the nearest float64; an error when there is no finite one.
func (n Number) Float64() (float64, error) {
	f, err := strconv.ParseFloat(string(n), 64)
	if err == nil && (math.IsInf(f, 0) || math.IsNaN(f)) {
		return 0, &strconv.NumError{Func: "ParseFloat", Num: string(n), Err: strconv.ErrRange}
	}
	return f, err
}

// ---- owners ---------------------------------------------------------------

// owner is what a Ref walks from: a root node, and whether and where it
// may be written.
type owner interface {
	// root is the root node's address now, or an error when it is gone.
	root() (unsafe.Pointer, error)
	// buildAlloc is the allocator a write builds in, or ErrReadOnly.
	buildAlloc() (unsafe.Pointer, error)
	// cloneAlloc is the allocator a clone is made in.
	cloneAlloc() (unsafe.Pointer, error)
}

// owned is a root node this package allocated and frees.
type owned struct {
	node    unsafe.Pointer
	alloc   unsafe.Pointer // nil: the core library's own
	closed  bool
	cleanup runtime.Cleanup
}

func (o *owned) root() (unsafe.Pointer, error) {
	if o.closed {
		return nil, ErrClosed
	}
	return o.node, nil
}

func (o *owned) buildAlloc() (unsafe.Pointer, error) {
	if o.closed {
		return nil, ErrClosed
	}
	return o.allocator()
}

func (o *owned) cloneAlloc() (unsafe.Pointer, error) { return o.allocator() }

func (o *owned) allocator() (unsafe.Pointer, error) {
	if o.alloc != nil {
		return o.alloc, nil
	}
	return coreAlloc()
}

// coreAlloc is the core library's own allocator, opening it first.
func coreAlloc() (unsafe.Pointer, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	return lib.Alloc(), nil
}

// borrowed is memory this package does not free: find answers the node
// again on every access, and keep, when set, is checked open first.
type borrowed struct {
	find func() unsafe.Pointer
	keep owner
	// alloc is set for a tree lent writable, and names where writes build.
	alloc unsafe.Pointer
	// open, when set, says whether the call that lent it is still running.
	open func() bool
}

func (b *borrowed) root() (unsafe.Pointer, error) {
	if b.open != nil && !b.open() {
		return nil, ErrClosed
	}
	if b.keep != nil {
		if _, err := b.keep.root(); err != nil {
			return nil, err
		}
	}
	node := b.find()
	if node == nil {
		return nil, ErrClosed
	}
	return node, nil
}

func (b *borrowed) buildAlloc() (unsafe.Pointer, error) {
	if _, err := b.root(); err != nil {
		return nil, err
	}
	if b.alloc == nil {
		return nil, ErrReadOnly
	}
	return b.alloc, nil
}

func (b *borrowed) cloneAlloc() (unsafe.Pointer, error) { return coreAlloc() }

// ---- Ref ------------------------------------------------------------------

type step struct {
	key   string
	index int
	isKey bool
}

// Ref is a node inside a tree: what it belongs to, and a path of keys and
// indices walked again on every access, because a write beside a node can
// move the array it sits in. The zero Ref names nothing and reads as
// absent. A Ref is a value: copy it freely; it is valid while what it
// belongs to is.
type Ref struct {
	o    owner
	path []step
}

func (r Ref) with(s step) Ref {
	path := make([]step, len(r.path), len(r.path)+1)
	copy(path, r.path)
	return Ref{o: r.o, path: append(path, s)}
}

// Key is the node under key, if this node is a map that has one; a Ref
// naming nothing reads as absent.
func (r Ref) Key(key string) Ref { return r.with(step{key: key, isKey: true}) }

// Index is the element at i (negative counts from the end), if this node
// is a list that long.
func (r Ref) Index(i int) Ref { return r.with(step{index: i}) }

// node is the node's address now: nil (and no error) when the path names
// nothing, an error when what it belongs to is gone.
func (r Ref) node() (unsafe.Pointer, error) {
	if r.o == nil {
		return nil, nil
	}
	node, err := r.o.root()
	if err != nil {
		return nil, err
	}
	for _, s := range r.path {
		if node == nil {
			return nil, nil
		}
		if s.isKey {
			node = native.Find(node, s.key)
			continue
		}
		if native.Tag(node) != native.TagList {
			return nil, nil
		}
		n := int(native.Len(node))
		i := s.index
		if i < 0 {
			i += n
		}
		if i < 0 || i >= n {
			return nil, nil
		}
		node = native.ListElem(node, uintptr(i))
	}
	return node, nil
}

// requireNode is the node, or an error naming the path.
func (r Ref) requireNode() (unsafe.Pointer, error) {
	node, err := r.node()
	if err != nil {
		return nil, err
	}
	if node == nil {
		return nil, &Error{Status: NotFound, Message: "no value at " + r.Path()}
	}
	return node, nil
}

// Path is the path this Ref walks, in the intake path grammar
// (`agent[1].name`); empty for a root.
func (r Ref) Path() string {
	var b strings.Builder
	for _, s := range r.path {
		if !s.isKey {
			b.WriteString("[" + strconv.Itoa(s.index) + "]")
			continue
		}
		if plainKey(s.key) {
			if b.Len() > 0 {
				b.WriteByte('.')
			}
			b.WriteString(s.key)
		} else {
			b.WriteString("[" + strconv.Quote(s.key) + "]")
		}
	}
	return b.String()
}

func plainKey(k string) bool {
	if k == "" {
		return false
	}
	for _, c := range k {
		if !(c == '_' || c == '-' || c >= '0' && c <= '9' || c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z') {
			return false
		}
	}
	return true
}

// Tag is the node's tag: TagAbsent when the path names nothing or what
// it belongs to is gone.
func (r Ref) Tag() Tag {
	node, _ := r.node()
	return Tag(native.Tag(node))
}

// IsAbsent reports whether the node is absent.
func (r Ref) IsAbsent() bool { return r.Tag() == TagAbsent }

// IsNull reports whether the node is null.
func (r Ref) IsNull() bool { return r.Tag() == TagNull }

// BoolOr is the boolean, or fallback for anything else.
func (r Ref) BoolOr(fallback bool) bool {
	node, _ := r.node()
	if native.Tag(node) != native.TagBool {
		return fallback
	}
	return native.BoolOf(node)
}

// IntOr is the number as a signed 64-bit integer, or fallback: a wrong
// kind, a fraction or an exponent spelling, or a value out of range all
// fall back, never truncate.
func (r Ref) IntOr(fallback int64) int64 {
	text, ok := r.NumberText()
	if !ok {
		return fallback
	}
	i, err := text.Int64()
	if err != nil {
		return fallback
	}
	return i
}

// FloatOr is the number as the nearest float64, or fallback.
func (r Ref) FloatOr(fallback float64) float64 {
	text, ok := r.NumberText()
	if !ok {
		return fallback
	}
	f, err := text.Float64()
	if err != nil {
		return fallback
	}
	return f
}

// StringOr is the text, or fallback for anything else.
func (r Ref) StringOr(fallback string) string {
	if text, ok := r.Text(); ok {
		return text
	}
	return fallback
}

// Text is the text of a string node.
func (r Ref) Text() (string, bool) {
	node, _ := r.node()
	if native.Tag(node) != native.TagString {
		return "", false
	}
	return native.TextOf(node), true
}

// NumberText is a number node's exact text.
func (r Ref) NumberText() (Number, bool) {
	node, _ := r.node()
	if native.Tag(node) != native.TagNumber {
		return "", false
	}
	return Number(native.TextOf(node)), true
}

// Bytes is a copy of a bytes node's content.
func (r Ref) Bytes() ([]byte, bool) {
	node, _ := r.node()
	if native.Tag(node) != native.TagBytes {
		return nil, false
	}
	at := native.Payload(node)
	return native.Bytes(native.OwnedPtr(at), native.OwnedLen(at)), true
}

// Any is the whole node as plain Go data: nil for absent and null, bool,
// Number, string, []byte, []any and map[string]any.
func (r Ref) Any() (any, error) {
	var out any
	err := r.Decode(&out)
	return out, err
}

// Decode reads the node into what dst points at, as encoding/json's
// Unmarshal does: see the package documentation for the Go types it
// fills. An absent node leaves dst as it was.
func (r Ref) Decode(dst any) error {
	node, err := r.node()
	if err != nil {
		return err
	}
	return decodeInto(node, dst, r.Path())
}

// Map is the node as a map.
func (r Ref) Map() (MapRef, error) {
	node, err := r.requireNode()
	if err != nil {
		return MapRef{}, err
	}
	if native.Tag(node) != native.TagMap {
		return MapRef{}, &Error{Status: WrongKind, Message: "the value at " + r.where() + " is a " + Tag(native.Tag(node)).String() + ", not a map"}
	}
	return MapRef{r}, nil
}

// List is the node as a list.
func (r Ref) List() (ListRef, error) {
	node, err := r.requireNode()
	if err != nil {
		return ListRef{}, err
	}
	if native.Tag(node) != native.TagList {
		return ListRef{}, &Error{Status: WrongKind, Message: "the value at " + r.where() + " is a " + Tag(native.Tag(node)).String() + ", not a list"}
	}
	return ListRef{r}, nil
}

func (r Ref) where() string {
	if p := r.Path(); p != "" {
		return p
	}
	return "the root"
}

// Clone is a deep, independent copy of the node, as a new Value to close.
func (r Ref) Clone() (*Value, error) {
	node, err := r.requireNode()
	if err != nil {
		return nil, err
	}
	alloc, err := r.o.cloneAlloc()
	if err != nil {
		return nil, err
	}
	out := native.Calloc(native.SizeValue)
	if err := check(native.ValueClone(alloc, node, out), ""); err != nil {
		native.Free(out)
		return nil, err
	}
	return adopt(out, alloc), nil
}

// writable is the node and the allocator a write builds in.
func (r Ref) writable() (unsafe.Pointer, unsafe.Pointer, error) {
	if r.o == nil {
		return nil, nil, ErrReadOnly
	}
	alloc, err := r.o.buildAlloc()
	if err != nil {
		return nil, nil, err
	}
	node, err := r.requireNode()
	if err != nil {
		return nil, nil, err
	}
	return node, alloc, nil
}

// ---- MapRef ---------------------------------------------------------------

// MapRef is a Ref whose node is a map. Keys compare as bytes and keep
// insertion order; setting a key it has replaces the value in place.
type MapRef struct{ Ref }

func (m MapRef) mapNode() unsafe.Pointer {
	node, _ := m.node()
	if native.Tag(node) != native.TagMap {
		return nil
	}
	return node
}

// Len is the number of entries.
func (m MapRef) Len() int {
	node := m.mapNode()
	if node == nil {
		return 0
	}
	return int(native.Len(node))
}

// Has reports whether key is there.
func (m MapRef) Has(key string) bool { return native.Find(m.mapNode(), key) != nil }

// Get is the entry under key.
func (m MapRef) Get(key string) (Ref, bool) {
	if !m.Has(key) {
		return Ref{}, false
	}
	return m.Key(key), true
}

// Keys are the keys, in insertion order, as the map is now.
func (m MapRef) Keys() []string {
	node := m.mapNode()
	if node == nil {
		return nil
	}
	n := native.Len(node)
	keys := make([]string, 0, n)
	for i := uintptr(0); i < n; i++ {
		keys = append(keys, native.EntryKey(native.EntryAt(node, i)))
	}
	return keys
}

// All iterates the entries in insertion order, as the map was when the
// iteration began.
func (m MapRef) All() iter.Seq2[string, Ref] {
	return func(yield func(string, Ref) bool) {
		for _, k := range m.Keys() {
			if !yield(k, m.Key(k)) {
				return
			}
		}
	}
}

// Set writes x (anything NewValue takes) under key, replacing what was
// there. A failed conversion leaves the map as it was.
func (m MapRef) Set(key string, x any) error {
	node, alloc, err := m.writable()
	if err != nil {
		return err
	}
	return withScratch(func(tmp unsafe.Pointer) error {
		if err := writeGo(tmp, x, alloc); err != nil {
			return err
		}
		defer native.ValueFree(tmp)
		return check(native.MapSet(alloc, node, key, tmp), "")
	})
}

// Delete removes and frees the entry under key, and reports whether there
// was one.
func (m MapRef) Delete(key string) (bool, error) {
	node, _, err := m.writable()
	if err != nil {
		return false, err
	}
	st := native.MapDiscard(node, key)
	if st == native.ErrNotFound {
		return false, nil
	}
	return true, check(st, "")
}

// Clear removes and frees every entry.
func (m MapRef) Clear() error {
	node, _, err := m.writable()
	if err != nil {
		return err
	}
	return check(native.MapClear(node), "")
}

// CopyFrom copies every entry of src (a map), replacing collisions in
// place.
func (m MapRef) CopyFrom(src Ref) error {
	node, alloc, err := m.writable()
	if err != nil {
		return err
	}
	from, err := src.requireNode()
	if err != nil {
		return err
	}
	return check(native.MapCopyFrom(alloc, node, from), "")
}

// ---- ListRef --------------------------------------------------------------

// ListRef is a Ref whose node is a list. The C surface appends, removes by
// index and clears; Set and Insert are built from those (every element
// copied, the list cleared, the new order pushed back) and are O(n).
type ListRef struct{ Ref }

// Len is the number of elements.
func (l ListRef) Len() int {
	node, _ := l.node()
	if native.Tag(node) != native.TagList {
		return 0
	}
	return int(native.Len(node))
}

// At is the element at i (negative counts from the end).
func (l ListRef) At(i int) (Ref, bool) {
	n := l.Len()
	if i < 0 {
		i += n
	}
	if i < 0 || i >= n {
		return Ref{}, false
	}
	return l.Index(i), true
}

// All iterates the elements, as the list was when the iteration began.
func (l ListRef) All() iter.Seq2[int, Ref] {
	return func(yield func(int, Ref) bool) {
		n := l.Len()
		for i := 0; i < n; i++ {
			if !yield(i, l.Index(i)) {
				return
			}
		}
	}
}

// Push appends each value. A failed conversion appends nothing more and
// leaves what was pushed before it.
func (l ListRef) Push(xs ...any) error {
	node, alloc, err := l.writable()
	if err != nil {
		return err
	}
	return withScratch(func(tmp unsafe.Pointer) error {
		for _, x := range xs {
			if err := writeGo(tmp, x, alloc); err != nil {
				return err
			}
			st := native.ListPush(alloc, node, tmp)
			native.ValueFree(tmp)
			if err := check(st, ""); err != nil {
				return err
			}
		}
		return nil
	})
}

func (l ListRef) index(i int) (int, error) {
	n := l.Len()
	j := i
	if j < 0 {
		j += n
	}
	if j < 0 || j >= n {
		return 0, &Error{Status: NotFound, Message: "index " + strconv.Itoa(i) + " is outside a list of " + strconv.Itoa(n)}
	}
	return j, nil
}

// RemoveAt removes and frees the element at i.
func (l ListRef) RemoveAt(i int) error {
	node, _, err := l.writable()
	if err != nil {
		return err
	}
	j, err := l.index(i)
	if err != nil {
		return err
	}
	return check(native.ListDiscard(node, uintptr(j)), "")
}

// Clear removes and frees every element.
func (l ListRef) Clear() error {
	node, _, err := l.writable()
	if err != nil {
		return err
	}
	return check(native.ListClear(node), "")
}

// Set replaces the element at i. O(n).
func (l ListRef) Set(i int, x any) error {
	j, err := l.index(i)
	if err != nil {
		return err
	}
	return l.rebuild(x, func(elems []unsafe.Pointer, built unsafe.Pointer) []unsafe.Pointer {
		native.ValueFree(elems[j])
		native.Free(elems[j])
		elems[j] = built
		return elems
	})
}

// Insert puts x before index i, clamped to the list. O(n).
func (l ListRef) Insert(i int, x any) error {
	return l.rebuild(x, func(elems []unsafe.Pointer, built unsafe.Pointer) []unsafe.Pointer {
		n := len(elems)
		j := i
		if j < 0 {
			j = max(0, n+j)
		}
		j = min(j, n)
		elems = append(elems, nil)
		copy(elems[j+1:], elems[j:])
		elems[j] = built
		return elems
	})
}

func (l ListRef) rebuild(x any, change func([]unsafe.Pointer, unsafe.Pointer) []unsafe.Pointer) error {
	node, alloc, err := l.writable()
	if err != nil {
		return err
	}
	if native.Tag(node) != native.TagList {
		return &Error{Status: WrongKind, Message: "the value is not a list"}
	}
	built := native.Calloc(native.SizeValue)
	if err := writeGo(built, x, alloc); err != nil {
		native.Free(built)
		return err
	}
	var elems []unsafe.Pointer
	defer func() {
		for _, e := range elems {
			native.ValueFree(e)
			native.Free(e)
		}
	}()
	n := native.Len(node)
	for i := uintptr(0); i < n; i++ {
		c := native.Calloc(native.SizeValue)
		elems = append(elems, c)
		if err := check(native.ValueClone(alloc, native.ListElem(node, i), c), ""); err != nil {
			native.ValueFree(built)
			native.Free(built)
			return err
		}
	}
	elems = change(elems, built)
	if err := check(native.ListClear(node), ""); err != nil {
		return err
	}
	for _, e := range elems {
		if err := check(native.ListPush(alloc, node, e), ""); err != nil {
			return err
		}
	}
	return nil
}

// ---- Value ----------------------------------------------------------------

// Value owns a tree's root node. Close frees it; a Value dropped unclosed
// is freed when the garbage collector finds it and every Ref into it
// unreachable, at a moment nobody chooses, so close what you build. The
// methods of Ref read and change the root. A Value is not safe for
// concurrent use; reading it from several goroutines at once is.
type Value struct {
	Ref
	o *owned
}

// Option says how a Value is built.
type Option func(*options)

type options struct {
	alloc unsafe.Pointer
}

// InAllocator builds in the guatiao_alloc at alloc, which the caller
// keeps alive for as long as every tree built through it.
func InAllocator(alloc unsafe.Pointer) Option {
	return func(o *options) { o.alloc = alloc }
}

func newOwned(alloc unsafe.Pointer) *Value {
	o := &owned{node: native.Calloc(native.SizeValue), alloc: alloc}
	o.cleanup = runtime.AddCleanup(o, freeRoot, o.node)
	return &Value{Ref: Ref{o: o}, o: o}
}

func freeRoot(node unsafe.Pointer) {
	native.ValueFree(node)
	native.Free(node)
}

// adopt is a Value taking over the root node at node (from Calloc, which
// it now frees), built in alloc.
func adopt(node, alloc unsafe.Pointer) *Value {
	o := &owned{node: node, alloc: alloc}
	o.cleanup = runtime.AddCleanup(o, freeRoot, o.node)
	return &Value{Ref: Ref{o: o}, o: o}
}

// NewValue converts Go data to a tree: nil to null, bool, the integer and
// float types and Number to a number (its exact text; a float that is not
// finite is an error), string, []byte to bytes, slices and arrays to
// lists, maps and structs to maps, time.Time to an instant, time.Duration
// to an exact length, a TextMarshaler to its text, and a *Value, Ref,
// MapRef or ListRef to a deep copy. See the package documentation for the
// rules. A failed conversion frees what it built.
func NewValue(x any, opts ...Option) (*Value, error) {
	if _, err := core(); err != nil {
		return nil, err
	}
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	v := newOwned(o.alloc)
	alloc, err := v.o.allocator()
	if err == nil {
		err = writeGo(v.o.node, x, alloc)
	}
	if err != nil {
		v.Close()
		return nil, err
	}
	return v, nil
}

// Absent is an absent value.
func Absent() *Value { return newOwned(nil) }

// Null is a null value.
func Null() (*Value, error) { return NewValue(nil) }

// NewMap is an empty map.
func NewMap(opts ...Option) (*Value, error) { return NewValue(map[string]any{}, opts...) }

// NewList is an empty list.
func NewList(opts ...Option) (*Value, error) { return NewValue([]any{}, opts...) }

// Close frees the tree; a second call does nothing. It always answers nil.
func (v *Value) Close() error {
	if v == nil || v.o == nil || v.o.closed {
		return nil
	}
	v.o.closed = true
	v.o.cleanup.Stop()
	freeRoot(v.o.node)
	return nil
}

// Closed reports whether Close ran.
func (v *Value) Closed() bool { return v == nil || v.o == nil || v.o.closed }

// takeRoot moves the tree out into a fresh node the caller owns, leaving
// this Value absent.
func (v *Value) takeRoot() (unsafe.Pointer, error) {
	if v.Closed() {
		return nil, ErrClosed
	}
	moved := native.Calloc(native.SizeValue)
	native.Copy(moved, v.o.node, native.SizeValue)
	native.Zero(v.o.node, native.SizeValue)
	return moved, nil
}

// withScratch runs body with a zeroed C node, freed after.
func withScratch(body func(tmp unsafe.Pointer) error) error {
	tmp := native.Calloc(native.SizeValue)
	defer native.Free(tmp)
	return body(tmp)
}

// withNodes runs body with each item as a node: a Ref (or Value) as it is,
// plain data built for the call and freed after.
func withNodes(items []any, body func(nodes []unsafe.Pointer) error) error {
	nodes := make([]unsafe.Pointer, len(items))
	var built []*Value
	defer func() {
		for _, v := range built {
			v.Close()
		}
	}()
	for i, item := range items {
		if r, ok := asRef(item); ok {
			node, err := r.requireNode()
			if err != nil {
				return err
			}
			nodes[i] = node
			continue
		}
		v, err := NewValue(item)
		if err != nil {
			return err
		}
		built = append(built, v)
		nodes[i] = v.o.node
	}
	return body(nodes)
}

// asRef is item as a Ref when it is one of this package's views.
func asRef(item any) (Ref, bool) {
	switch x := item.(type) {
	case *Value:
		if x == nil {
			return Ref{}, false
		}
		return x.Ref, true
	case Ref:
		return x, true
	case *Ref:
		if x == nil {
			return Ref{}, false
		}
		return *x, true
	case MapRef:
		return x.Ref, true
	case ListRef:
		return x.Ref, true
	}
	return Ref{}, false
}

// ValueFromRaw takes over the root node at root, a guatiao_value in
// memory from C's malloc holding a tree, as a Value whose Close frees the
// tree through the allocators it records and then the node: for a C
// function that answers through an out parameter. Writes build in the
// core library's allocator.
func ValueFromRaw(root unsafe.Pointer) *Value { return adopt(root, nil) }

// Raw is the node's address now, for a C function that takes a
// guatiao_value: valid until the tree is next written or closed.
func (r Ref) Raw() (unsafe.Pointer, error) { return r.requireNode() }
