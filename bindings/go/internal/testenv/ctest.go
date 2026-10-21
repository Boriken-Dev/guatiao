// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package testenv

/*
#cgo linux LDFLAGS: -ldl
#include <stdbool.h>
#include <stdlib.h>
#ifdef _WIN32
#include <windows.h>
static void *te_open(const char *path) { return (void *)LoadLibraryA(path); }
static void *te_sym(void *h, const char *name) { return (void *)GetProcAddress((HMODULE)h, name); }
#else
#include <dlfcn.h>
static void *te_open(const char *path) { return dlopen(path, RTLD_NOW | RTLD_LOCAL); }
static void *te_sym(void *h, const char *name) { return dlsym(h, name); }
#endif
static void te_void_int(void *fn, int i) { ((void (*)(int))fn)(i); }
static void te_void_bool(void *fn, bool b) { ((void (*)(bool))fn)(b); }
static int te_int_int(void *fn, int i) { return ((int (*)(int))fn)(i); }
*/
import "C"

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"unsafe"
)

// Filename is the platform's file name for a library of this base name.
func Filename(base string) string {
	switch runtime.GOOS {
	case "windows":
		return base + ".dll"
	case "darwin":
		return "lib" + base + ".dylib"
	}
	return "lib" + base + ".so"
}

// Workspace is the directory GUATIAO_LIBRARY names.
func Workspace() string { return os.Getenv("GUATIAO_LIBRARY") }

// Compiler is the C compiler cgo uses, or "".
func Compiler() string {
	if cc := os.Getenv("CC"); cc != "" {
		return cc
	}
	out, err := exec.Command("go", "env", "CC").Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

// BuildC builds one of the repository's C libraries (a path under
// crates/guatiao/tests/c_consumer) into dir as the library name.
func BuildC(source, dir, name string) (string, error) {
	repo := Repo()
	cc := Compiler()
	if repo == "" || cc == "" {
		return "", errors.New("no checkout or no C compiler")
	}
	out := filepath.Join(dir, Filename(name))
	args := []string{"-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"}
	if runtime.GOOS != "windows" {
		args = append(args, "-fPIC")
	}
	args = append(args, "-I", filepath.Join(repo, "crates", "guatiao", "include"),
		"-I", filepath.Join(repo, "examples", "greeter_kind", "include"), "-o", out,
		filepath.Join(repo, "crates", "guatiao", "tests", "c_consumer", source))
	if text, err := exec.Command(cc, args...).CombinedOutput(); err != nil {
		return "", fmt.Errorf("%s: %v\n%s", cc, err, text)
	}
	return out, nil
}

// CLib is a C library opened for its plain functions.
type CLib struct{ h unsafe.Pointer }

// OpenC opens the library at path.
func OpenC(path string) (*CLib, error) {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	h := C.te_open(cpath)
	if h == nil {
		return nil, fmt.Errorf("%s does not open", path)
	}
	return &CLib{h: h}, nil
}

func (l *CLib) sym(name string) unsafe.Pointer {
	cname := C.CString(name)
	defer C.free(unsafe.Pointer(cname))
	fn := C.te_sym(l.h, cname)
	if fn == nil {
		panic("no " + name)
	}
	return fn
}

// VoidInt calls a `void name(int)`.
func (l *CLib) VoidInt(name string, i int) { C.te_void_int(l.sym(name), C.int(i)) }

// VoidBool calls a `void name(bool)`.
func (l *CLib) VoidBool(name string, b bool) { C.te_void_bool(l.sym(name), C.bool(b)) }

// IntInt calls an `int name(int)`.
func (l *CLib) IntInt(name string, i int) int { return int(C.te_int_int(l.sym(name), C.int(i))) }
