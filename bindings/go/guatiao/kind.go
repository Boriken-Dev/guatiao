// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"reflect"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// TreeClass is how a tree crosses a slot: as a value, a map or a list.
// Code guatiao-kind generates names it; nothing else needs to.
type TreeClass uint8

// The classes.
const (
	ValueClass TreeClass = iota
	MapClass
	ListClass
)

func (c TreeClass) tag() uint32 {
	switch c {
	case MapClass:
		return native.TagMap
	case ListClass:
		return native.TagList
	}
	return native.TagAbsent
}

func (c TreeClass) String() string {
	return [...]string{"value", "map", "list"}[c]
}

// KindSpec is a kind's table as guatiao-kind reads it from the C header:
// its name, its floor hash, whether it is an object kind (destroy first),
// and its slots in the header's order. Generated code writes one; nothing
// else needs to.
type KindSpec struct {
	Name      string
	FloorHash uint32
	Object    bool
	Slots     []SlotSpec
}

// SlotSpec is one slot of a KindSpec: its name as the header spells it,
// whether every table has it and whether it can refuse, the C function
// serving it, and the interface whose method it is.
type SlotSpec struct {
	Name     string
	Required bool
	Fallible bool
	Serve    unsafe.Pointer
	Iface    reflect.Type
}

type kindDesc struct {
	spec   KindSpec
	first  uintptr // offset of the first slot after the header (and destroy)
	size   uintptr // the table's size
	floor  uintptr // the smallest table: up to the last required slot
	tables sync.Map
}

func newKindDesc(spec KindSpec) *kindDesc {
	d := &kindDesc{spec: spec, first: native.SizeKindHeader}
	if spec.Object {
		d.first += native.Word
	}
	d.floor = d.first
	for i, s := range spec.Slots {
		if s.Required {
			d.floor = d.first + uintptr(i+1)*native.Word
		}
	}
	d.size = d.first + uintptr(len(spec.Slots))*native.Word
	return d
}

func (d *kindDesc) offset(slot int) uintptr { return d.first + uintptr(slot)*native.Word }

func (d *kindDesc) slotIndex(name string) int {
	for i, s := range d.spec.Slots {
		if s.Name == name {
			return i
		}
	}
	return -1
}

// AnyKind is any *Kind[R]: what a provider names among its options, and
// what generated code passes for an object kind.
type AnyKind interface {
	desc() *kindDesc
	applyProvider(*providerSpec)
}

// Kind is a kind's descriptor, as guatiao-kind generates it: R is the
// generated Remote type, which implements the kind's interfaces.
type Kind[R any] struct {
	d    *kindDesc
	wrap func(Remote) R
}

// NewKind is the descriptor of spec, whose remotes wrap answers. Code
// guatiao-kind generates calls it; nothing else needs to.
func NewKind[R any](spec KindSpec, wrap func(Remote) R) *Kind[R] {
	return &Kind[R]{d: newKindDesc(spec), wrap: wrap}
}

func (k *Kind[R]) desc() *kindDesc { return k.d }

// Name is the kind's name, as a provider's kinds list it.
func (k *Kind[R]) Name() string { return k.d.spec.Name }

// FloorHash is the header's <table>_FLOOR_HASH.
func (k *Kind[R]) FloorHash() uint32 { return k.d.spec.FloorHash }

// TableSize is the size of this kind's whole table.
func (k *Kind[R]) TableSize() uintptr { return k.d.size }

// Bind is the provider under key as this kind: a remote calling its table
// with its own instance. ErrNotFound when it serves no table for the
// kind, a *FloorMismatch when its table is not the one this kind
// describes. Valid while the library is: until it is unloaded or, once
// retired, collected.
func (k *Kind[R]) Bind(reg *Registry, key string) (R, error) {
	var zero R
	t, err := reg.ProviderTable(key, k.d.spec.Name)
	if err != nil {
		return zero, err
	}
	return k.BindTable(t.Pointer, t.Size, t.Ctx)
}

// BindInstance is an instance a registry built, as this kind. Valid until
// the instance is closed.
func (k *Kind[R]) BindInstance(i *Instance) (R, error) {
	var zero R
	if i == nil || i.closed.Load() {
		return zero, ErrClosed
	}
	t, err := i.reg.ProviderTable(i.key, k.d.spec.Name)
	if err != nil {
		return zero, err
	}
	return k.BindTable(t.Pointer, t.Size, i.ctx)
}

// BindTable is any table of this kind, over ctx: the primitive the other
// binds go through.
func (k *Kind[R]) BindTable(table unsafe.Pointer, size, ctx uintptr) (R, error) {
	var zero R
	r, err := newRemote(k.d, table, size, ctx, false, nil)
	if err != nil {
		return zero, err
	}
	return k.wrap(r), nil
}

// ---- Remote -----------------------------------------------------------------

// Remote is a caller's handle on a table: what each generated remote
// embeds. For an object this side owns, Close destroys it once; for a
// provider's table, or an object lent for a call, it does nothing.
type Remote struct{ r *remoteState }

type remoteState struct {
	kind    *kindDesc
	table   unsafe.Pointer
	size    uintptr
	ctx     uintptr
	owned   bool
	closed  atomic.Bool
	open    func() bool
	cleanup runtime.Cleanup
}

type rawObject struct {
	table unsafe.Pointer
	size  uintptr
	ctx   uintptr
}

func (o rawObject) destroy(kind *kindDesc) {
	if o.table == nil || o.size < native.SizeKindHeader+native.Word {
		return
	}
	if fn := native.SlotAt(o.table, native.SizeKindHeader); fn != nil {
		native.CallDestroy(fn, o.ctx)
	}
}

