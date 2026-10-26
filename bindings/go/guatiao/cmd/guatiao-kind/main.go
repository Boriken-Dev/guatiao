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
// values. The kinds' description says what each slot takes
// (`-description`, the <crate>.kinds.json a kind crate commits, from the
// package's directory), or their C header does (`-header`); the
// interfaces say how Go spells it. Run it with go generate:
//
//	//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/guatiao/cmd/guatiao-kind -description ../include/greeter_kind.kinds.json
//
// The C table name in the marker is needed only with -header; without
// it, it is <kind name>_vtable.
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
	description := flag.String("description", "", "the kinds' description document, from the package's directory")
	dir := flag.String("dir", ".", "the package's directory")
	out := flag.String("o", "guatiao_kinds", "the base name of what is written")
	check := flag.Bool("check", false, "write nothing; fail when what is there is not what would be written")
	flag.Parse()
	from := *header
	if (*header == "") == (*description == "") {
		fmt.Fprintln(os.Stderr, "guatiao-kind: -description names the kinds' document, or -header their C header: one of them")
		os.Exit(2)
	}
	if *description != "" {
		from = *description
	}
	if err := run(*dir, from, *out, *check); err != nil {
		fmt.Fprintln(os.Stderr, "guatiao-kind:", err)
		os.Exit(1)
	}
}
