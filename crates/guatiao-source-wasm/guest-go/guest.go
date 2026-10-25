// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package guest writes providers in Go for the guatiao-source-wasm source: a
// library is a WebAssembly reactor built by Go's own toolchain,
//
//	GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared -o mylib.wasm .
//
// which registers its Library in an init function. Calls arrive at a
// Handler with the kind and slot named and their arguments as values:
// Absent, nil (null), bool, Number, string, []byte, []any or *Map.
package guest

import (
	"fmt"
	"strconv"
)

// Status is a guatiao status: what a refusal crosses as.
type Status uint32

// The statuses a refusal may carry, as guatiao.h names them.
const (
	StatusBadValue    Status = 7
	StatusAlloc       Status = 8
	StatusWrongKind   Status = 9
	StatusNotFound    Status = 10
	StatusNull        Status = 11
	StatusGone        Status = 12
	StatusUnsupported Status = 13
	StatusBusy        Status = 14
	StatusInternal    Status = 100
)

// Refusal is an error with a guatiao status. Any other error a handler
// returns crosses as StatusBadValue with its text.
type Refusal struct {
	Status  Status
	Message string
}

func (r *Refusal) Error() string { return r.Message }

// Refuse is a refusal with this status.
func Refuse(status Status, format string, args ...any) *Refusal {
	return &Refusal{Status: status, Message: fmt.Sprintf(format, args...)}
}

// Handler answers the calls of a provider, or of an object a provider
// answered. Its answer is nil for nothing, an *Object for an object, or a
// value.
type Handler interface {
	Call(c *Call) (any, error)
}

// HandlerFunc is a function as a Handler.
type HandlerFunc func(c *Call) (any, error)

// Call calls f.
func (f HandlerFunc) Call(c *Call) (any, error) { return f(c) }

// Object is an answer for a slot that answers an object: its calls come to
// Handler until the caller destroys it. A handler that is also a Releaser
// hears that, and drops the host objects it holds then.
type Object struct {
	Handler Handler
}

// Releaser is an object's handler that is told when the caller destroyed
// the object.
type Releaser interface {
	Release()
}

// Provider is one provider: an id unique in its library (the host serves
// it as `<module>_<id>`), a name for a person, the kinds it serves with
// the slots it answers, and its handler. A slot left out runs the kind's
// default at the caller; a required one left out refuses the library.
type Provider struct {
	ID      string
	Name    string
	Serves  []Serves
	Handler Handler
}

// Serves names a kind and the slots a provider answers of it.
type Serves struct {
	Kind  string
	Slots []string
}

// Library is the providers a module offers, in the order the host
// addresses them.
type Library struct {
	Providers []Provider
}

func (l *Library) declaration() *Map {
	providers := []any{}
	for _, p := range l.Providers {
		serves := NewMap()
		for _, s := range p.Serves {
			slots := []any{}
			for _, slot := range s.Slots {
				slots = append(slots, slot)
			}
			serves.Set(s.Kind, slots)
		}
		name := p.Name
		if name == "" {
			name = p.ID
		}
		providers = append(providers, NewMap().Set("id", p.ID).Set("name", name).Set("serves", serves))
	}
	return NewMap().Set("providers", providers)
}

// Call is one call: the kind and slot it is for, and its arguments, one per
// argument of the slot, in order.
type Call struct {
	kind, method string
	args         []any
	changed      []bool
}

// Kind is the kind called.
func (c *Call) Kind() string { return c.kind }

// Method is the slot called.
func (c *Call) Method() string { return c.method }

// Len is how many arguments it has.
func (c *Call) Len() int { return len(c.args) }

// Arg is argument i as it arrived.
func (c *Call) Arg(i int) (any, error) {
	if i < 0 || i >= len(c.args) {
		return nil, Refuse(StatusBadValue, "`%s` has no argument %d", c.method, i)
	}
	return c.args[i], nil
}

func (c *Call) not(i int, what string) error {
	return Refuse(StatusBadValue, "`%s`'s argument %d is not %s", c.method, i, what)
}

