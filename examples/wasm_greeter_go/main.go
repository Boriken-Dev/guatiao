// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Greeters written in Go and compiled to WebAssembly: the same kinds the
// Rust, C, Python and Lua greeters serve, offered to any host through the
// guatiao-source-wasm source, sandboxed.
//
//	GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared -o wasm_greeter_go.wasm .
//
// The module is the library: its id is the manifest's `module`; Go writes
// no custom section, so its version is the manifest's.
package main

import (
	"strings"

	"github.com/Boriken-Dev/guatiao/crates/guatiao-source-wasm/guest-go"
)

// The word a greeting starts with; the tests build the module again with
// another (-ldflags "-X main.word=howdy") to show a refresh serving new
// code.
var word = "hello"

func greeting(name string) (*guest.Map, error) {
	if name == "" {
		return nil, guest.Refuse(guest.StatusBadValue, "nobody to greet")
	}
	return guest.NewMap().Set("greeting", word+", "+name), nil
}

func unknown(c *guest.Call) error {
	return guest.Refuse(guest.StatusUnsupported, "no `%s` for `%s`", c.Method(), c.Kind())
}

// chat is one conversation `start` hands back: the listener it was
// handed, and what was said.
type chat struct {
	listener *guest.HostObject
	said     []string
}

func (ch *chat) Call(c *guest.Call) (any, error) {
	switch c.Method() {
	case "say":
		what, err := c.Text(0)
		if err != nil {
			return nil, err
		}
		if what == "" {
			return nil, guest.Refuse(guest.StatusBadValue, "nothing to say")
		}
		if _, err := ch.listener.Call("heard", what); err != nil {
			return nil, err
		}
		ch.said = append(ch.said, what)
		return nil, nil
	case "read":
		dst, err := c.Buffer(0)
		if err != nil {
			return nil, err
		}
		return copy(dst, strings.Join(ch.said, " ")), nil
	case "turns":
		return len(ch.said), nil
	}
	return nil, unknown(c)
}

// Release lets the listener go with the conversation.
func (ch *chat) Release() { ch.listener.Drop() }

// hello serves two kinds, keeping its count in itself.
type hello struct{ asked int64 }

func (h *hello) Call(c *guest.Call) (any, error) {
	switch c.Kind() + "." + c.Method() {
	case "greeter.greet":
		name, err := c.Text(0)
		if err != nil {
			return nil, err
		}
		if name == "panic" {
			panic("a provider bug, on request")
		}
		return greeting(name)
	case "greeter.check":
		config, err := c.LentMap(0)
		if err != nil {
			return nil, err
		}
		if _, ok := config.Get("name"); !ok {
			config.Set("name", "ana")
		}
		config.Set("checked_by", "Go, in wasm")
		return true, nil
	case "greeter.greet_from":
		config, err := c.Map(0)
		if err != nil {
			return nil, err
		}
		name, _ := config.Get("name")
		text, _ := name.(string)
		return greeting(text)
	case "greeter.greet_into":
		name, err := c.Text(0)
		if err != nil {
			return nil, err
		}
		answer, err := greeting(name)
		if err != nil {
			return nil, err
		}
		listener, err := c.LentObject(1)
		if err != nil {
			return nil, err
		}
		text, _ := answer.Get("greeting")
		_, err = listener.Call("heard", text)
		return nil, err
	case "greeter.start":
		listener, err := c.Object(0)
		if err != nil {
			return nil, err
		}
		return &guest.Object{Handler: &chat{listener: listener}}, nil
	case "counter.count":
		h.asked++
		return h.asked, nil
	}
	return nil, unknown(c)
}

// polite answers `greet` alone: `shout` and the rest run the kind's
// defaults at the caller.
func polite(c *guest.Call) (any, error) {
	if c.Method() != "greet" {
		return nil, unknown(c)
	}
	name, err := c.Text(0)
	if err != nil {
		return nil, err
	}
	if name == "" {
		return nil, guest.Refuse(guest.StatusBadValue, "nobody to greet, politely")
	}
	return guest.NewMap().Set("greeting", "good day, "+name), nil
}

// wide answers 64-bit integers, both ways, at their limits.
func wide(c *guest.Call) (any, error) {
	switch c.Method() {
	case "echo_i64":
		return c.Int(0)
	case "echo_u64":
		return c.Uint(0)
	}
	return nil, unknown(c)
}

func init() {
	guest.Serve(&guest.Library{Providers: []guest.Provider{
		{
			ID: "hello", Name: "Hello, in Go (wasm)", Handler: &hello{},
			Serves: []guest.Serves{
				{Kind: "greeter", Slots: []string{"greet", "check", "greet_from", "greet_into", "start"}},
				{Kind: "counter", Slots: []string{"count"}},
			},
		},
		{
			ID: "polite", Name: "Polite, in Go (wasm)", Handler: guest.HandlerFunc(polite),
			Serves: []guest.Serves{{Kind: "greeter", Slots: []string{"greet"}}},
		},
		{
			ID: "wide", Name: "Wide, in Go (wasm)", Handler: guest.HandlerFunc(wide),
			Serves: []guest.Serves{{Kind: "wide", Slots: []string{"echo_i64", "echo_u64"}}},
		},
	}})
}

// A reactor's main is never run: init registers the library.
func main() {}
