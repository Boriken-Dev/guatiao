// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The greeters of go_greeter as a library file any host's scan finds:
//
//	go build -buildmode=c-shared -o go_greeter.dll ./go_provider/cshared   # libgo_greeter.so, libgo_greeter.dylib
//
// It exports guatiao_library_entry, answered by the library, and
// guatiao_declares (declares.c), which a scan reads as data before it
// maps the file. The library's own guatiao is found as every Go program
// using the binding finds it: GUATIAO_LIBRARY, or the platform's search.
package main

import "C"

import (
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	gogreeter "github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter"
)

var library, libraryErr = gogreeter.Library()

func init() {
	if libraryErr != nil {
		guatiao.Warn(libraryErr)
	}
}

//export guatiao_library_entry
func guatiao_library_entry(host unsafe.Pointer) unsafe.Pointer { return library.Answer(host) }

func main() {}
