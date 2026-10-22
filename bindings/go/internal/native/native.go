// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package native is the binding's one door to C: finding and opening the
// guatiao libraries, a Go function per export, and reading and writing
// the structs the header declares.
//
// Every size and offset comes from the header's own types, so nothing
// here restates a layout. Nodes, allocators and descriptors cross as
// unsafe.Pointer values into C memory; a Go string or byte slice is only
// ever passed for the duration of one call.
package native

/*
#cgo CFLAGS: -I${SRCDIR}/include -DGUATIAO_SERDE_TOML -DGUATIAO_SERDE_YAML
#cgo linux LDFLAGS: -ldl
#include <stdlib.h>
#include "native.h"
*/
import "C"

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"unsafe"
)

// Layout, from the header's types.
const (
	SizeValue       = unsafe.Sizeof(C.guatiao_value{})
	SizeEntry       = unsafe.Sizeof(C.guatiao_entry{})
	SizeString      = unsafe.Sizeof(C.guatiao_string{})
	SizeMap         = unsafe.Sizeof(C.guatiao_map{})
	SizeList        = unsafe.Sizeof(C.guatiao_list{})
	SizeStr         = unsafe.Sizeof(C.guatiao_str{})
	SizeAlloc       = unsafe.Sizeof(C.guatiao_alloc{})
	SizeProviderErr = unsafe.Sizeof(C.guatiao_provider_error{})
	SizeObject      = unsafe.Sizeof(C.guatiao_object{})
	SizeKindHeader  = unsafe.Sizeof(C.guatiao_kind_header{})
	SizeParseError  = unsafe.Sizeof(C.guatiao_parse_error{})
	SizeTextError   = unsafe.Sizeof(C.guatiao_text_error{})
	OffsetPayload   = unsafe.Offsetof(C.guatiao_value{}.payload)
	OffsetEntryVal  = unsafe.Offsetof(C.guatiao_entry{}.value)
	Word            = unsafe.Sizeof(uintptr(0))
	ABIVersion      = C.GUATIAO_ABI_VERSION
)

// The status codes and tags, as the header defines them.
const (
	OK            = uint32(C.GUATIAO_OK)
	ErrBadValue   = uint32(C.GUATIAO_ERR_BAD_VALUE)
	ErrAlloc      = uint32(C.GUATIAO_ERR_ALLOC)
	ErrWrongKind  = uint32(C.GUATIAO_ERR_WRONG_KIND)
	ErrNotFound   = uint32(C.GUATIAO_ERR_NOT_FOUND)
	ErrNull       = uint32(C.GUATIAO_ERR_NULL)
	ErrGone       = uint32(C.GUATIAO_ERR_GONE)
	ErrUnsupport  = uint32(C.GUATIAO_ERR_UNSUPPORTED)
	ErrBusy       = uint32(C.GUATIAO_ERR_BUSY)
	ErrInternal   = uint32(C.GUATIAO_ERR_INTERNAL)
	TagAbsent     = uint32(C.GUATIAO_ABSENT)
	TagNull       = uint32(C.GUATIAO_NULL)
	TagBool       = uint32(C.GUATIAO_BOOL)
	TagNumber     = uint32(C.GUATIAO_NUMBER)
	TagString     = uint32(C.GUATIAO_STRING)
	TagBytes      = uint32(C.GUATIAO_BYTES)
	TagList       = uint32(C.GUATIAO_LIST)
	TagMap        = uint32(C.GUATIAO_MAP)
	FormatJSON    = uint32(C.GUATIAO_FORMAT_JSON)
	FormatTOML    = uint32(C.GUATIAO_FORMAT_TOML)
	FormatYAML    = uint32(C.GUATIAO_FORMAT_YAML)
	BytesDataURI  = uint32(C.GUATIAO_BYTES_DATA_URI)
	BytesBase64   = uint32(C.GUATIAO_BYTES_BASE64)
	BytesArray    = uint32(C.GUATIAO_BYTES_ARRAY)
	BytesRefuse   = uint32(C.GUATIAO_BYTES_REFUSE)
	ReadDataURIs  = uint32(C.GUATIAO_READ_DATA_URIS)
	Pretty        = uint32(C.GUATIAO_PRETTY)
	MaxDepth      = int(C.GUATIAO_MAX_DEPTH)
	LowUnmapped   = uintptr(0x10000)
	ContextStride = uintptr(16)
)

