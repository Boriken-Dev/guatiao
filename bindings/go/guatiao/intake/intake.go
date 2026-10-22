// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package intake is taking a value in from a person, through the
// guatiao_intake library: checking a form against its schema, laying a
// schema out in sections, conditional visibility, the path grammar
// (`agent[1].name`), and the flat `key -> text` projection a front end
// stores.
//
// Every schema, form, value and store argument is a *guatiao.Value, a
// view, or plain data guatiao.NewValue takes.
package intake

import (
	"errors"
	"fmt"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

func lib() (*native.Lib, error) {
	if err := guatiao.Load(); err != nil {
		return nil, err
	}
	l, err := native.Intake()
	if err != nil {
		return nil, lift(err)
	}
	return l, nil
}

// answer runs call with a fresh out node and hands it to read as a Value
// it owns.
func answer(call func(alloc, out unsafe.Pointer) uint32, read func(status uint32, out *guatiao.Value) error) error {
	l, err := lib()
	if err != nil {
		return err
	}
	out := native.Calloc(native.SizeValue)
	st := call(l.Alloc(), out)
	return read(st, guatiao.Adopt(out, l.Alloc()))
}

// plain decodes an answer into dst and frees it.
func plain(dst any) func(uint32, *guatiao.Value) error {
	return func(st uint32, out *guatiao.Value) error {
		defer out.Close()
		if err := guatiao.StatusError(st, ""); err != nil {
			return err
		}
		return out.Decode(dst)
	}
}

// owned hands an answer over as a Value to close; ok is false and the
// Value nil when it is absent.
func owned(dst **guatiao.Value) func(uint32, *guatiao.Value) error {
	return func(st uint32, out *guatiao.Value) error {
		if err := guatiao.StatusError(st, ""); err != nil {
			out.Close()
			return err
		}
		if out.IsAbsent() {
			out.Close()
			return nil
		}
		*dst = out
		return nil
	}
}

// Check is nil when form fits schema, else the reason as a map: `kind`
// (`malformed`, `unknown_field`, `duplicate_section`,
// `condition_refused` or `cyclic_condition`), `at`, whichever of `path`,
// `id`, `field` and `expected` apply, and a `message`.
func Check(schema, form any) (map[string]any, error) {
	var why map[string]any
	err := guatiao.WithNodes([]any{schema, form}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeCheck(n[0], n[1], alloc, out)
		}, func(st uint32, out *guatiao.Value) error {
			defer out.Close()
			if st == native.OK {
				return nil
			}
			if st != native.ErrBadValue {
				return guatiao.StatusError(st, "")
			}
			return out.Decode(&why)
		})
	})
	return why, err
}

// Section is one group of Layout: the form's section (nil for the
// default one) and its fields' keys in order.
type Section struct {
	Section map[string]any `guatiao:"section"`
	Fields  []string       `guatiao:"fields"`
}

// Layout is the schema's fields grouped into sections and put in order.
func Layout(schema, form any) ([]Section, error) {
	var sections []Section
	err := guatiao.WithNodes([]any{schema, form}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeLayout(n[0], n[1], alloc, out)
		}, plain(&sections))
	})
	return sections, err
}

// IsVisible reports whether the field under key is shown, given the
// values entered so far. guatiao.ErrNotFound when key, or a key a
// condition reads, names no field.
func IsVisible(schema, form any, key string, values any) (bool, error) {
	var shown bool
	err := guatiao.WithNodes([]any{schema, form, values}, func(n []unsafe.Pointer) error {
		if _, err := lib(); err != nil {
			return err
		}
		ok, st := native.IntakeIsVisible(n[0], n[1], key, n[2])
		shown = ok
		return guatiao.StatusError(st, "")
	})
	return shown, err
}

// borrowedAt is a read-only Ref over what find answers inside source,
// found again on every access; false when it answers nothing now.
func borrowedAt(source any, find func(node unsafe.Pointer) unsafe.Pointer) (guatiao.Ref, bool, error) {
	if _, err := lib(); err != nil {
		return guatiao.Ref{}, false, err
	}
	node, ok, err := guatiao.NodeOf(source)
	if !ok {
		return guatiao.Ref{}, false, fmt.Errorf("guatiao: %T is not a value; pass a *guatiao.Value or a view", source)
	}
	if err != nil {
		return guatiao.Ref{}, false, err
	}
	if find(node) == nil {
		return guatiao.Ref{}, false, nil
	}
	again := func() unsafe.Pointer {
		node, _, err := guatiao.NodeOf(source)
		if err != nil || node == nil {
			return nil
		}
		return find(node)
	}
	ref, err := guatiao.Borrow(again, source)
	if err != nil {
		return guatiao.Ref{}, false, err
	}
	return ref, true, nil
}

