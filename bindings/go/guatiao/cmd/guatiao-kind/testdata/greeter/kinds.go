// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package greeter declares the greeter kinds as Go interfaces with no
// document behind them; they describe byte for byte as the document
// #[guatiao::kind] writes (examples/greeter_kind/include/greeter_kind.kinds.json).
package greeter

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

// Greets by name.
//
//guatiao:kind greeter
//guatiao:appended Shouter Starter IntoGreeter FromGreeter Checker
type Greeter interface {
	// `{"greeting": "..."}`, or why not.
	Greet(name string) (map[string]any, error)
}

// Appended after `greet`: a library built before it existed makes
// the proxy run this body instead.
type Shouter interface {
	//guatiao:infallible
	Shout(name string) (string, error)
}

// Appended: a conversation the caller then drives, told to the
// listener the caller handed in. A greeter built before this slot
// existed, or one that holds no conversations, answers the default.
type Starter interface {
	Start(listener Listener) (Conversation, error)
}

// Appended: the greeting, told to a listener the caller LENDS for
// this call alone. It stays on the caller's side: nothing is
// allocated to pass it and nothing is destroyed after. A greeter
// built before this slot existed greets through `greet`.
type IntoGreeter interface {
	//guatiao:lent listener
	GreetInto(name string, listener Listener) error
}

// Appended: the greeting for a configuration the caller HANDS OVER
// (`{"name": ".."}`). The greeter owns the map from the call on,
// whatever it answers, and may keep it; nothing is copied to pass
// it. A greeter built before this slot existed greets the name
// through `greet`.
type FromGreeter interface {
	//guatiao:taken config
	GreetFrom(config Named) (map[string]any, error)
}

// Named is what greet_from is handed.
type Named struct {
	Name string `guatiao:"name"`
}

// Appended: completes a configuration for `greet_from` that the
// caller LENDS for this call, in the caller's own map: fills in what
// it leaves out and adds what follows from it. Answers whether it can
// be greeted. A greeter built before this slot existed changes
// nothing.
type Checker interface {
	//guatiao:lent config
	Check(config guatiao.MapRef) (bool, error)
}

// What a conversation tells whoever is listening. Implemented by the
// HOST and handed to the library, which owns it from then on.
//
//guatiao:kind listener
type Listener interface {
	// Something was said.
	//
	//guatiao:infallible
	Heard(what string) error
	Close() error
}

// One conversation a greeter started: state the caller drives.
//
//guatiao:kind conversation
type Conversation interface {
	// Say something; the listener hears it.
	Say(what string) error
	// Copies the transcript so far into `dst` and answers how many bytes
	// it wrote — an out-buffer, the caller's own memory.
	//
	//guatiao:infallible
	//guatiao:mut dst
	Read(dst []byte) (int64, error)
	// How many things were said.
	//
	//guatiao:infallible
	Turns() (int64, error)
	Close() error
}

// Counts how often it was asked.
//
//guatiao:kind counter
type Counter interface {
	// The count so far, after this call.
	//
	//guatiao:infallible
	Count() (int64, error)
}

// Answers the 64-bit integer it is given, unchanged: what a host in any
// language crosses its 64-bit numbers with, both ways and at their
// limits.
//
//guatiao:kind wide
type Wide interface {
	// `x`, as it arrived.
	EchoI64(x int64) (int64, error)
	// `x`, as it arrived.
	EchoU64(x uint64) (uint64, error)
}