// ---- finding the libraries ------------------------------------------------

// NotFound says no file for a library opened, and where each route looked.
type NotFound struct {
	Basename string
	Looked   []string
}

func (e *NotFound) Error() string {
	return fmt.Sprintf("could not find the %s library: looked in %v", e.Basename, e.Looked)
}

// Missing says an export the loaded build lacks.
type Missing struct {
	Path, Symbol, Feature string
}

func (e *Missing) Error() string {
	if e.Feature != "" {
		return fmt.Sprintf("%s was not built with %s (feature %q)", e.Path, e.Symbol, e.Feature)
	}
	return fmt.Sprintf("%s was not built with %s", e.Path, e.Symbol)
}

// Lib is one opened library: where it came from, its default allocator
// and the optional exports it lacks.
type Lib struct {
	Path    string
	which   C.int
	alloc   *C.guatiao_alloc
	missing map[string]string
}

// Alloc is the library's default allocator, filled once and kept for the
// process: every tree built in it records this address.
func (l *Lib) Alloc() unsafe.Pointer { return unsafe.Pointer(l.alloc) }

// Need answers a Missing for an export this build lacks, else nil.
func (l *Lib) Need(symbol string) error {
	if feature, gone := l.missing[symbol]; gone {
		return &Missing{Path: l.Path, Symbol: symbol, Feature: feature}
	}
	return nil
}

var (
	mu      sync.Mutex
	chosen  [3]string
	libs    [3]*Lib
	failure [3]error
)

var basenames = [3]string{"guatiao", "guatiao_serde", "guatiao_intake"}

// Each surface's own environment variable, consulted before
// GUATIAO_LIBRARY; the core has none of its own.
var variables = [3]string{"", "GUATIAO_SERDE_LIBRARY", "GUATIAO_INTAKE_LIBRARY"}

// A symbol only each surface exports: what a shared file is checked for.
var probes = [3]string{"guatiao_value_free", "guatiao_json_parse_ex", "guatiao_intake_check"}

// The surfaces, as the which of Use and open.
const (
	LibCore   = 0
	LibSerde  = 1
	LibIntake = 2
)

// Filename is the platform's file name for a library of this base name.
func Filename(base string) string {
	switch runtime.GOOS {
	case "windows":
		return base + ".dll"
	case "darwin":
		return "lib" + base + ".dylib"
	default:
		return "lib" + base + ".so"
	}
}

// Use names the library of surface which (LibCore, LibSerde, LibIntake)
// to open instead of looking for one: a file exporting it, whatever it is
// called, or a directory holding it. Naming another once one is open is
// an error; naming the same one is not.
func Use(which int, where string) error {
	mu.Lock()
	defer mu.Unlock()
	if libs[which] != nil {
		same := where
		if isDir(where) {
			same = filepath.Join(where, Filename(basenames[which]))
		}
		a, _ := filepath.Abs(same)
		b, _ := filepath.Abs(libs[which].Path)
		if a != b {
			return fmt.Errorf("the %s library is already loaded from %s", basenames[which], libs[which].Path)
		}
		return nil
	}
	chosen[which] = where
	return nil
}

func isDir(p string) bool {
	info, err := os.Stat(p)
	return err == nil && info.IsDir()
}

func isFile(p string) bool {
	info, err := os.Stat(p)
	return err == nil && !info.IsDir()
}

// A file to try. A shared one (a GUATIAO_LIBRARY file, for a surface other
// than the core) that lacks the surface is passed over, not an error.
type try struct {
	path   string
	shared bool
}

// candidates are the files to try for surface which: what Use named, its
// own variable, GUATIAO_LIBRARY, then the platform's search. A directory
// holds the surface's file under its platform name; a file named for the
// surface is taken as it is; a GUATIAO_LIBRARY file serves every surface
// it exports, else the surface's own file beside it does.
func candidates(which int) (tries []try, looked []string) {
	filename := Filename(basenames[which])
	named := func(where, route string, shared bool) {
		looked = append(looked, fmt.Sprintf("%s (%s)", route, where))
		switch {
		case isDir(where):
			if c := filepath.Join(where, filename); isFile(c) {
				tries = append(tries, try{path: c})
			}
		case isFile(where):
			tries = append(tries, try{path: where, shared: shared})
			if shared {
				if c := filepath.Join(filepath.Dir(where), filename); isFile(c) {
					tries = append(tries, try{path: c})
				}
			}
		}
	}
	if chosen[which] != "" {
		named(chosen[which], "UseLibrary", false)
	}
	if v := variables[which]; v != "" {
		if own := os.Getenv(v); own != "" {
			named(own, v, false)
		} else {
			looked = append(looked, v+" (unset)")
		}
	}
	if shared := os.Getenv("GUATIAO_LIBRARY"); shared != "" {
		named(shared, "GUATIAO_LIBRARY", which != LibCore)
	} else {
		looked = append(looked, "GUATIAO_LIBRARY (unset)")
	}
	tries = append(tries, try{path: filename})
	looked = append(looked, "the platform's search for "+filename)
	return tries, looked
}

