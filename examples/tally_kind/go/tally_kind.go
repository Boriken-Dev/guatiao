// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package tallykind declares the tally kinds in Go, as interfaces: a kind
// born here, with no trait, header or document behind it. guatiao-kind
// reads these interfaces into ../include/tally_kind.kinds.json, the same
// document the TypeScript declaration in ../tally_kind.ts is read into,
// and writes the glue (guatiao_kinds.go, .h, .c) from it. What a Go type
// cannot say (a slot that cannot fail, an argument lent or handed over) is
// a //guatiao: directive in the method's doc comment.
package tallykind

//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/guatiao/cmd/guatiao-kind -document ../include/tally_kind.kinds.json

// What a tally tells whoever is watching it. Implemented by the host.
//
//guatiao:kind watcher
type Watcher interface {
	// `name` now stands at `total`.
	//
	//guatiao:infallible
	Changed(name string, total int64) error
	Close() error
}

// Counts by name.
//
//guatiao:kind tally
//guatiao:appended AllCounter Knower Checksummer Reporter Loader
type Tally interface {
	// Adds `by` to the count under `name`: the new count, or why not.
	Add(name string, by uint32) (int64, error)
	// The count under `name`: 0 when there is none.
	//
	//guatiao:infallible
	Total(name string) (int64, error)
}

// Optional: every count, as `{name: count}`.
type AllCounter interface {
	//guatiao:infallible
	All() (map[string]int64, error)
}

// Optional: whether `name` has a count.
type Knower interface {
	//guatiao:infallible
	Known(name string) (bool, error)
}

// Optional: the sum of the bytes, modulo 2^32.
type Checksummer interface {
	//guatiao:infallible
	Checksum(data []byte) (uint32, error)
}

// Optional: tells `watcher` each count, for this call alone.
type Reporter interface {
	//guatiao:lent watcher
	Report(watcher Watcher) error
}

// Optional: adds the counts of `counts` (`{name: count}`), which the
// tally owns from the call on; answers how many names it held.
type Loader interface {
	//guatiao:taken counts
	Load(counts map[string]int64) (uint32, error)
}
