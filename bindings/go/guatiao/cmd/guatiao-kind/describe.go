// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/token"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// A kind born in Go: its interfaces are the only source, and what a Go type
// cannot say (an argument lent rather than borrowed, a slot that cannot
// fail) is said by a directive in the method's doc comment.
//
//	//guatiao:kind tally                  the kind (on the interface)
//	//guatiao:optional Aller Knower       optional slots, an interface each, in table order:
//	                                      after the kind's methods (on the interface) or
//	                                      before a method (on that method)
//	//guatiao:floorhash 2688319391        the floor hash, when it is not to be computed
//	//guatiao:infallible                  the slot cannot fail (on a method)
//	//guatiao:lent watcher                these arguments are lent for the call
//	//guatiao:taken counts                these trees are handed over
//	//guatiao:nullable v                  this borrowed Ref may be null
//	//guatiao:mut dst                     these bytes are written in place

type outDocument struct {
	Kinds []outKind `json:"kinds"`
}

type outKind struct {
	Name      string    `json:"name"`
	FloorHash uint32    `json:"floor_hash"`
	Object    bool      `json:"object"`
	Doc       string    `json:"doc,omitempty"`
	Slots     []outSlot `json:"slots"`
}

type outSlot struct {
	Name     string    `json:"name"`
	Required bool      `json:"required"`
	Fallible bool      `json:"fallible"`
	Args     []outType `json:"args"`
	Answer   *outType  `json:"answer,omitempty"`
	Doc      string    `json:"doc,omitempty"`
}

type outType struct {
	Name     string `json:"name,omitempty"`
	Type     string `json:"type"`
	Kind     string `json:"kind,omitempty"`
	Pass     string `json:"pass,omitempty"`
	Nullable bool   `json:"nullable,omitempty"`
}

// describeScalars are the description's scalar types, by the Go type a
// method spells them with.
var describeScalars = map[string]string{
	"bool": "bool", "int8": "i8", "int16": "i16", "int32": "i32", "int64": "i64",
	"uint8": "u8", "byte": "u8", "uint16": "u16", "uint32": "u32", "uint64": "u64",
	"int": "isize", "uint": "usize", "float32": "f32", "float64": "f64",
}

// describe is the document of the package's kinds, read from their
// interfaces, in the order the package declares them (files by name, then
// as written). Every object kind a slot reaches is one of them.
func (p *pkgInfo) describe() ([]byte, error) {
	doc := outDocument{Kinds: []outKind{}}
	named := map[string]string{}
	for _, k := range p.kinds {
		if other, twice := named[k.kind]; twice {
			return nil, fmt.Errorf("%s: %s and %s are both the kind %s", p.at(p.ifaces[k.iface].pos), other, k.iface, k.kind)
		}
		named[k.kind] = k.iface
		d, err := p.describeKind(k)
		if err != nil {
			return nil, err
		}
		doc.Kinds = append(doc.Kinds, d)
	}
	var b bytes.Buffer
	enc := json.NewEncoder(&b)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(doc); err != nil {
		return nil, err
	}
	return b.Bytes(), nil
}

// directives are the //guatiao: lines of a comment, as name -> words, in
// the order written; allowed names the ones that may appear there.
func directives(cg *ast.CommentGroup, where string, allowed ...string) (map[string][]string, error) {
	out := map[string][]string{}
	if cg == nil {
		return out, nil
	}
	for _, c := range cg.List {
		rest, ok := strings.CutPrefix(c.Text, "//guatiao:")
		if !ok {
			continue
		}
		fields := strings.Fields(rest)
		if len(fields) == 0 || !slices.Contains(allowed, fields[0]) {
			if len(allowed) == 0 {
				return nil, fmt.Errorf("%s: //guatiao:%s does not belong here", where, rest)
			}
			return nil, fmt.Errorf("%s: //guatiao:%s is not one of %s here", where, rest, strings.Join(allowed, ", "))
		}
		if len(fields) == 1 && fields[0] != "infallible" && fields[0] != "kind" {
			return nil, fmt.Errorf("%s: //guatiao:%s names what it marks", where, fields[0])
		}
		if _, twice := out[fields[0]]; twice {
			return nil, fmt.Errorf("%s: //guatiao:%s is written twice", where, fields[0])
		}
		out[fields[0]] = fields[1:]
	}
	return out, nil
}

// at is a position as a refusal names it: the file's name and the line.
func (p *pkgInfo) at(pos token.Pos) string {
	where := p.fset.Position(pos)
	return fmt.Sprintf("%s:%d", filepath.Base(where.Filename), where.Line)
}