// Locate is the file the lookup for surface which settles on now, without
// opening it for use: the first candidate that is a file (a shared one
// only when it exports the surface), else the bare file name the
// platform's search would be given. A *NotFound when no route names
// anything and the platform's search is all that is left is not an error
// here: the bare name is answered.
func Locate(which int) (string, error) {
	mu.Lock()
	defer mu.Unlock()
	tries, looked := candidates(which)
	for _, t := range tries {
		if !isFile(t.path) {
			continue
		}
		if t.shared {
			cpath, csym := C.CString(t.path), C.CString(probes[which])
			found := C.gt_carries(cpath, csym) != 0
			C.free(unsafe.Pointer(cpath))
			C.free(unsafe.Pointer(csym))
			if !found {
				continue
			}
		}
		return t.path, nil
	}
	return "", &NotFound{Basename: basenames[which], Looked: looked}
}

func open(which int) (*Lib, error) {
	mu.Lock()
	defer mu.Unlock()
	if libs[which] != nil {
		return libs[which], nil
	}
	if failure[which] != nil {
		return nil, failure[which]
	}
	base := basenames[which]
	tries, looked := candidates(which)
	for _, t := range tries {
		path := t.path
		cpath := C.CString(path)
		var buf [512]C.char
		rc := C.gt_open(C.int(which), cpath, &buf[0], C.size_t(len(buf)))
		C.free(unsafe.Pointer(cpath))
		why := C.GoString(&buf[0])
		if rc == 2 {
			if t.shared {
				continue
			}
			// A file that opened but is not a build of this library.
			return nil, &Missing{Path: path, Symbol: why}
		}
		if rc != 0 {
			continue
		}
		lib := &Lib{Path: path, which: C.int(which), missing: map[string]string{}}
		for i := 0; ; i++ {
			var name, feature *C.char
			if C.gt_missing(C.int(which), C.int(i), &name, &feature) == 0 {
				break
			}
			lib.missing[C.GoString(name)] = C.GoString(feature)
		}
		alloc := (*C.guatiao_alloc)(C.calloc(1, C.size_t(SizeAlloc)))
		var st C.guatiao_status
		switch which {
		case 0:
			st = C.gt_core_guatiao_alloc_default(alloc)
		case 1:
			st = C.gt_serde_guatiao_alloc_default(alloc)
		default:
			st = C.gt_intake_guatiao_alloc_default(alloc)
		}
		if st != 0 {
			C.free(unsafe.Pointer(alloc))
			return nil, fmt.Errorf("guatiao_alloc_default of %s answered status %d", path, uint32(st))
		}
		lib.alloc = alloc
		libs[which] = lib
		return lib, nil
	}
	err := &NotFound{Basename: base, Looked: looked}
	failure[which] = err
	return nil, err
}

// Core opens the guatiao library on first use.
func Core() (*Lib, error) { return open(0) }

// Serde opens guatiao_serde on first use.
func Serde() (*Lib, error) { return open(1) }

// Intake opens guatiao_intake on first use.
func Intake() (*Lib, error) { return open(2) }

// ---- memory -------------------------------------------------------------

// Calloc answers n zeroed bytes of C memory; it panics when the C heap is
// out, as a Go allocation does.
func Calloc(n uintptr) unsafe.Pointer {
	if n == 0 {
		n = 1
	}
	p := C.calloc(1, C.size_t(n))
	if p == nil {
		panic("guatiao: the C heap is out of memory")
	}
	return p
}

// Free releases memory Calloc answered.
func Free(p unsafe.Pointer) {
	if p != nil {
		C.free(p)
	}
}

