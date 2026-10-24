// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package intake is taking a value in from a person, through the
// guatiao_intake library: checking a form against its schema, laying a
// schema out in sections, conditional visibility, the path grammar
// (`agent[1].name`), and the flat `key -> text` projection a front end
// stores. It is the module
// github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake, over the
// guatiao module's values:
//
//	import intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"
//
// Every schema, form, value and store argument is a *guatiao.Value, a
// view, or plain data guatiao.NewValue takes.
package intake

import (
	"fmt"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// library is guatiao_intake.
var library = &guatiao.Surface{
	Basename: "guatiao_intake",
	Variable: "GUATIAO_INTAKE_LIBRARY",
	Probe:    "guatiao_intake_check",
	Exports:  exports(),
}

func exports() map[string]string {
	all := map[string]string{}
	for _, name := range exportNames {
		all[name] = ""
	}
	return all
}

// answer runs call, given the address of the export name, with a fresh
// out node, and hands it to read as a Value it owns.
func answer(name string, call func(fn, alloc, out unsafe.Pointer) uint32, read func(status uint32, out *guatiao.Value) error) error {
	fn, err := library.Symbol(name)
	if err != nil {
		return err
	}
	out := calloc()
	st := call(fn, library.Alloc(), out)
	return read(st, guatiao.Adopt(out, library.Alloc()))
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

// ProviderFormKey is the key of a provider's meta holding its form
// document: read it with guatiao's Registry.ProviderMeta, then Check it
// against the provider's ProviderConfig.
const ProviderFormKey = "form"

// Check is nil when form fits schema, else the reason as a map: `kind`
// (`malformed`, `unknown_field`, `duplicate_section`,
// `condition_refused` or `cyclic_condition`), `at`, whichever of `path`,
// `id`, `field` and `expected` apply, and a `message`.
func Check(schema, form any) (map[string]any, error) {
	var why map[string]any
	err := guatiao.WithNodes([]any{schema, form}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_check", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callTwo(fn, n[0], n[1], alloc, out)
		}, func(st uint32, out *guatiao.Value) error {
			defer out.Close()
			if st == statusOK {
				return nil
			}
			if st != badValue {
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
		return answer("guatiao_intake_layout", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callTwo(fn, n[0], n[1], alloc, out)
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
		fn, err := library.Symbol("guatiao_intake_is_visible")
		if err != nil {
			return err
		}
		ok, st := callVisible(fn, n[0], n[1], key, n[2])
		shown = ok
		return guatiao.StatusError(st, "")
	})
	return shown, err
}

// borrowedAt is a read-only Ref over what find answers inside source,
// found again on every access; false when it answers nothing now.
func borrowedAt(source any, name, path string) (guatiao.Ref, bool, error) {
	fn, err := library.Symbol(name)
	if err != nil {
		return guatiao.Ref{}, false, err
	}
	find := func(node unsafe.Pointer) unsafe.Pointer { return callFind(fn, node, path) }
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
	return borrowedAt(value, "guatiao_intake_path_get", path)
}

// Resolve is the field of schema (a *guatiao.Value or a view) governing
// a flat key, borrowed as At borrows; false for none.
func Resolve(schema any, key string) (guatiao.Ref, bool, error) {
	return borrowedAt(schema, "guatiao_intake_resolve", key)
}

// ForSchema is the form a schema implies, a Value to close.
func ForSchema(schema any) (*guatiao.Value, error) {
	var form *guatiao.Value
	err := guatiao.WithNodes([]any{schema}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_for_schema", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callOne(fn, n[0], alloc, out)
		}, owned(&form))
	})
	return form, err
}

// FormFor is the form to show the field at path with, a Value to close,
// or nil when there is none.
func FormFor(form, schema any, path string) (*guatiao.Value, error) {
	var found *guatiao.Value
	err := guatiao.WithNodes([]any{form, schema}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_form_for", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callTwoText(fn, n[0], n[1], path, alloc, out)
		}, owned(&found))
	})
	return found, err
}

// FlatKeys are the flat keys the field under key projects onto.
// guatiao.ErrWrongKind for a key that names no field.
func FlatKeys(schema any, key string) ([]string, error) {
	var keys []string
	err := guatiao.WithNodes([]any{schema}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_flat_keys", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callOneText(fn, n[0], key, alloc, out)
		}, plain(&keys))
	})
	return keys, err
}

// Flatten writes value into a flat store, one entry per scalar leaf.
func Flatten(schema any, key string, value any) (map[string]string, error) {
	store := map[string]string{}
	err := guatiao.WithNodes([]any{schema, value}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_flatten", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callOneTextOne(fn, n[0], key, n[1], alloc, out)
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
		if !textStore(node, store) {
			return nil, &guatiao.Error{Status: guatiao.WrongKind, Message: "a flat store maps strings to strings"}
		}
	} else if _, ok := store.(map[string]string); !ok {
		return nil, &guatiao.Error{Status: guatiao.WrongKind, Message: fmt.Sprintf("a flat store maps strings to strings, not a %T", store)}
	}
	var rebuilt *guatiao.Value
	err := guatiao.WithNodes([]any{schema, store}, func(n []unsafe.Pointer) error {
		return answer("guatiao_intake_unflatten", func(fn, alloc, out unsafe.Pointer) uint32 {
			return callOneTextOne(fn, n[0], key, n[1], alloc, out)
		}, func(st uint32, out *guatiao.Value) error {
			if st == wrongKind {
				out.Close()
				return nil
			}
			return owned(&rebuilt)(st, out)
		})
	})
	return rebuilt, err
}

// textStore reports whether node, inside keep, is a map of strings.
func textStore(node unsafe.Pointer, keep any) bool {
	ref, err := guatiao.Borrow(func() unsafe.Pointer { return node }, keep)
	if err != nil {
		return false
	}
	m, err := ref.Map()
	if err != nil {
		return false
	}
	for _, v := range m.All() {
		if v.Tag() != guatiao.TagString {
			return false
		}
	}
	return true
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
func UseLibrary(where string) error { return library.Use(where) }