// At is the value at path inside value (a *guatiao.Value or a view),
// borrowed from it: read-only, found again on every access, and valid
// while value is. false when the path names nothing.
func At(value any, path string) (guatiao.Ref, bool, error) {
	return borrowedAt(value, func(node unsafe.Pointer) unsafe.Pointer { return native.IntakePathGet(node, path) })
}

// Resolve is the field of schema (a *guatiao.Value or a view) governing
// a flat key, borrowed as At borrows; false for none.
func Resolve(schema any, key string) (guatiao.Ref, bool, error) {
	return borrowedAt(schema, func(node unsafe.Pointer) unsafe.Pointer { return native.IntakeResolve(node, key) })
}

// ForSchema is the form a schema implies, a Value to close.
func ForSchema(schema any) (*guatiao.Value, error) {
	var form *guatiao.Value
	err := guatiao.WithNodes([]any{schema}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeForSchema(n[0], alloc, out)
		}, owned(&form))
	})
	return form, err
}

// FormFor is the form to show the field at path with, a Value to close,
// or nil when there is none.
func FormFor(form, schema any, path string) (*guatiao.Value, error) {
	var found *guatiao.Value
	err := guatiao.WithNodes([]any{form, schema}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeFormFor(n[0], n[1], path, alloc, out)
		}, owned(&found))
	})
	return found, err
}

// FlatKeys are the flat keys the field under key projects onto.
// guatiao.ErrWrongKind for a key that names no field.
func FlatKeys(schema any, key string) ([]string, error) {
	var keys []string
	err := guatiao.WithNodes([]any{schema}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeFlatKeys(n[0], key, alloc, out)
		}, plain(&keys))
	})
	return keys, err
}

// Flatten writes value into a flat store, one entry per scalar leaf.
func Flatten(schema any, key string, value any) (map[string]string, error) {
	store := map[string]string{}
	err := guatiao.WithNodes([]any{schema, value}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeFlatten(n[0], key, n[1], alloc, out)
		}, plain(&store))
	})
	return store, err
}

// Unflatten rebuilds the field under key from a flat store (a
// map[string]string, or a view of a map of strings), as a Value to
// close; nil when nothing is stored under it. A store holding anything
// but strings is an error.
func Unflatten(schema any, key string, store any) (*guatiao.Value, error) {
	if node, ok, err := guatiao.NodeOf(store); ok {
		if err != nil {
			return nil, err
		}
		if !textStore(node) {
			return nil, &guatiao.Error{Status: guatiao.WrongKind, Message: "a flat store maps strings to strings"}
		}
	} else if _, ok := store.(map[string]string); !ok {
		return nil, &guatiao.Error{Status: guatiao.WrongKind, Message: fmt.Sprintf("a flat store maps strings to strings, not a %T", store)}
	}
	var rebuilt *guatiao.Value
	err := guatiao.WithNodes([]any{schema, store}, func(n []unsafe.Pointer) error {
		return answer(func(alloc, out unsafe.Pointer) uint32 {
			return native.IntakeUnflatten(n[0], key, n[1], alloc, out)
		}, func(st uint32, out *guatiao.Value) error {
			if st == native.ErrWrongKind {
				out.Close()
				return nil
			}
			return owned(&rebuilt)(st, out)
		})
	})
	return rebuilt, err
}

func textStore(node unsafe.Pointer) bool {
	if native.Tag(node) != native.TagMap {
		return false
	}
	for i := uintptr(0); i < native.Len(node); i++ {
		if native.Tag(native.EntryValue(native.EntryAt(node, i))) != native.TagString {
			return false
		}
	}
	return true
}

// lift turns the native layer's failure to open a library into the
// guatiao package's error for it.
func lift(err error) error {
	var nf *native.NotFound
	if errors.As(err, &nf) {
		return &guatiao.LibraryNotFound{Basename: nf.Basename, Looked: nf.Looked}
	}
	var ms *native.Missing
	if errors.As(err, &ms) {
		return &guatiao.MissingSymbol{Path: ms.Path, Symbol: ms.Symbol, Feature: ms.Feature}
	}
	return err
}

// UseLibrary names the guatiao_intake library to load instead of looking for
// one: a file exporting its functions, whatever it is called (an
// application's own library carrying several surfaces, say), or a
// directory holding it under its platform file name. Without it the
// library is found through GUATIAO_INTAKE_LIBRARY, then GUATIAO_LIBRARY (a
// directory, or a file exporting it, else the file beside it), then the
// platform's search. Call it before the first call that needs the
// library; naming another file once one is loaded is an error, naming the
// same one is not.
func UseLibrary(where string) error { return native.Use(native.LibIntake, where) }
