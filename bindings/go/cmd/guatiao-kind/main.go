// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Command guatiao-kind writes the Go glue of a package's guatiao kinds:
// for each interface marked
//
//	//guatiao:kind <kind name> <C table name>
//
// a descriptor (<Interface>Kind, a *guatiao.Kind), a remote
// (<Interface>Remote, implementing the interface and every optional one),
// and the cgo that calls a table of the kind and serves one over Go
// values. The kind's C header says what each slot takes (`-header`, from
// the package's directory); the interfaces say how Go spells it. Run it
// with go generate:
//
//	//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/cmd/guatiao-kind -header ../include/greeter_kind.h
//
// A kind's interface holds its required slots, an object kind's its
// Close (the destroy slot) too; each other slot is a method of an
// interface of its own in the package. A slot's Go method is its C name
// in CamelCase (greet_into is GreetInto) and returns an error last. It
// writes guatiao_kinds.go, guatiao_kinds.h and guatiao_kinds.c beside the
// package's sources; -check writes nothing and fails when they are not
// what it would write.
package main

import (
	"flag"
	"fmt"
	"os"
)

func main() {
	header := flag.String("header", "", "the kinds' C header, from the package's directory")
	dir := flag.String("dir", ".", "the package's directory")
	out := flag.String("o", "guatiao_kinds", "the base name of what is written")
	check := flag.Bool("check", false, "write nothing; fail when what is there is not what would be written")
	flag.Parse()
	if *header == "" {
		fmt.Fprintln(os.Stderr, "guatiao-kind: -header names the kinds' C header")
		os.Exit(2)
	}
	if err := run(*dir, *header, *out, *check); err != nil {
		fmt.Fprintln(os.Stderr, "guatiao-kind:", err)
		os.Exit(1)
	}
}
