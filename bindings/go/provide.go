// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"reflect"
	"unsafe"
)

// Provider is one thing a library written in Go offers: a Go value (or a
// way to build one from a configuration) serving the kinds it names.
// Single and Configured make one.
type Provider struct{ spec *providerSpec }

type providerSpec struct {
	name        string
	id          string
	displayName string
	version     string
	meta        map[string]any
	kinds       []*kindDesc
	available   func() error
	// instance is the provider's own one, or nil.
	instance any
	// build makes an instance from a configuration the host checked;
	// config is its argument's type, schema the schema it takes.
	build   func(config unsafe.Pointer) (any, error)
	config  reflect.Type
	schema  any
	typ     reflect.Type
	buildFn uintptr
	optErr  error
}

// ProviderOption says something about a provider: a kind it serves (a
// *Kind is one), or ID, DisplayName, ProviderVersion, ProviderMeta,
// AvailableIf, Default, ConfigSchema.
type ProviderOption interface{ applyProvider(*providerSpec) }

type providerFunc func(*providerSpec)

func (f providerFunc) applyProvider(s *providerSpec) { f(s) }

func (k *Kind[R]) applyProvider(s *providerSpec) { s.kinds = append(s.kinds, k.d) }

// ID is the provider's id, which defaults to {last element of the library
// id}_{name}.
func ID(id string) ProviderOption { return providerFunc(func(s *providerSpec) { s.id = id }) }

// DisplayName is a name to show a person.
func DisplayName(name string) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.displayName = name })
}

// ProviderVersion is the provider's own version; empty, the default,
// inherits the library's.
func ProviderVersion(v string) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.version = v })
}

// ProviderMeta is anything else the provider says, as a map.
func ProviderMeta(meta map[string]any) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.meta = meta })
}

// AvailableIf asks whether the provider can run here: nil when it can,
// an error saying why not. It is asked every time a host asks.
func AvailableIf(check func() error) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.available = check })
}

// Default offers instance as the provider's own beside the ones a
// Configured provider builds, the one a host calls without configuring.
func Default(instance any) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.instance = instance })
}

// ConfigSchema is the raw JSON Schema a Configured provider takes, in
// place of the one its configuration type describes: for a configuration
// read as map[string]any, or a schema a type cannot say.
func ConfigSchema(schema any) ProviderOption {
	return providerFunc(func(s *providerSpec) { s.schema = schema })
}

func newProvider(name string, s *providerSpec, opts []ProviderOption) Provider {
	s.name = name
	for _, opt := range opts {
		if opt == nil {
			continue
		}
		opt.applyProvider(s)
	}
	return Provider{spec: s}
}

// Single is a provider that is its one instance: instance serves every
// kind among opts, through the methods it has. When instance implements
// io.Closer, Library.Close closes it.
func Single(name string, instance any, opts ...ProviderOption) Provider {
	s := &providerSpec{instance: instance}
	if instance != nil {
		s.typ = reflect.TypeOf(instance)
	} else {
		s.optErr = fmt.Errorf("guatiao: the provider %s has no instance", name)
	}
	return newProvider(name, s, opts)
}

// Configured is a provider that builds an instance per configuration: the
// host's configuration, checked against the schema C's type describes
// (SchemaOf[C]), is decoded into a C and handed to build. P is what build
// makes, which serves every kind among opts; an instance that implements
// io.Closer is closed when its host destroys it.
func Configured[C, P any](name string, build func(C) (P, error), opts ...ProviderOption) Provider {
	s := &providerSpec{config: reflect.TypeFor[C](), typ: reflect.TypeFor[P]()}
	if build == nil {
		s.optErr = fmt.Errorf("guatiao: the provider %s has no build function", name)
	} else {
		s.buildFn = reflect.ValueOf(build).Pointer()
	}
	s.build = func(config unsafe.Pointer) (any, error) {
		var c C
		if err := decodeInto(config, &c, ""); err != nil {
			return nil, err
		}
		p, err := build(c)
		if err != nil {
			return nil, err
		}
		return p, nil
	}
	return newProvider(name, s, opts)
}

// check refuses a provider that cannot be described: no kinds, an object
// kind among them, a value that does not implement one.
func (s *providerSpec) check() error {
	if s.optErr != nil {
		return s.optErr
	}
	if s.name == "" {
		return fmt.Errorf("guatiao: a provider has a name")
	}
	if len(s.kinds) == 0 {
		return fmt.Errorf("guatiao: the provider %s names no kind", s.name)
	}
	for _, d := range s.kinds {
		if d.spec.Object {
			return fmt.Errorf("guatiao: %s: %s is an object kind; a provider serves provider kinds", s.name, d.spec.Name)
		}
		if _, err := d.tableFor(s.typ); err != nil {
			return fmt.Errorf("guatiao: the provider %s: %w", s.name, err)
		}
	}
	if s.instance != nil && s.build != nil && !reflect.TypeOf(s.instance).AssignableTo(s.typ) {
		// The default answers through the same tables as the instances.
		return fmt.Errorf("guatiao: the provider %s's default is a %T, not a %s", s.name, s.instance, s.typ)
	}
	return nil
}