// docText is a doc comment as a description says it: its text, directives
// left out, with no blank line before or after.
func docText(cg *ast.CommentGroup) string {
	if cg == nil {
		return ""
	}
	return strings.Trim(cg.Text(), "\n")
}

var slotName = regexp.MustCompile(`^[a-z][a-z0-9]*(_[a-z0-9]+)*$`)

// snake is a method's slot name: GreetInto is greet_into. Refused unless
// the glue's CamelCase of it is the method again.
func snake(method, where string) (string, error) {
	var b strings.Builder
	for i, r := range method {
		if r >= 'A' && r <= 'Z' {
			if i > 0 && method[i-1] >= 'A' && method[i-1] <= 'Z' {
				return "", fmt.Errorf("%s: %s has capitals in a row, so no slot name comes back to it; "+
					"spell it as its slot name in CamelCase (ParseHttp for parse_http)", where, method)
			}
			if i > 0 {
				b.WriteByte('_')
			}
			r += 'a' - 'A'
		}
		b.WriteRune(r)
	}
	name := b.String()
	if !slotName.MatchString(name) || camel(name) != method {
		return "", fmt.Errorf("%s: %s is no slot's name in CamelCase (letters and digits, starting with a capital)", where, method)
	}
	return name, nil
}

// describeKind is one kind as its interfaces say it: the kind's methods in
// order, each optional slot where a //guatiao:optional names it.
func (p *pkgInfo) describeKind(k *kindDecl) (outKind, error) {
	where := p.at(p.ifaces[k.iface].pos) + ": " + k.iface
	dirs, err := directives(k.doc, where, "kind", "optional", "floorhash")
	if err != nil {
		return outKind{}, err
	}
	_, object := k.required["Close"]
	d := outKind{Name: k.kind, Object: object, Doc: docText(k.doc), Slots: []outSlot{}}
	add := func(m *goMethod, required bool, fallback *ast.CommentGroup) error {
		s, err := p.describeSlot(m, required, fallback, "optional")
		if err != nil {
			return err
		}
		for _, other := range d.Slots {
			if other.Name == s.Name {
				return fmt.Errorf("%s: two slots of %s are %s", p.at(m.pos), k.kind, s.Name)
			}
		}
		if m.name == "Destroy" {
			return fmt.Errorf("%s: destroy is the slot an object kind's table starts with (its Close), and no other", p.at(m.pos))
		}
		d.Slots = append(d.Slots, s)
		return nil
	}
	// The slots of the interfaces an optional directive names, in order.
	optional := func(names []string, where string) error {
		for _, name := range names {
			gi, ok := p.ifaces[name]
			switch {
			case !ok:
				return fmt.Errorf("%s: //guatiao:optional names %s, which is no interface of the package", where, name)
			case p.byIface[name] != nil:
				return fmt.Errorf("%s: //guatiao:optional names %s, which is a kind of its own", where, name)
			case len(gi.methods) != 1:
				return fmt.Errorf("%s: %s, an optional slot, has one method, not %d", where, name, len(gi.methods))
			}
			m := gi.methods[0]
			if _, err := directives(gi.doc, p.at(gi.pos)+": "+name+" (a directive of a slot goes on its method)"); err != nil {
				return err
			}
			if _, clash := k.required[m.name]; clash {
				return fmt.Errorf("%s: %s.%s is a method of %s already", p.at(m.pos), name, m.name, k.iface)
			}
			// The glue finds an optional slot's method by its name among the
			// interfaces that are no kind: it must be the only one.
			for _, other := range p.methods[m.name] {
				if other != m && p.byIface[other.iface] == nil {
					return fmt.Errorf("%s: %s and %s both have %s; an optional slot's method is declared once",
						p.at(m.pos), name, other.iface, m.name)
				}
			}
			if err := add(m, false, gi.doc); err != nil {
				return err
			}
		}
		return nil
	}
	for _, m := range k.order {
		at := p.at(m.pos) + ": " + k.iface + "." + m.name
		if object && m.name == "Close" {
			if _, err := directives(m.doc, at); err != nil {
				return d, err
			}
			continue
		}
		before, err := directives(m.doc, at, "optional", "infallible", "lent", "taken", "nullable", "mut")
		if err != nil {
			return d, err
		}
		if err := optional(before["optional"], at); err != nil {
			return d, err
		}
		if err := add(m, true, nil); err != nil {
			return d, err
		}
	}
	if err := optional(dirs["optional"], where); err != nil {
		return d, err
	}
	// The library reads the kind whether or not its hash is pinned: what no
	// kind may declare is refused there.
	if d.FloorHash, err = floorHashOf(object, d.Slots); err != nil {
		return d, fmt.Errorf("%s: %w", where, err)
	}
	if pinned, ok := dirs["floorhash"]; ok {
		n, err := strconv.ParseUint(strings.Join(pinned, " "), 10, 32)
		if err != nil {
			return d, fmt.Errorf("%s: //guatiao:floorhash is a 32-bit unsigned integer, not %q", where, strings.Join(pinned, " "))
		}
		d.FloorHash = uint32(n)
	}
	return d, nil
}

