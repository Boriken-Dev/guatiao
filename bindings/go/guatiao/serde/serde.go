// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package serde reads and writes values as JSON, TOML and YAML documents,
// through the guatiao_serde library.
//
//	v, err := serde.Parse(`{"port": 5900}`, serde.JSON)
//	defer v.Close()
//	text, err := serde.Emit(v, serde.YAML)
//
// A number keeps its exact text, except that an exponent is read back with
// its sign (1e400 is 1e+400). TOML and YAML need a guatiao_serde built
// with them; without, the call answers a *guatiao.MissingSymbol naming the
// feature.
package serde

import (
	"fmt"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// Format is a document format.
type Format string

// The formats.
const (
	JSON Format = "json"
	TOML Format = "toml"
	YAML Format = "yaml"
)

type spec struct {
	parse, emit string
	pretty      string
	feature     string
}

var specs = map[Format]spec{
	JSON: {parse: "guatiao_json_parse_ex", emit: "guatiao_json_emit", pretty: "guatiao_json_emit_pretty"},
	TOML: {parse: "guatiao_toml_parse_ex", emit: "guatiao_toml_emit", feature: "toml"},
	YAML: {parse: "guatiao_yaml_parse_ex", emit: "guatiao_yaml_emit", feature: "yaml"},
}

var names = map[uint32]Format{formatJSON: JSON, formatTOML: TOML, formatYAML: YAML}

// library is guatiao_serde, with every export a format names.
var library = &guatiao.Surface{
	Basename: "guatiao_serde",
	Variable: "GUATIAO_SERDE_LIBRARY",
	Probe:    "guatiao_json_parse_ex",
	Exports:  exports(),
}

func exports() map[string]string {
	all := map[string]string{"guatiao_value_free": ""}
	for _, s := range specs {
		for _, name := range []string{s.parse, s.emit, s.pretty} {
			if name != "" {
				all[name] = s.feature
			}
		}
	}
	return all
}

// Option says how a document is read or written.
type Option func(*options)

type options struct {
	how    uint32
	pretty bool
	alloc  unsafe.Pointer
}

// How passes the Bytes and ReadDataURIs flags.
func How(flags uint32) Option { return func(o *options) { o.how |= flags } }

// Pretty indents what Emit writes, for a person to read.
func Pretty() Option { return func(o *options) { o.pretty = true } }

// InAllocator builds what Parse reads in the guatiao_alloc at alloc,
// which the caller keeps alive while the tree lives.
func InAllocator(alloc unsafe.Pointer) Option { return func(o *options) { o.alloc = alloc } }

// ParseError says why a document did not parse: where, from 1, the
// column in characters (0 when the parser did not say), and the parser's
// own reason, which never quotes the document. errors.Is matches it
// against guatiao.ErrBadValue.
type ParseError struct {
	Format       Format
	Line, Column int
	Reason       string
}

func (e *ParseError) Error() string {
	if e.Line == 0 {
		return fmt.Sprintf("guatiao: %s: %s", e.Format, e.Reason)
	}
	return fmt.Sprintf("guatiao: %s: %s at line %d, column %d", e.Format, e.Reason, e.Line, e.Column)
}

// Unwrap is guatiao.ErrBadValue.
func (e *ParseError) Unwrap() error { return guatiao.ErrBadValue }

// open answers the format's spec and the addresses of the export symbol
// picks and of guatiao_value_free, the library open.
func open(format Format, symbol func(spec) string) (s spec, fn, free unsafe.Pointer, err error) {
	s, ok := specs[format]
	if !ok {
		return spec{}, nil, nil, fmt.Errorf("guatiao: unknown format %q", format)
	}
	if fn, err = library.Symbol(symbol(s)); err != nil {
		return s, nil, nil, err
	}
	if free, err = library.Symbol("guatiao_value_free"); err != nil {
		return s, nil, nil, err
	}
	return s, fn, free, nil
}

// Parse reads a document into a new Value to close.
func Parse(text string, format Format, opts ...Option) (*guatiao.Value, error) {
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	_, fn, free, err := open(format, func(s spec) string { return s.parse })
	if err != nil {
		return nil, err
	}
	alloc := o.alloc
	if alloc == nil {
		alloc = library.Alloc()
	}
	out := calloc()
	failed, st := parse(fn, free, text, o.how, alloc, out)
	if st != statusOK {
		release(free, out)
		if st == badValue {
			f := names[failed.format]
			if f == "" {
				f = format
			}
			return nil, &ParseError{Format: f, Line: int(failed.line), Column: int(failed.column), Reason: failed.message}
		}
		return nil, guatiao.StatusError(st, failed.message)
	}
	return guatiao.Adopt(out, alloc), nil
}

// Emit writes x (a Value, a view, or anything guatiao.NewValue takes) as
// a document.
func Emit(x any, format Format, opts ...Option) (string, error) {
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	s, fn, free, err := open(format, func(s spec) string {
		if o.pretty && s.pretty != "" {
			return s.pretty
		}
		return s.emit
	})
	if err != nil {
		return "", err
	}
	how := o.how
	if o.pretty && s.pretty == "" {
		how |= pretty
	}
	var text string
	err = guatiao.WithNodes([]any{x}, func(nodes []unsafe.Pointer) error {
		out := calloc()
		defer release(free, out)
		if err := guatiao.StatusError(emit(fn, nodes[0], how, library.Alloc(), out), ""); err != nil {
			return err
		}
		text, _ = textOf(out)
		return nil
	})
	return text, err
}

// UseLibrary names the guatiao_serde library to load instead of looking for
// one: a file exporting its functions, whatever it is called (an
// application's own library carrying several surfaces, say), or a
// directory holding it under its platform file name. Without it the
// library is found through GUATIAO_SERDE_LIBRARY, then GUATIAO_LIBRARY (a
// directory, or a file exporting it, else the file beside it), then the
// platform's search. Call it before the first call that needs the
// library; naming another file once one is loaded is an error, naming the
// same one is not.
func UseLibrary(where string) error { return library.Use(where) }
