// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package native is the binding's one door to C: finding and opening the
// guatiao library and any other a package names, a Go function per export, and reading and writing
// the structs the header declares.
//
// Every size and offset comes from the header's own types, so nothing
// here restates a layout. Nodes, allocators and descriptors cross as
// unsafe.Pointer values into C memory; a Go string or byte slice is only
// ever passed for the duration of one call.
package native

/*
#cgo CFLAGS: -I${SRCDIR}/include
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

// Lib is the opened guatiao library: where it came from, its default
// allocator and the optional exports it lacks.
type Lib struct {
	Path    string
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

// Spec names a library to look for.
type Spec struct {
	// Base is the file name without the platform's prefix and suffix.
	Base string
	// Variable is its own environment variable, consulted before
	// GUATIAO_LIBRARY; "" for none.
	Variable string
	// Probe is an export only this library has: a file GUATIAO_LIBRARY
	// names that lacks it is passed over for the file beside it. "" takes
	// that file as it is, as for the core, which every library carries.
	Probe string
}

var coreSpec = Spec{Base: "guatiao"}

var (
	mu         sync.Mutex
	coreChosen string
	coreLib    *Lib
	coreFailed error
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

// Same reports whether where (a file, or a directory holding the
// library s names) is the file at path.
func Same(s Spec, where, path string) bool {
	if isDir(where) {
		where = filepath.Join(where, Filename(s.Base))
	}
	a, _ := filepath.Abs(where)
	b, _ := filepath.Abs(path)
	return a == b
}

// Use names the guatiao library to open instead of looking for one: a
// file exporting it, whatever it is called, or a directory holding it.
// Naming another once one is open is an error; naming the same one is not.
func Use(where string) error {
	mu.Lock()
	defer mu.Unlock()
	if coreLib != nil {
		if !Same(coreSpec, where, coreLib.Path) {
			return fmt.Errorf("the guatiao library is already loaded from %s", coreLib.Path)
		}
		return nil
	}
	coreChosen = where
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

// A file to try. A shared one (a GUATIAO_LIBRARY file, for a library with
// a probe) that lacks the library is passed over, not an error.
type try struct {
	path   string
	shared bool
}

// candidates are the files to try for s: chosen (what a Use named), its
// own variable, GUATIAO_LIBRARY, then the platform's search. A directory
// holds the library under its platform name; a file named for it is taken
// as it is; a GUATIAO_LIBRARY file serves every library it exports, else
// the library's own file beside it does.
func candidates(s Spec, chosen string) (tries []try, looked []string) {
	filename := Filename(s.Base)
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
	if chosen != "" {
		named(chosen, "UseLibrary", false)
	}
	if s.Variable != "" {
		if own := os.Getenv(s.Variable); own != "" {
			named(own, s.Variable, false)
		} else {
			looked = append(looked, s.Variable+" (unset)")
		}
	}
	if shared := os.Getenv("GUATIAO_LIBRARY"); shared != "" {
		named(shared, "GUATIAO_LIBRARY", s.Probe != "")
	} else {
		looked = append(looked, "GUATIAO_LIBRARY (unset)")
	}
	tries = append(tries, try{path: filename})
	looked = append(looked, "the platform's search for "+filename)
	return tries, looked
}

func carries(path, symbol string) bool {
	cpath, csym := C.CString(path), C.CString(symbol)
	defer C.free(unsafe.Pointer(cpath))
	defer C.free(unsafe.Pointer(csym))
	return C.gt_carries(cpath, csym) != 0
}

// Locate is the file the lookup for s settles on now, with chosen as a Use
// named it, without opening it for use: the first candidate that is a
// file (a shared one only when it exports s.Probe). A *NotFound when no
// route names a file and the platform's search is all that is left.
func Locate(s Spec, chosen string) (string, error) {
	tries, looked := candidates(s, chosen)
	for _, t := range tries {
		if !isFile(t.path) {
			continue
		}
		if t.shared && !carries(t.path, s.Probe) {
			continue
		}
		return t.path, nil
	}
	return "", &NotFound{Basename: s.Base, Looked: looked}
}

// Opened is a library other than the core, open for the process: the
// address of each export resolved, and the optional ones it lacks.
type Opened struct {
	Path    string
	Symbols map[string]unsafe.Pointer
	Missing map[string]string
	Alloc   unsafe.Pointer
}

// Open finds the library s names, with chosen as a Use named it, and
// resolves exports (each name and its feature: "" for one every build
// has) and guatiao_alloc_default. A file that opens but lacks a required
// export is a *Missing, or passed over when it is a shared one.
func Open(s Spec, chosen string, exports map[string]string) (*Opened, error) {
	tries, looked := candidates(s, chosen)
	for _, t := range tries {
		cpath := C.CString(t.path)
		var buf [512]C.char
		h := C.gt_load(cpath, &buf[0], C.size_t(len(buf)))
		C.free(unsafe.Pointer(cpath))
		if h == nil {
			continue
		}
		o := &Opened{Path: t.path, Symbols: map[string]unsafe.Pointer{}, Missing: map[string]string{}}
		absent := ""
		find := func(name string) unsafe.Pointer {
			cname := C.CString(name)
			defer C.free(unsafe.Pointer(cname))
			return C.gt_find(h, cname)
		}
		for name, feature := range exports {
			if p := find(name); p != nil {
				o.Symbols[name] = p
			} else if feature == "" {
				absent = name
				break
			} else {
				o.Missing[name] = feature
			}
		}
		defaults := find("guatiao_alloc_default")
		if absent == "" && defaults == nil {
			absent = "guatiao_alloc_default"
		}
		if absent != "" {
			C.gt_unload(h)
			if t.shared {
				continue
			}
			// A file that opened but is not a build of this library.
			return nil, &Missing{Path: t.path, Symbol: absent}
		}
		alloc := (*C.guatiao_alloc)(C.calloc(1, C.size_t(SizeAlloc)))
		if st := C.gt_alloc_default_at(defaults, alloc); st != 0 {
			C.free(unsafe.Pointer(alloc))
			return nil, fmt.Errorf("guatiao_alloc_default of %s answered status %d", t.path, uint32(st))
		}
		o.Alloc = unsafe.Pointer(alloc)
		return o, nil
	}
	return nil, &NotFound{Basename: s.Base, Looked: looked}
}

// Core opens the guatiao library on first use.
func Core() (*Lib, error) {
	mu.Lock()
	defer mu.Unlock()
	if coreLib != nil {
		return coreLib, nil
	}
	if coreFailed != nil {
		return nil, coreFailed
	}
	tries, looked := candidates(coreSpec, coreChosen)
	for _, t := range tries {
		path := t.path
		cpath := C.CString(path)
		var buf [512]C.char
		rc := C.gt_open(cpath, &buf[0], C.size_t(len(buf)))
		C.free(unsafe.Pointer(cpath))
		why := C.GoString(&buf[0])
		if rc == 2 {
			// A file that opened but is not a build of this library.
			return nil, &Missing{Path: path, Symbol: why}
		}
		if rc != 0 {
			continue
		}
		lib := &Lib{Path: path, missing: map[string]string{}}
		for i := 0; ; i++ {
			var name, feature *C.char
			if C.gt_missing(C.int(i), &name, &feature) == 0 {
				break
			}
			lib.missing[C.GoString(name)] = C.GoString(feature)
		}
		alloc := (*C.guatiao_alloc)(C.calloc(1, C.size_t(SizeAlloc)))
		if st := C.gt_core_guatiao_alloc_default(alloc); st != 0 {
			C.free(unsafe.Pointer(alloc))
			return nil, fmt.Errorf("guatiao_alloc_default of %s answered status %d", path, uint32(st))
		}
		lib.alloc = alloc
		coreLib = lib
		return lib, nil
	}
	coreFailed = &NotFound{Basename: coreSpec.Base, Looked: looked}
	return nil, coreFailed
}

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
	return a.struct_size >= C.uint32_t(unsafe.Sizeof(C.guatiao_alloc{})) && a.alloc != nil && a.free != nil
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

// ValueMakeLocal moves the tree at node where it stands into alloc, so it
// outlives the library whose allocator built it.
func ValueMakeLocal(node, alloc unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_value_make_local(n(node), a(alloc)))
}

// MapSettleLent settles a map lent to a slot into home, as ValueMakeLocal
// moves a tree, and removes any key the slot wrote that is not UTF-8
// (ErrBadValue then).
func MapSettleLent(node, home unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_map_settle_lent(n(node), a(home)))
}

// KindFloorHash is the floor hash of the kind whose description kind
// holds as JSON, or the status and, in why, the reason it is not one.
func KindFloorHash(kind string, alloc unsafe.Pointer, out *uint32, why unsafe.Pointer) uint32 {
	return uint32(C.gt_core_guatiao_kind_floor_hash(str(kind), a(alloc), (*C.uint32_t)(unsafe.Pointer(out)), n(why)))
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