// Copy copies n bytes from src to dst, which must not overlap.
func Copy(dst, src unsafe.Pointer, n uintptr) {
	if n == 0 {
		return
	}
	copy(unsafe.Slice((*byte)(dst), n), unsafe.Slice((*byte)(src), n))
}

// Zero clears n bytes at p.
func Zero(p unsafe.Pointer, n uintptr) {
	clear(unsafe.Slice((*byte)(p), n))
}

// Bytes copies n bytes at p into Go memory.
func Bytes(p unsafe.Pointer, n uintptr) []byte {
	if n == 0 || p == nil {
		return []byte{}
	}
	return append([]byte(nil), unsafe.Slice((*byte)(p), n)...)
}

// Text copies n bytes at p into a Go string.
func Text(p unsafe.Pointer, n uintptr) string {
	if n == 0 || p == nil {
		return ""
	}
	return string(unsafe.Slice((*byte)(p), n))
}

// SameBytes reports whether the n bytes at p are s.
func SameBytes(p unsafe.Pointer, n uintptr, s string) bool {
	if n != uintptr(len(s)) {
		return false
	}
	if n == 0 {
		return true
	}
	return string(unsafe.Slice((*byte)(p), n)) == s
}

// str is `s` as a guatiao_str for one call: the bytes are the Go string's.
func str(s string) C.guatiao_str {
	if len(s) == 0 {
		return C.guatiao_str{}
	}
	return C.guatiao_str{ptr: (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s))), len: C.size_t(len(s))}
}

func bytesArg(b []byte) C.guatiao_bytes {
	if len(b) == 0 {
		return C.guatiao_bytes{}
	}
	return C.guatiao_bytes{ptr: (*C.uint8_t)(unsafe.Pointer(unsafe.SliceData(b))), len: C.size_t(len(b))}
}

// ---- reading nodes ----------------------------------------------------------

func val(p unsafe.Pointer) *C.guatiao_value { return (*C.guatiao_value)(p) }

// Tag is the node's tag; absent for a nil node.
func Tag(node unsafe.Pointer) uint32 {
	if node == nil {
		return TagAbsent
	}
	return uint32(val(node).tag)
}

// SetTag writes a node's tag.
func SetTag(node unsafe.Pointer, tag uint32) { val(node).tag = C.uint32_t(tag) }

// Payload is the address of a node's payload.
func Payload(node unsafe.Pointer) unsafe.Pointer { return unsafe.Add(node, OffsetPayload) }

func owned(p unsafe.Pointer) *C.guatiao_string { return (*C.guatiao_string)(p) }

// OwnedPtr, OwnedLen, OwnedCap and OwnedAlloc read the {ptr, len, cap,
// alloc} every owned container (text, bytes, list, map) starts with.
func OwnedPtr(p unsafe.Pointer) unsafe.Pointer { return unsafe.Pointer(owned(p).ptr) }
func OwnedLen(p unsafe.Pointer) uintptr        { return uintptr(owned(p).len) }
func OwnedCap(p unsafe.Pointer) uintptr        { return uintptr(owned(p).cap) }
func OwnedAlloc(p unsafe.Pointer) unsafe.Pointer {
	return unsafe.Pointer(owned(p).alloc)
}

// SetOwnedAlloc writes the allocator field of an owned container, as a
// carrier names the allocator an answer is built in.
func SetOwnedAlloc(p, alloc unsafe.Pointer) { owned(p).alloc = (*C.guatiao_alloc)(alloc) }

// BoolOf reads a BOOL node's payload.
func BoolOf(node unsafe.Pointer) bool { return bool(*(*C.bool)(Payload(node))) }

// Len is the length of a node's text, bytes, list or map.
func Len(node unsafe.Pointer) uintptr { return OwnedLen(Payload(node)) }

// TextOf copies a STRING or NUMBER node's text.
func TextOf(node unsafe.Pointer) string {
	at := Payload(node)
	return Text(OwnedPtr(at), OwnedLen(at))
}

// ListElem is the address of a LIST node's element i.
func ListElem(node unsafe.Pointer, i uintptr) unsafe.Pointer {
	return unsafe.Add(OwnedPtr(Payload(node)), i*SizeValue)
}

// EntryAt is the address of a MAP node's entry i.
func EntryAt(node unsafe.Pointer, i uintptr) unsafe.Pointer {
	return unsafe.Add(OwnedPtr(Payload(node)), i*SizeEntry)
}

