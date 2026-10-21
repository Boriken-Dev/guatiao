// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package gogreeter is greeters written in Go: the same kinds the Rust, C
// and Python greeters serve, offered by a library a host calls whatever
// language wrote it. The package is the library: LibraryOf names it by
// its import path and versions it from the build.
package gogreeter

import (
	"errors"
	"strings"
	"sync/atomic"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	greeterkind "github.com/Boriken-Dev/guatiao/examples/greeter_kind/go"
)

// Greeting is what the greeters say; the examples change it to show a
// different build.
var Greeting = "hello"

// Hello serves the greeter kind, every optional slot of it, and the
// counter kind: its own one instance.
type Hello struct {
	asked atomic.Int64
}

// The compiler checks what Hello implements.
var (
	_ greeterkind.Greeter     = (*Hello)(nil)
	_ greeterkind.Shouter     = (*Hello)(nil)
	_ greeterkind.Starter     = (*Hello)(nil)
	_ greeterkind.IntoGreeter = (*Hello)(nil)
	_ greeterkind.FromGreeter = (*Hello)(nil)
	_ greeterkind.Checker     = (*Hello)(nil)
	_ greeterkind.Counter     = (*Hello)(nil)
)

func (h *Hello) Greet(name string) (greeterkind.Greeting, error) {
	if name == "panic" {
		panic("a provider bug, on request")
	}
	if name == "" {
		return greeterkind.Greeting{}, guatiao.Errorf(guatiao.BadValue, "nobody to greet")
	}
	return greeterkind.Greeting{Greeting: Greeting + ", " + name}, nil
}

func (h *Hello) Shout(name string) (string, error) {
	g, err := h.Greet(name)
	return strings.ToUpper(g.Greeting), err
}

func (h *Hello) Start(listener greeterkind.Listener) (greeterkind.Conversation, error) {
	return &Chat{listener: listener}, nil
}

func (h *Hello) GreetInto(name string, listener greeterkind.Listener) error {
	g, err := h.Greet(name)
	if err != nil {
		return err
	}
	return listener.Heard(g.Greeting)
}

func (h *Hello) GreetFrom(config greeterkind.Named) (greeterkind.Greeting, error) {
	return h.Greet(config.Name)
}

func (h *Hello) Check(config guatiao.MapRef) (bool, error) {
	name := config.Key("name").StringOr("world")
	if err := config.Set("name", name); err != nil {
		return false, err
	}
	if err := config.Set("greeting", Greeting+", "+name); err != nil {
		return false, err
	}
	return name != "", nil
}

func (h *Hello) Count() (int64, error) { return h.asked.Add(1), nil }

// Chat is one conversation Start hands back, holding the listener it was
// handed; closing it closes the listener.
type Chat struct {
	listener   greeterkind.Listener
	transcript []string
}

var _ greeterkind.Conversation = (*Chat)(nil)

func (c *Chat) Say(what string) error {
	if what == "" {
		return guatiao.Errorf(guatiao.BadValue, "nothing to say")
	}
	c.transcript = append(c.transcript, what)
	return c.listener.Heard(what)
}

func (c *Chat) Read(dst []byte) (int64, error) {
	return int64(copy(dst, strings.Join(c.transcript, "\n"))), nil
}

func (c *Chat) Turns() (int64, error) { return int64(len(c.transcript)), nil }

func (c *Chat) Close() error { return c.listener.Close() }

// ShouterConfig is what a Shouter is built from; its schema is read from
// the struct.
type ShouterConfig struct {
	Prefix string `guatiao:"prefix" schema:"minLength=1"`
}

// Shouter is built from a configuration: one instance per configuration.
type Shouter struct{ prefix string }

var _ greeterkind.Greeter = (*Shouter)(nil)

// NewShouter builds one.
func NewShouter(config ShouterConfig) (*Shouter, error) {
	if config.Prefix == "" {
		return nil, errors.New("a shouter needs a prefix")
	}
	return &Shouter{prefix: config.Prefix}, nil
}

func (s *Shouter) Greet(name string) (greeterkind.Greeting, error) {
	return greeterkind.Greeting{Greeting: s.prefix + " " + strings.ToUpper(name)}, nil
}

// Library is the library this package is: what a host in this process
// registers, and what the c-shared build answers. Its id is the package's
// import path. A build from a checkout records no module version, so this
// one states its own; a module installed at a tag records its version,
// which LibraryOf reads when no LibraryVersion says otherwise.
func Library() (*guatiao.Library, error) {
	return guatiao.LibraryOf(
		guatiao.Single("hello", &Hello{}, greeterkind.GreeterKind, greeterkind.CounterKind,
			guatiao.DisplayName("Hello, in Go")),
		guatiao.Configured("shouter", NewShouter, greeterkind.GreeterKind),
		guatiao.LibraryVersion("1.0.0"),
		guatiao.Summary("Greeters written in Go"),
		guatiao.License("MPL-2.0"),
	)
}