// Text is a text argument.
func (c *Call) Text(i int) (string, error) {
	v, err := c.Arg(i)
	if s, ok := v.(string); ok && err == nil {
		return s, nil
	}
	if err != nil {
		return "", err
	}
	return "", c.not(i, "text")
}

// Bytes is a bytes argument.
func (c *Call) Bytes(i int) ([]byte, error) {
	v, err := c.Arg(i)
	if err != nil {
		return nil, err
	}
	if b, ok := v.([]byte); ok {
		return b, nil
	}
	return nil, c.not(i, "bytes")
}

// Int is a signed whole-number argument.
func (c *Call) Int(i int) (int64, error) {
	v, err := c.Arg(i)
	if err != nil {
		return 0, err
	}
	if n, ok := v.(Number); ok {
		if x, ok := n.Int(); ok {
			return x, nil
		}
	}
	return 0, c.not(i, "a signed whole number")
}

// Uint is an unsigned whole-number argument.
func (c *Call) Uint(i int) (uint64, error) {
	v, err := c.Arg(i)
	if err != nil {
		return 0, err
	}
	if n, ok := v.(Number); ok {
		if x, ok := n.Uint(); ok {
			return x, nil
		}
	}
	return 0, c.not(i, "an unsigned whole number")
}

// Float is a floating-point argument.
func (c *Call) Float(i int) (float64, error) {
	v, err := c.Arg(i)
	if err != nil {
		return 0, err
	}
	if n, ok := v.(Number); ok {
		if x, ok := n.Float(); ok {
			return x, nil
		}
	}
	return 0, c.not(i, "a number")
}

// Bool is a boolean argument.
func (c *Call) Bool(i int) (bool, error) {
	v, err := c.Arg(i)
	if err != nil {
		return false, err
	}
	if b, ok := v.(bool); ok {
		return b, nil
	}
	return false, c.not(i, "a boolean")
}

// Map is a map argument, borrowed or handed over.
func (c *Call) Map(i int) (*Map, error) {
	v, err := c.Arg(i)
	if err != nil {
		return nil, err
	}
	if m, ok := v.(*Map); ok {
		return m, nil
	}
	return nil, c.not(i, "a map")
}

// LentMap is a lent map: what it holds when the call returns is written
// back over the caller's.
func (c *Call) LentMap(i int) (*Map, error) {
	m, err := c.Map(i)
	if err == nil {
		c.changed[i] = true
	}
	return m, err
}

// SetLent replaces lent argument i, to be written back.
func (c *Call) SetLent(i int, value any) error {
	if _, err := c.Arg(i); err != nil {
		return err
	}
	c.args[i] = value
	c.changed[i] = true
	return nil
}

// Buffer is the caller's buffer (bytes_mut), as long as the caller's:
// what it holds when the call returns is written back.
func (c *Call) Buffer(i int) ([]byte, error) {
	b, err := c.Bytes(i)
	if err == nil {
		c.changed[i] = true
	}
	return b, err
}

// Object is an object handed over: the call's, until it is dropped.
func (c *Call) Object(i int) (*HostObject, error) {
	h, err := c.Int(i)
	if err != nil {
		return nil, err
	}
	return &HostObject{handle: h, owned: true}, nil
}

// LentObject is an object lent for this call: the host refuses it once
// the call returns.
func (c *Call) LentObject(i int) (*HostObject, error) {
	h, err := c.Int(i)
	if err != nil {
		return nil, err
	}
	return &HostObject{handle: h}, nil
}

// HostObject is an object of the host's: a listener handed over or lent,
// or an object a host's slot answered. Its calls go to the host.
type HostObject struct {
	handle int64
	owned  bool
}

// Call calls method of the object's kind with args, in the slot's order.
// It answers Absent for a slot that answers nothing.
func (o *HostObject) Call(method string, args ...any) (any, error) {
	request, err := Encode(append([]any{method}, args...))
	if err != nil {
		return nil, err
	}
	status, result := hostObjectCall(o.handle, request)
	if status != 0 {
		return nil, &Refusal{Status: Status(status), Message: string(result)}
	}
	if len(result) == 0 {
		return Absent, nil
	}
	return Decode(result)
}

