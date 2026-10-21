// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package guatiao is the Go binding of the guatiao ABI: one value model
// for passing data between languages, a JSON Schema that describes a
// value, and a registry that loads libraries of providers.
//
// The binding calls the C ABI through cgo and ships no native library: it
// loads one at run time, so a build needs a C compiler and nothing else.
//
// # Finding the native library
//
// Nothing native is touched until the first call that needs it. The
// library is then found in this order: a path given to UseLibrary before
// that call (a file exporting the ABI, whatever it is called, or a
// directory); GUATIAO_LIBRARY, a directory holding guatiao.dll,
// libguatiao.so or libguatiao.dylib, or such a file; the platform's own
// search by file name. guatiao_serde and guatiao_intake are found the
// same way, beside it under their own names, when the serde and intake
// packages are first used. Not found is a *LibraryNotFound naming every
// route; an export the loaded build lacks is a *MissingSymbol.
//
// # Values
//
// A *Value owns a tree's root; Close frees it. A Ref addresses a node
// inside a tree by a path of keys and indices, walked again on every
// access, so it stays valid while the tree grows around it. MapRef and
// ListRef are Refs over a map and a list.
//
//	v, err := guatiao.NewValue(map[string]any{"host": "10.0.0.1", "port": 5900})
//	defer v.Close()
//	m, _ := v.Map()
//	m.Set("port", 5901)
//	v.Key("port").IntOr(22) // 5901
//
// # Go values and trees
//
// NewValue, MapRef.Set, ListRef.Push and the kind glue convert Go data to
// a tree, and Ref.Decode converts back, as encoding/json does:
//
//   - nil is null; bool is a boolean.
//   - The integer and float types are numbers, written as their exact
//     text; decoding refuses a number that does not fit the target, a
//     fraction or an exponent into an integer, and never rounds one into
//     another. Number holds any number's exact text, and is what decoding
//     into any gives. *big.Int is an integer of any size.
//   - string is a string (UTF-8, or refused); []byte is bytes.
//   - A slice or an array is a list; decoding into an array refuses a
//     list of another length.
//   - A map keyed by strings, integers, bools or a TextMarshaler is a map,
//     its keys written sorted (integers by value), since a Go map has no
//     order.
//   - A struct is a map: each exported field under its `guatiao` tag's
//     name, else its `json` tag's, else its own; "-" leaves a field out,
//     `omitempty` leaves out a zero one, and a nil pointer field is always
//     left out, as an absent field is. An untagged embedded struct lends
//     its fields. Decoding ignores keys it has no field for.
//   - time.Time is an instant's text; time.Duration is an exact length,
//     [seconds, nanoseconds], read back from any of its three spellings.
//   - A Marshaler or Unmarshaler says what it is itself; otherwise a
//     TextMarshaler or TextUnmarshaler is its text.
//   - A *Value, Ref, MapRef or ListRef is copied in; decoding into a
//     *Value takes a copy.
//   - Decoding into any gives nil, bool, Number, string, []byte, []any
//     and map[string]any.
//
// The forms are fixed for every language at
// https://boriken-dev.github.io/guatiao/encodings/.
//
// # Hosting, kinds and providers
//
// A Registry loads libraries and lists their providers; a kind's
// descriptor binds a provider's table as a remote. A kind is a Go
// interface whose glue guatiao-kind (cmd/guatiao-kind) writes from the
// kind's C header, run by go generate; a provider is any Go type
// implementing it, named with Single or Configured in a Library, which
// LibraryOf names by its package and versions from the build. A Library
// is registered in this process, or built with -buildmode=c-shared and
// answered from an exported guatiao_library_entry, a file any host loads.
//
// # Threads
//
// A Value is not safe for concurrent use; reading one from several
// goroutines at once is. A Registry is not either. A slot this package
// serves runs on the thread that calls it, one Go did not start
// included.
package guatiao
