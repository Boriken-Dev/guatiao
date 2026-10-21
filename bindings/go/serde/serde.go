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

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/bridge"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/native"
)

// Format is a document format.
type Format string

// The formats.
const (
	JSON Format = "json"
	TOML Format = "toml"
	YAML Format = "yaml"
)

// How bytes are written, and whether data URIs are read back as bytes:
// one Bytes flag, or'd with ReadDataURIs, through How.
const (
	BytesDataURI = native.BytesDataURI
	BytesBase64  = native.BytesBase64
	BytesArray   = native.BytesArray
	BytesRefuse  = native.BytesRefuse
	ReadDataURIs = native.ReadDataURIs
)

type spec struct {
	code        uint32
	parse, emit string
	pretty      string
	feature     string
}

var specs = map[Format]spec{
	JSON: {code: native.FormatJSON, parse: "guatiao_json_parse_ex", emit: "guatiao_json_emit", pretty: "guatiao_json_emit_pretty"},
	TOML: {code: native.FormatTOML, parse: "guatiao_toml_parse_ex", emit: "guatiao_toml_emit", feature: "toml"},
	YAML: {code: native.FormatYAML, parse: "guatiao_yaml_parse_ex", emit: "guatiao_yaml_emit", feature: "yaml"},
}

var names = map[uint32]Format{native.FormatJSON: JSON, native.FormatTOML: TOML, native.FormatYAML: YAML}

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

func open(format Format, symbol func(spec) string) (*native.Lib, spec, error) {
	s, ok := specs[format]
	if !ok {
		return nil, spec{}, fmt.Errorf("guatiao: unknown format %q", format)
	}
	if err := bridge.Core(); err != nil {
		return nil, s, err
	}
	lib, err := native.Serde()
	if err != nil {
		return nil, s, bridge.Lift(err)
	}
	if err := lib.Need(symbol(s)); err != nil {
		return nil, s, bridge.Lift(err)
	}
	return lib, s, nil
}

// Parse reads a document into a new Value to close.
func Parse(text string, format Format, opts ...Option) (*guatiao.Value, error) {
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	lib, s, err := open(format, func(s spec) string { return s.parse })
	if err != nil {
		return nil, err
	}
	alloc := o.alloc
	if alloc == nil {
		alloc = lib.Alloc()
	}
	out := native.Calloc(native.SizeValue)
	failure, st := lib.Parse(s.code, text, o.how, alloc, out)
	if st != native.OK {
		native.ValueFree(out)
		native.Free(out)
		if st == native.ErrBadValue {
			f := names[failure.Format]
			if f == "" {
				f = format
			}
			return nil, &ParseError{Format: f, Line: int(failure.Line), Column: int(failure.Column), Reason: failure.Message}
		}
		return nil, bridge.Check(st, failure.Message)
	}
	return bridge.Adopt(out, alloc).(*guatiao.Value), nil
}

// Emit writes x (a Value, a view, or anything guatiao.NewValue takes) as
// a document.
func Emit(x any, format Format, opts ...Option) (string, error) {
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	lib, s, err := open(format, func(s spec) string {
		if o.pretty && s.pretty != "" {
			return s.pretty
		}
		return s.emit
	})
	if err != nil {
		return "", err
	}
	var text string
	err = bridge.WithNodes([]any{x}, func(nodes []unsafe.Pointer) error {
		out := native.Calloc(native.SizeValue)
		defer func() {
			native.ValueFree(out)
			native.Free(out)
		}()
		if err := bridge.Check(lib.Emit(s.code, nodes[0], o.how, o.pretty, lib.Alloc(), out), ""); err != nil {
			return err
		}
		if native.Tag(out) == native.TagString {
			text = native.TextOf(out)
		}
		return nil
	})
	return text, err
}
