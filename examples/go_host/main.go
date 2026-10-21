// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A plugin host written in Go.
//
// Scans the directories it is given for libraries declaring the greeter
// kind, whatever language they were written in, and calls each greeter
// through the kind's table: a map back, a map lent and changed in place, a
// map handed over, an object handed in and one handed back, an object lent
// for one call. A provider that builds instances from a configuration is
// given one its schema refuses, then one it accepts. The greeters written
// in Go, linked into this program, are registered beside them. Everything
// it opened is closed before it exits, and it prints what it did.
//
//	go run ./go_host DIR [DIR ...] [-python-source BRIDGE -python-dir DIR ... -python-path DIR ...]
package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	gogreeter "github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter"
	greeterkind "github.com/Boriken-Dev/guatiao/examples/greeter_kind/go"
)

// ear is a listener: what a conversation tells, kept.
type ear struct {
	heard  []string
	closed bool
}

func (e *ear) Heard(what string) error {
	e.heard = append(e.heard, what)
	return nil
}

func (e *ear) Close() error {
	e.closed = true
	return nil
}

type list []string

func (l *list) String() string     { return strings.Join(*l, ",") }
func (l *list) Set(v string) error { *l = append(*l, v); return nil }

func main() {
	var pythonDirs, pythonPaths list
	pythonSource := flag.String("python-source", "", "the guatiao-py bridge, as a source of Python libraries")
	flag.Var(&pythonDirs, "python-dir", "a directory of Python libraries for the bridge (repeatable)")
	flag.Var(&pythonPaths, "python-path", "a directory the Python libraries import from (repeatable)")
	withGo := flag.Bool("go", true, "register the greeters written in Go, linked into this program")
	flag.Parse()
	if err := run(os.Stdout, flag.Args(), *pythonSource, pythonDirs, pythonPaths, *withGo); err != nil {
		fmt.Fprintln(os.Stderr, "go_host:", err)
		os.Exit(1)
	}
}

func run(w io.Writer, dirs []string, pythonSource string, pythonDirs, pythonPaths []string, withGo bool) error {
	reg, err := guatiao.NewRegistry("go-host", "1.0")
	if err != nil {
		return err
	}
	var built []*guatiao.Library
	defer func() {
		reg.Close()
		// What this host built for itself, once no registry can call it.
		for _, lib := range built {
			lib.Close()
		}
		fmt.Fprintf(w, "objects a library can still reach: %d\n", guatiao.LiveObjects())
	}()
	for _, dir := range dirs {
		report, err := reg.ScanDir(dir, guatiao.Rules("kind=greeter"))
		if err != nil {
			return err
		}
		loaded, _ := report["loaded"].([]any)
		for _, file := range loaded {
			fmt.Fprintf(w, "loaded %s\n", filepath.Base(fmt.Sprint(file)))
		}
	}
	if withGo {
		lib, err := gogreeter.Library()
		if err != nil {
			return err
		}
		built = append(built, lib)
		if _, err := reg.RegisterLibrary(lib); err != nil {
			return err
		}
		fmt.Fprintf(w, "registered %s %s from Go\n", lib.ID(), lib.Version())
	}
	if pythonSource != "" {
		if err := reg.AddSourceFile(pythonSource, map[string]any{"dirs": pythonDirs, "paths": pythonPaths}); err != nil {
			return err
		}
		changed, err := reg.Refresh()
		if err != nil {
			return err
		}
		for _, served := range asMaps(changed["loaded"]) {
			fmt.Fprintf(w, "served %v from Python\n", served["id"])
		}
		for _, refused := range asMaps(changed["refused"]) {
			fmt.Fprintf(w, "refused %v: %v\n", refused["id"], refused["why"])
		}
	}

	providers, err := reg.Providers("greeter")
	if err != nil {
		return err
	}
	for _, provider := range providers {
		key := fmt.Sprint(provider["key"])
		fmt.Fprintf(w, "\n%s (%v %v)\n", key, provider["library"], provider["version"])
		say := func(format string, args ...any) { fmt.Fprintf(w, "  "+format+"\n", args...) }
		greeter, err := greeterkind.GreeterKind.Bind(reg, key)
		var mismatch *guatiao.FloorMismatch
		if errors.As(err, &mismatch) {
			say("not this greeter kind: %s", mismatch.Message)
			continue
		}
		if err != nil {
			return err
		}
		if provider["has_config"] != true {
			if err := exercise(greeter, say); err != nil {
				return err
			}
			continue
		}
		// Built from a configuration: one the schema refuses, then one it
		// accepts. The registry checks it before the provider sees it.
		if inst, err := reg.Create(key, map[string]any{}); err == nil {
			inst.Close()
		} else {
			say("an empty configuration is refused: %v", err)
		}
		config := map[string]any{"prefix": "hey"}
		inst, err := reg.Create(key, config)
		if err != nil {
			return err
		}
		say("instance from %v", config)
		configured, err := greeterkind.GreeterKind.BindInstance(inst)
		if err == nil {
			err = exercise(configured, say)
		}
		inst.Close()
		if err != nil {
			return err
		}
	}

	// Unmapped where the library agrees; a library with no say in it is
	// retired instead, and stays mapped.
	fmt.Fprintln(w)
	libraries, err := reg.Libraries()
	if err != nil {
		return err
	}
	for _, lib := range libraries {
		key := fmt.Sprint(lib["key"])
		if err := reg.Unload(key); err == nil {
			fmt.Fprintf(w, "unloaded %s\n", key)
			continue
		} else if retire := reg.Retire(key); retire != nil {
			return retire
		} else {
			fmt.Fprintf(w, "retired %s, still mapped: unload said %v\n", key, err)
		}
	}
	return nil
}