// Drop lets go of an object handed over; the host destroys it. Go has no
// destructor to do it: an object not dropped goes with the module's
// instance.
func (o *HostObject) Drop() {
	if o.owned {
		o.owned = false
		hostDrop(o.handle)
	}
}

// Handle is the object's handle, to hand it on to a host slot that takes
// one; the object is no longer this one's to drop.
func (o *HostObject) Handle() Number {
	o.owned = false
	return Number(strconv.FormatInt(o.handle, 10))
}

// What the host's imports are, set by the build for wasip1 and by the
// tests elsewhere.
var (
	hostAnswer     func(b []byte)
	hostWriteBack  func(index uint32, b []byte) uint32
	hostObjectCall func(handle int64, request []byte) (uint32, []byte)
	hostDrop       func(handle int64)
)

// The library registered, and the objects its providers answered.
var (
	served  *Library
	objects = map[int64]Handler{}
	nextID  int64
)

// Serve registers the library this module offers. Call it from an init
// function: a reactor's main is never run.
func Serve(library *Library) {
	served = library
}

func refuse(err error) uint32 {
	r, ok := err.(*Refusal)
	if !ok {
		r = &Refusal{Status: StatusBadValue, Message: err.Error()}
	}
	hostAnswer([]byte(r.Message))
	return uint32(r.Status)
}

func declare() uint32 {
	if served == nil {
		return refuse(Refuse(StatusInternal, "no library was served: call guest.Serve in an init function"))
	}
	out, err := Encode(served.declaration())
	if err != nil {
		return refuse(err)
	}
	hostAnswer(out)
	return 0
}

func read(request []byte) (*Call, error) {
	value, err := Decode(request)
	if err != nil {
		return nil, Refuse(StatusBadValue, "the request: %v", err)
	}
	list, ok := value.([]any)
	if !ok || len(list) < 2 {
		return nil, Refuse(StatusBadValue, "the request is not a list [kind, method, arg...]")
	}
	kind, ok1 := list[0].(string)
	method, ok2 := list[1].(string)
	if !ok1 || !ok2 {
		return nil, Refuse(StatusBadValue, "the request names no kind and method")
	}
	args := list[2:]
	return &Call{kind: kind, method: method, args: args, changed: make([]bool, len(args))}, nil
}

func dispatch(object bool, target int64, c *Call) (answer any, err error) {
	defer func() {
		if p := recover(); p != nil {
			err = Refuse(StatusInternal, "panic: %v", p)
		}
	}()
	var h Handler
	switch {
	case object:
		h = objects[target]
		if h == nil {
			return nil, Refuse(StatusGone, "no object %d", target)
		}
	case served != nil && target >= 0 && target < int64(len(served.Providers)):
		h = served.Providers[target].Handler
	default:
		return nil, Refuse(StatusNotFound, "no provider %d", target)
	}
	answer, err = h.Call(c)
	if err != nil {
		return nil, err
	}
	for i, changed := range c.changed {
		if !changed {
			continue
		}
		out, err := Encode(c.args[i])
		if err != nil {
			return nil, err
		}
		if status := hostWriteBack(uint32(i), out); status != 0 {
			return nil, Refuse(Status(status), "argument %d of `%s` could not be written back: is it lent?", i, c.method)
		}
	}
	return answer, nil
}

func call(object bool, target int64, request []byte) uint32 {
	c, err := read(request)
	if err != nil {
		return refuse(err)
	}
	answer, err := dispatch(object, target, c)
	if err != nil {
		return refuse(err)
	}
	switch a := answer.(type) {
	case nil:
		return 0
	case *Object:
		nextID++
		objects[nextID] = a.Handler
		answer = nextID
	}
	out, err := Encode(answer)
	if err != nil {
		return refuse(err)
	}
	hostAnswer(out)
	return 0
}

func release(object int64) {
	h := objects[object]
	delete(objects, object)
	if r, ok := h.(Releaser); ok {
		r.Release()
	}
}