// EntryKey copies an entry's key.
func EntryKey(entry unsafe.Pointer) string {
	return Text(OwnedPtr(entry), OwnedLen(entry))
}

// EntryValue is the address of an entry's value.
func EntryValue(entry unsafe.Pointer) unsafe.Pointer { return unsafe.Add(entry, OffsetEntryVal) }

// Find is the address of the value under key in a MAP node, or nil.
func Find(node unsafe.Pointer, key string) unsafe.Pointer {
	if Tag(node) != TagMap {
		return nil
	}
	n := Len(node)
	for i := uintptr(0); i < n; i++ {
		e := EntryAt(node, i)
		if SameBytes(OwnedPtr(e), OwnedLen(e), key) {
			return EntryValue(e)
		}
	}
	return nil
}

// Complete reports whether alloc names an allocator that can build.
func Complete(alloc unsafe.Pointer) bool {
	if alloc == nil {
		return false
	}
	a := (*C.guatiao_alloc)(alloc)
	floor := C.uint32_t(unsafe.Offsetof(C.guatiao_alloc{}.free) + Word)
	return a.struct_size >= floor && a.alloc != nil && a.free != nil
}

// Recorded is the allocator a container node records, when it is complete.
func Recorded(node unsafe.Pointer) unsafe.Pointer {
	t := Tag(node)
	if t >= TagNumber && t <= TagMap {
		if a := OwnedAlloc(Payload(node)); Complete(a) {
			return a
		}
	}
	return nil
}

// ---- the core exports ---------------------------------------------------------

func n(p unsafe.Pointer) *C.guatiao_value { return (*C.guatiao_value)(p) }
func a(p unsafe.Pointer) *C.guatiao_alloc { return (*C.guatiao_alloc)(p) }

// ValueFree frees what the node owns. Before the core library is open no
// tree can own anything, and it does nothing.
func ValueFree(node unsafe.Pointer) uint32 {
	if C.gt_core.guatiao_value_free == nil {
		return OK
	}
	return uint32(C.gt_core_guatiao_value_free(n(node)))
}
func ValueClone(alloc, src, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_clone(a(alloc), n(src), n(out)))
}
func ValueNull(out unsafe.Pointer) uint32   { return uint32(C.gt_core_guatiao_value_null(n(out))) }
func ValueAbsent(out unsafe.Pointer) uint32 { return uint32(C.gt_core_guatiao_value_absent(n(out))) }
func ValueBool(b bool, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_bool(C.bool(b), n(out)))
}
func ValueMap(alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_map(a(alloc), n(out)))
}
func ValueList(alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_list(a(alloc), n(out)))
}
func ValueString(alloc unsafe.Pointer, text string, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_string(a(alloc), str(text), n(out)))
}
func ValueNumber(alloc unsafe.Pointer, text string, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_number(a(alloc), str(text), n(out)))
}
func ValueBytes(alloc unsafe.Pointer, data []byte, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_bytes(a(alloc), bytesArg(data), n(out)))
}
func MapSet(alloc, node unsafe.Pointer, key string, value unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_map_set(a(alloc), n(node), str(key), n(value)))
}
func MapDiscard(node unsafe.Pointer, key string) uint32 {
	return uint32(C.gt_core_guatiao_map_discard(n(node), str(key)))
}
func MapClear(node unsafe.Pointer) uint32 { return uint32(C.gt_core_guatiao_map_clear(n(node))) }
func MapCopyFrom(alloc, dst, src unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_map_copy_from(a(alloc), n(dst), n(src)))
}
func ListPush(alloc, node, value unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_list_push(a(alloc), n(node), n(value)))
}
func ListDiscard(node unsafe.Pointer, i uintptr) uint32 {
	return uint32(C.gt_core_guatiao_list_discard(n(node), C.size_t(i)))
}
func ListClear(node unsafe.Pointer) uint32 { return uint32(C.gt_core_guatiao_list_clear(n(node))) }
func SchemaRedact(schema, value unsafe.Pointer, placeholder string, alloc, out unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_schema_redact(n(schema), n(value), str(placeholder), a(alloc), n(out)))
}

// Loaded reports whether the core library's exports are resolved.
func Loaded() bool { return C.gt_core.guatiao_value_free != nil }