func (p *pkgInfo) describeSlot(m *goMethod, required bool, fallback *ast.CommentGroup, also ...string) (outSlot, error) {
	where := p.at(m.pos) + ": " + m.iface + "." + m.name
	name, err := snake(m.name, where)
	if err != nil {
		return outSlot{}, err
	}
	dirs, err := directives(m.doc, where, append([]string{"infallible", "lent", "taken", "nullable", "mut"}, also...)...)
	if err != nil {
		return outSlot{}, err
	}
	if words, ok := dirs["infallible"]; ok && len(words) > 0 {
		return outSlot{}, fmt.Errorf("%s: //guatiao:infallible takes no argument", where)
	}
	marks := map[string]string{}
	for _, mark := range []string{"lent", "taken", "nullable", "mut"} {
		for _, arg := range dirs[mark] {
			if marks[arg] != "" {
				return outSlot{}, fmt.Errorf("%s: %s is marked %s and %s", where, arg, marks[arg], mark)
			}
			marks[arg] = mark
		}
	}
	doc := m.doc
	if docText(doc) == "" {
		doc = fallback
	}
	s := outSlot{Name: name, Required: required, Args: []outType{}, Doc: docText(doc)}
	_, s.Fallible = dirs["infallible"]
	s.Fallible = !s.Fallible
	if n := len(m.results); n == 0 || n > 2 || m.results[n-1] != "error" {
		return s, fmt.Errorf("%s returns an error last, after the answer if it has one", where)
	}
	gt := p.gt(m.file)
	for _, gp := range m.params {
		if gp.name == "_" || gp.unnamed {
			return s, fmt.Errorf("%s: every argument has a name of its own, which the description carries", where)
		}
		t, err := p.describeArg(gp, marks[gp.name], gt)
		if err != nil {
			return s, fmt.Errorf("%s(%s): %w", where, gp.name, err)
		}
		delete(marks, gp.name)
		s.Args = append(s.Args, t)
	}
	if len(marks) > 0 {
		var names []string
		for arg, mark := range marks {
			names = append(names, "//guatiao:"+mark+" "+arg)
		}
		sort.Strings(names)
		return s, fmt.Errorf("%s: %s: no argument has that name", where, strings.Join(names, ", "))
	}
	if len(m.results) == 2 {
		t, err := p.describeAnswer(m.results[0], m.resultExprs[0], gt)
		if err != nil {
			return s, fmt.Errorf("%s, the answer: %w", where, err)
		}
		s.Answer = &t
	}
	return s, nil
}

// objectKind is the kind an interface of the package names, which must be
// an object kind.
func (p *pkgInfo) objectKind(t string) (string, bool, error) {
	k, ok := p.byIface[t]
	if !ok {
		return "", false, nil
	}
	if _, object := k.required["Close"]; !object {
		return "", true, fmt.Errorf("%s is a kind but no object kind (it has no Close() error)", t)
	}
	return k.kind, true, nil
}

