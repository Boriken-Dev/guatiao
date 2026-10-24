// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package rustcaller drives greeter_caller, a Rust caller of any greeter:
// given a registry's host block and a provider's key, it calls every slot
// of the greeter kind (or the 64-bit limits of the wide kind) through the
// kind's Rust proxy and answers what each call gave. The examples' tests
// use it to have Rust call a greeter written in Go.
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

typedef guatiao_status (*rc_wide)(const guatiao_host_info *host, guatiao_str key, guatiao_value *out);

static void *rc_open(const char *path, const char *symbol) {
#ifdef _WIN32
  HMODULE h = LoadLibraryA(path);
  return h ? (void *)GetProcAddress(h, symbol) : NULL;
#else
  void *h = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  return h ? dlsym(h, symbol) : NULL;
#endif
}

static uint32_t rc_call_wide(void *fn, const void *host, const char *key, size_t keylen, void *out) {
  guatiao_str k = {(const uint8_t *)key, keylen};
  return ((rc_wide)fn)((const guatiao_host_info *)host, k, (guatiao_value *)out);
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

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// Caller is greeter_caller_run and greeter_caller_wide, loaded from a
// build of greeter_caller.
type Caller struct{ fn, wide unsafe.Pointer }

// Open loads greeter_caller from path.
func Open(path string) (*Caller, error) {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	run, wide := C.CString("greeter_caller_run"), C.CString("greeter_caller_wide")
	defer C.free(unsafe.Pointer(run))
	defer C.free(unsafe.Pointer(wide))
	fn, wideFn := C.rc_open(cpath, run), C.rc_open(cpath, wide)
	if fn == nil || wideFn == nil {
		return nil, fmt.Errorf("%s does not open or exports no greeter_caller_run and greeter_caller_wide", path)
	}
	return &Caller{fn: fn, wide: wideFn}, nil
}

// RunWide calls the wide provider filed under key with the 64-bit limits
// through host's services, and answers the report {i64, u64}, a Value to
// close.
func (c *Caller) RunWide(host unsafe.Pointer, key string) (*guatiao.Value, error) {
	out := C.rc_out()
	ckey := C.CString(key)
	defer C.free(unsafe.Pointer(ckey))
	st := C.rc_call_wide(c.wide, host, ckey, C.size_t(len(key)), out)
	report := guatiao.ValueFromRaw(out)
	if st != 0 {
		report.Close()
		return nil, fmt.Errorf("greeter_caller_wide answered status %d", st)
	}
	return report, nil
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