// exercise calls every slot of the greeter kind its table carries.
func exercise(g *greeterkind.GreeterRemote, say func(string, ...any)) error {
	answer, err := g.Greet("ana")
	if err != nil {
		return err
	}
	say("greet: %q", answer.Greeting)
	if _, err := g.Greet(""); err != nil {
		say("greet refused: %v", err)
	}
	if g.Has("shout") {
		loud, err := g.Shout("ana")
		if err != nil {
			return err
		}
		say("shout: %s", loud)
	}
	if g.Has("check") {
		config, err := guatiao.NewValue(map[string]any{"name": "bo"})
		if err != nil {
			return err
		}
		m, _ := config.Map()
		usable, err := g.Check(m)
		if err != nil {
			config.Close()
			return err
		}
		lent, _ := config.Any()
		say("check: %v, lent map now %v", usable, lent)
		config.Close()
	}
	if g.Has("greet_from") {
		answer, err := g.GreetFrom(greeterkind.Named{Name: "cy"})
		if err != nil {
			return err
		}
		say("greetFrom: %q", answer.Greeting)
	}
	if g.Has("greet_into") {
		e := &ear{}
		if err := g.GreetInto("dee", e); err != nil {
			return err
		}
		say("greetInto: the lent listener heard %q", e.heard)
	}
	if g.Has("start") {
		e := &ear{}
		conversation, err := g.Start(e)
		if err != nil {
			say("start refused: %v", err)
			return nil
		}
		defer conversation.Close()
		for _, line := range []string{"hi", "again"} {
			if err := conversation.Say(line); err != nil {
				return err
			}
		}
		transcript := make([]byte, 64)
		n, err := conversation.Read(transcript)
		if err != nil {
			return err
		}
		turns, err := conversation.Turns()
		if err != nil {
			return err
		}
		say("conversation: %d turns, transcript %q, listener heard %q", turns, transcript[:n], e.heard)
	}
	return nil
}

func asMaps(x any) []map[string]any {
	list, _ := x.([]any)
	var out []map[string]any
	for _, item := range list {
		if m, ok := item.(map[string]any); ok {
			out = append(out, m)
		}
	}
	return out
}