func newRemote(d *kindDesc, table unsafe.Pointer, size, ctx uintptr, owned bool, open func() bool) (Remote, error) {
	if table == nil {
		return Remote{}, &FloorMismatch{Message: "no " + d.spec.Name + " table"}
	}
	if size < d.floor {
		return Remote{}, &FloorMismatch{Message: fmt.Sprintf("the %s table is %d bytes, its required slots need %d", d.spec.Name, size, d.floor)}
	}
	if _, hash := native.TableHeader(table); hash != d.spec.FloorHash {
		return Remote{}, &FloorMismatch{Message: fmt.Sprintf("the table's floor hash %d is not the %s kind's %d", hash, d.spec.Name, d.spec.FloorHash)}
	}
	rs := &remoteState{kind: d, table: table, size: size, ctx: ctx, owned: owned, open: open}
	if owned {
		raw := rawObject{table: table, size: size, ctx: ctx}
		rs.cleanup = runtime.AddCleanup(rs, func(o rawObject) { o.destroy(d) }, raw)
	}
	return Remote{r: rs}, nil
}

func (r Remote) check() error {
	if r.r == nil {
		return &Error{Status: NullArgument, Message: "no table"}
	}
	if r.r.closed.Load() {
		return ErrClosed
	}
	if r.r.open != nil && !r.r.open() {
		return ErrClosed
	}
	return nil
}

func (r Remote) slotFn(i int) unsafe.Pointer {
	off := r.r.kind.offset(i)
	if off+native.Word > r.r.size {
		return nil
	}
	return native.SlotAt(r.r.table, off)
}

// Has reports whether the table carries the slot the header calls name.
func (r Remote) Has(name string) bool {
	if r.check() != nil {
		return false
	}
	i := r.r.kind.slotIndex(name)
	return i >= 0 && r.slotFn(i) != nil
}

// Kind is the kind's name.
func (r Remote) Kind() string {
	if r.r == nil {
		return ""
	}
	return r.r.kind.spec.Name
}

// Table is the table, its size and the ctx its slots take.
func (r Remote) Table() Table {
	if r.r == nil {
		return Table{}
	}
	return Table{Pointer: r.r.table, Size: r.r.size, Ctx: r.r.ctx}
}

// Close destroys an object this side owns, once; for anything else it
// does nothing. It answers nil.
func (r Remote) Close() error {
	if r.r == nil || !r.r.owned || !r.r.closed.CompareAndSwap(false, true) {
		return nil
	}
	r.r.cleanup.Stop()
	rawObject{table: r.r.table, size: r.r.size, ctx: r.r.ctx}.destroy(r.r.kind)
	return nil
}

// Closed reports whether the object was closed or handed over.
func (r Remote) Closed() bool { return r.r == nil || r.r.closed.Load() }

func (r Remote) guatiaoRemote() Remote { return r }

// remoteCarrier is anything embedding a Remote: every generated remote.
type remoteCarrier interface{ guatiaoRemote() Remote }

// giveUp hands an owned object over: from now on it is someone else's.
func (r Remote) giveUp() (rawObject, error) {
	if err := r.check(); err != nil {
		return rawObject{}, err
	}
	if !r.r.owned {
		return rawObject{}, &Error{Status: WrongKind, Message: "only an object this side owns can be handed over; this one is lent or a provider's"}
	}
	if !r.r.closed.CompareAndSwap(false, true) {
		return rawObject{}, ErrClosed
	}
	r.r.cleanup.Stop()
	return rawObject{table: r.r.table, size: r.r.size, ctx: r.r.ctx}, nil
}

// Table is a provider's table for one kind: its address, its size and
// the ctx its slots take. Pointer is nil when there is none.
type Table struct {
	Pointer unsafe.Pointer
	Size    uintptr
	Ctx     uintptr
}

// ---- serving a table over Go values -------------------------------------------

// tableFor is the table serving d through what t implements, built on
// first use and kept for the process: each slot t has, nil for the rest.
func (d *kindDesc) tableFor(t reflect.Type) (unsafe.Pointer, error) {
	if found, ok := d.tables.Load(t); ok {
		return found.(unsafe.Pointer), nil
	}
	var missing []string
	slots := make([]unsafe.Pointer, 0, len(d.spec.Slots)+1)
	if d.spec.Object {
		slots = append(slots, native.ObjectDestroy())
	}
	for _, s := range d.spec.Slots {
		has := s.Iface != nil && t.Implements(s.Iface)
		if !has && s.Required {
			missing = append(missing, s.Name)
		}
		if has {
			slots = append(slots, s.Serve)
		} else {
			slots = append(slots, nil)
		}
	}
	if len(missing) > 0 {
		return nil, &Error{Status: WrongKind, Message: fmt.Sprintf("%s does not implement %s, which the %s kind requires", t, strings.Join(missing, ", "), d.spec.Name)}
	}
	table := native.NewTable(d.spec.FloorHash, slots)
	actual, _ := d.tables.LoadOrStore(t, table)
	return actual.(unsafe.Pointer), nil
}

// serveObject is x as an object of d: a table over its methods and a ctx
// naming it, held until released.
func serveObject(d *kindDesc, x any) (rawObject, error) {
	if x == nil {
		return rawObject{}, &Error{Status: NullArgument, Message: "no object implementing " + d.spec.Name}
	}
	table, err := d.tableFor(reflect.TypeOf(x))
	if err != nil {
		return rawObject{}, err
	}
	return rawObject{table: table, size: d.size, ctx: hold(x, true)}, nil
}