func (p *pkgInfo) describeArg(gp goParam, mark, gt string) (outType, error) {
	t := outType{Name: gp.name}
	only := func(allowed ...string) error {
		if mark != "" && !slices.Contains(allowed, mark) {
			return fmt.Errorf("a %s cannot be marked %s", gp.typ, mark)
		}
		return nil
	}
	if s, ok := describeScalars[gp.typ]; ok {
		t.Type = s
		return t, only()
	}
	switch gp.typ {
	case "string":
		t.Type = "str"
		return t, only()
	case "[]byte":
		t.Type = "bytes"
		if mark == "mut" {
			t.Type = "bytes_mut"
		}
		return t, only("mut")
	case gt + ".Ref", gt + ".MapRef", gt + ".ListRef":
		t.Type = map[string]string{"Ref": "value", "MapRef": "map", "ListRef": "list"}[strings.TrimPrefix(gp.typ, gt+".")]
		t.Pass = "borrowed"
		switch mark {
		case "lent":
			t.Pass = "lent"
		case "nullable":
			if t.Type != "value" {
				return t, fmt.Errorf("only a Ref may be null")
			}
			t.Nullable = true
		case "":
			if t.Type == "list" {
				return t, fmt.Errorf("a list is not borrowed: mark it lent, or take a Go type it decodes into, marked taken")
			}
		default:
			return t, fmt.Errorf("a view is borrowed, or lent or nullable; a tree handed over is a *%s.Value or a Go type it decodes into", gt)
		}
		return t, nil
	case "*" + gt + ".Value":
		t.Type, t.Pass = "value", "taken"
		return t, only("taken")
	}
	if kind, isKind, err := p.objectKind(gp.typ); isKind {
		if err != nil {
			return t, err
		}
		t.Type, t.Kind, t.Pass = "object", kind, "taken"
		if mark == "lent" {
			t.Pass = "lent"
		}
		return t, only("lent", "taken")
	}
	tree, err := p.decoded(gp.expr, gt)
	if err != nil {
		return t, err
	}
	t.Type, t.Pass = tree, "borrowed"
	if mark == "taken" {
		t.Pass = "taken"
	} else if tree == "list" {
		return t, fmt.Errorf("a list is not borrowed: mark it taken")
	}
	return t, only("taken")
}

func (p *pkgInfo) describeAnswer(goType string, expr ast.Expr, gt string) (outType, error) {
	if s, ok := describeScalars[goType]; ok {
		return outType{Type: s}, nil
	}
	switch goType {
	case "string":
		return outType{Type: "text"}, nil
	case "*" + gt + ".Value":
		return outType{Type: "value"}, nil
	case gt + ".Ref", gt + ".MapRef", gt + ".ListRef":
		return outType{}, fmt.Errorf("a tree answered is a *%s.Value or a Go type, not a view", gt)
	}
	if kind, isKind, err := p.objectKind(goType); isKind {
		return outType{Type: "object", Kind: kind}, err
	}
	tree, err := p.decoded(expr, gt)
	return outType{Type: tree}, err
}

// decoded is the tree a Go type a value decodes into or encodes from
// crosses as: a struct or a map is a map, a slice a list, an interface a
// value. A type of the package's is read through its declaration; one from
// another package cannot be, and is refused.
func (p *pkgInfo) decoded(expr ast.Expr, gt string) (string, error) {
	seen := map[string]bool{}
	var of func(e ast.Expr, text string) (string, error)
	of = func(e ast.Expr, text string) (string, error) {
		switch x := e.(type) {
		case *ast.StructType:
			return "map", nil
		case *ast.MapType:
			if p.exprText(x.Key) != "string" {
				return "", fmt.Errorf("%s: a map's keys are text, so a Go map crossing as one has string keys", text)
			}
			return "map", nil
		case *ast.ArrayType:
			if x.Len == nil && p.exprText(x.Elt) != "byte" && p.exprText(x.Elt) != "uint8" {
				return "list", nil
			}
		case *ast.InterfaceType:
			return "value", nil
		case *ast.StarExpr:
			return of(x.X, p.exprText(x.X))
		case *ast.Ident:
			if x.Name == "any" {
				return "value", nil
			}
			if decl, ok := p.types[x.Name]; ok && !seen[x.Name] {
				seen[x.Name] = true
				return of(decl, x.Name)
			}
		}
		if strings.Contains(text, ".") && !strings.HasPrefix(text, gt+".") {
			return "", fmt.Errorf("%s is another package's type, which this command cannot read; name a type of this package", text)
		}
		return "", fmt.Errorf("%s crosses as no description type", text)
	}
	return of(expr, p.exprText(expr))
}

// floorHashOf is the floor hash of a kind born outside Rust, computed by
// the library (guatiao.KindFloorHash): FNV-1a (32-bit) of provider or
// object and every slot in table order, each spelled as #[guatiao::kind]
// spells it, so an interface repeating a trait's slots hashes as the
// trait does. Describing a kind therefore loads the library.
func floorHashOf(object bool, slots []outSlot) (uint32, error) {
	kind, err := json.Marshal(struct {
		Object bool      `json:"object"`
		Slots  []outSlot `json:"slots"`
	}{object, slots})
	if err != nil {
		return 0, err
	}
	return guatiao.KindFloorHash(kind)
}
