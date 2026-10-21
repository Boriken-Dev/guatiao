// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package rustcaller drives greeter_caller, a Rust caller of any greeter:
// given a registry's host block and a provider's key, it calls every slot
// of the greeter kind through the kind's Rust proxy and answers what each
// call gave. The examples' tests use it to have Rust call a greeter
// written in Go.
package rustcaller

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/guatiao/include
#cgo linux LDFLAGS: -ldl
#include <stdint.h>
#include <stdlib.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif
#include "guatiao.h"

typedef guatiao_status (*rc_run)(const guatiao_host_info *host, guatiao_str key, guatiao_str name,
                                 guatiao_value *out);

static void *rc_open(const char *path) {
#ifdef _WIN32
  HMODULE h = LoadLibraryA(path);
  return h ? (void *)GetProcAddress(h, "greeter_caller_run") : NULL;
#else
  void *h = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  return h ? dlsym(h, "greeter_caller_run") : NULL;
#endif
}

static uint32_t rc_call(void *fn, const void *host, const char *key, size_t keylen, const char *name,
                        size_t namelen, void *out) {
  guatiao_str k = {(const uint8_t *)key, keylen};
  guatiao_str n = {(const uint8_t *)name, namelen};
  return ((rc_run)fn)((const guatiao_host_info *)host, k, n, (guatiao_value *)out);
}

static void *rc_out(void) { return calloc(1, sizeof(guatiao_value)); }
*/
import "C"

import (
	"fmt"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
)

// Caller is greeter_caller_run, loaded from a build of greeter_caller.
type Caller struct{ fn unsafe.Pointer }

// Open loads greeter_caller from path.
func Open(path string) (*Caller, error) {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	fn := C.rc_open(cpath)
	if fn == nil {
		return nil, fmt.Errorf("%s does not open or exports no greeter_caller_run", path)
	}
	return &Caller{fn: fn}, nil
}

// Run calls every slot of the greeter filed under key through host's
// services, greeting name, and answers the report, a Value to close.
func (c *Caller) Run(host unsafe.Pointer, key, name string) (*guatiao.Value, error) {
	out := C.rc_out()
	ckey, cname := C.CString(key), C.CString(name)
	defer C.free(unsafe.Pointer(ckey))
	defer C.free(unsafe.Pointer(cname))
	st := C.rc_call(c.fn, host, ckey, C.size_t(len(key)), cname, C.size_t(len(name)), out)
	report := guatiao.ValueFromRaw(out)
	if st != 0 {
		report.Close()
		return nil, fmt.Errorf("greeter_caller_run answered status %d", st)
	}
	return report, nil
}
