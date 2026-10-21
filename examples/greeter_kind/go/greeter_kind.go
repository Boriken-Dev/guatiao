// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package greeterkind holds the kinds of include/greeter_kind.h, for Go:
// an interface per kind, holding the slots every table has, and an
// interface per slot appended after them. guatiao-kind reads the header
// and these interfaces and writes the rest (guatiao_kinds.go, .h, .c): a
// descriptor per kind (GreeterKind), a remote implementing every
// interface (GreeterRemote), and the cgo that calls and serves a table.
// A host calls a greeter through GreeterRemote; a provider implements
// Greeter, and any of the optional interfaces it wants.
package greeterkind

//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/cmd/guatiao-kind -header ../include/greeter_kind.h

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go"

// Greeting is what a greeter answers: the map {"greeting": text}.
type Greeting struct {
	Greeting string `guatiao:"greeting"`
}

// Named is what greet_from is handed: a map with a name.
type Named struct {
	Name string `guatiao:"name"`
}

// Greeter greets by name: greeter_vtable. Greet is the slot every table
// has; the others were appended, and an older table may lack them.
//
//guatiao:kind greeter greeter_vtable
type Greeter interface {
	Greet(name string) (Greeting, error)
}

// Shouter is greeter_vtable.shout: the greeting, loud.
type Shouter interface {
	Shout(name string) (string, error)
}

// Starter is greeter_vtable.start: a conversation told to the listener
// handed in, which the conversation then owns.
type Starter interface {
	Start(listener Listener) (Conversation, error)
}

// IntoGreeter is greeter_vtable.greet_into: the greeting told to a
// listener lent for the call.
type IntoGreeter interface {
	GreetInto(name string, listener Listener) error
}

// FromGreeter is greeter_vtable.greet_from: greets from a map handed
// over.
type FromGreeter interface {
	GreetFrom(config Named) (Greeting, error)
}

// Checker is greeter_vtable.check: fills a configuration lent for the
// call, where it stands, and says whether it is usable.
type Checker interface {
	Check(config guatiao.MapRef) (bool, error)
}

// Counter counts how often it was asked: counter_vtable.
//
//guatiao:kind counter counter_vtable
type Counter interface {
	Count() (int64, error)
}

// Listener is what a conversation tells: listener_vtable, an object kind,
// implemented by a host. Close is its destroy slot.
//
//guatiao:kind listener listener_vtable
type Listener interface {
	Heard(what string) error
	Close() error
}

// Conversation is one conversation a greeter started: conversation_vtable,
// an object kind its caller owns and closes.
//
//guatiao:kind conversation conversation_vtable
type Conversation interface {
	Say(what string) error
	Read(dst []byte) (int64, error)
	Turns() (int64, error)
	Close() error
}
