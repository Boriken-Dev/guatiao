// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"go/types"
	"hash/fnv"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

const bindingPath = "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

// goMethod is one method of an interface in the package.
type goMethod struct {
	iface       string
	name        string
	params      []goParam
	results     []string
	resultExprs []ast.Expr
	file        *ast.File
	doc         *ast.CommentGroup
	pos         token.Pos
}

// goIface is an interface of the package, kind or not.
type goIface struct {
	name    string
	doc     *ast.CommentGroup
	file    *ast.File
	pos     token.Pos
	methods []*goMethod
}

type goParam struct {
	name, typ string
	expr      ast.Expr
	unnamed   bool // written without a name; load called it a<i>
}

// kindDecl is an interface marked as a kind.
type kindDecl struct {
	iface, kind, table string
	required           map[string]*goMethod
	order              []*goMethod // required, as the interface lists them
	file               *ast.File
	doc                *ast.CommentGroup
}

type pkgInfo struct {
	name     string
	files    []*ast.File
	kinds    []*kindDecl
	byIface  map[string]*kindDecl
	methods  map[string][]*goMethod
	ifaces   map[string]*goIface  // every interface, by name
	types    map[string]ast.Expr  // every other type declaration, by name
	binding  map[*ast.File]string // the binding's import name in each file
	imports  map[string]string    // import name -> path, across the files declaring kinds
	fset     *token.FileSet
	prefix   string
	exprText func(ast.Expr) string
}

// run writes the glue of the package in dir, its kinds read from the
// file at from (relative to dir): a kinds document when its name ends in
// .json, else the kinds' C header.
func run(dir, from, out string, check bool) error {
	text, err := os.ReadFile(filepath.Join(dir, from))
	if err != nil {
		return err
	}
	tableOf := func(k *kindDecl) (*cTable, error) { return readTable(string(text), k.table) }
	if strings.HasSuffix(from, ".json") {
		doc, err := readDocument(text)
		if err != nil {
			return fmt.Errorf("%s: %w", from, err)
		}
		tableOf = func(k *kindDecl) (*cTable, error) { return describedTable(doc, k.kind, k.table) }
	}
	pkg, err := load(dir, out)
	if err != nil {
		return err
	}
	if len(pkg.kinds) == 0 {
		return fmt.Errorf("no interface in %s is marked //guatiao:kind", dir)
	}
	return pkg.write(dir, from, out, check, tableOf)
}

// runBorn writes the glue of a package whose kinds are born in Go: their
// document is written from the interfaces to document (relative to dir),
// and the glue from that document, as from any other.
func runBorn(dir, document, out string, check bool) error {
	pkg, err := load(dir, out)
	if err != nil {
		return err
	}
	if len(pkg.kinds) == 0 {
		return fmt.Errorf("no interface in %s is marked //guatiao:kind", dir)
	}
	text, err := pkg.describe()
	if err != nil {
		return err
	}
	doc, err := readDocument(text)
	if err != nil {
		return err
	}
	files, err := pkg.glue(dir, document, out,
		func(k *kindDecl) (*cTable, error) { return describedTable(doc, k.kind, k.table) })
	if err != nil {
		return err
	}
	path := filepath.Join(dir, document)
	if check {
		there, err := os.ReadFile(path)
		if err != nil || !bytes.Equal(there, text) {
			return fmt.Errorf("%s is not the document guatiao-kind writes from the interfaces: run go generate", path)
		}
	} else {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(path, text, 0o644); err != nil {
			return err
		}
	}
	return writeGlue(dir, document, out, check, files)
}

// write writes (or with check, compares) the glue of the package's kinds,
// each kind's table given by tableOf; from names where they were read.
func (p *pkgInfo) write(dir, from, out string, check bool, tableOf func(*kindDecl) (*cTable, error)) error {
	files, err := p.glue(dir, from, out, tableOf)
	if err != nil {
		return err
	}
	return writeGlue(dir, from, out, check, files)
}

// glue is the glue of the package's kinds, by file name, written nowhere.
func (p *pkgInfo) glue(dir, from, out string, tableOf func(*kindDecl) (*cTable, error)) (map[string][]byte, error) {
	importPath, err := importPathOf(dir)
	if err != nil {
		return nil, err
	}
	h := fnv.New32a()
	h.Write([]byte(importPath))
	p.prefix = fmt.Sprintf("gk%08x", h.Sum32())

	var plans []*kindPlan
	for _, k := range p.kinds {
		table, err := tableOf(k)
		if err != nil {
			return nil, err
		}
		plan, err := p.plan(k, table)
		if err != nil {
			return nil, err
		}
		plans = append(plans, plan)
	}
	goSrc, err := p.emitGo(plans, from, out)
	if err != nil {
		return nil, err
	}
	return map[string][]byte{
		out + ".go": goSrc,
		out + ".h":  p.emitH(plans, from, out),
		out + ".c":  p.emitC(plans, from, out),
	}, nil
}

// writeGlue writes the files glue made beside the package's sources, or
// with check fails when what is there differs.
func writeGlue(dir, from, out string, check bool, files map[string][]byte) error {
	for _, name := range []string{out + ".go", out + ".h", out + ".c"} {
		path := filepath.Join(dir, name)
		if check {
			there, err := os.ReadFile(path)
			if err != nil || !bytes.Equal(there, files[name]) {
				return fmt.Errorf("%s is not what guatiao-kind writes from %s: run go generate", path, from)
			}
			continue
		}
		if err := os.WriteFile(path, files[name], 0o644); err != nil {
			return err
		}
	}
	return nil
}

// importPathOf is the package's import path: its module's path and its
// directory inside the module.
func importPathOf(dir string) (string, error) {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return "", err
	}
	for at := abs; ; at = filepath.Dir(at) {
		data, err := os.ReadFile(filepath.Join(at, "go.mod"))
		if err == nil {
			for _, line := range strings.Split(string(data), "\n") {
				if rest, ok := strings.CutPrefix(strings.TrimSpace(line), "module "); ok {
					rel, _ := filepath.Rel(at, abs)
					p := strings.Trim(strings.TrimSpace(rest), `"`)
					if rel != "." {
						p += "/" + filepath.ToSlash(rel)
					}
					return p, nil
				}
			}
			return "", fmt.Errorf("%s names no module", filepath.Join(at, "go.mod"))
		}
		if filepath.Dir(at) == at {
			return "", fmt.Errorf("%s is in no module: no go.mod above it", dir)
		}
	}
}

func load(dir, out string) (*pkgInfo, error) {
	fset := token.NewFileSet()
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	pkg := &pkgInfo{byIface: map[string]*kindDecl{}, methods: map[string][]*goMethod{},
		ifaces: map[string]*goIface{}, types: map[string]ast.Expr{},
		binding: map[*ast.File]string{}, imports: map[string]string{}, fset: fset}
	pkg.exprText = func(e ast.Expr) string { return types.ExprString(e) }
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasSuffix(name, ".go") || strings.HasSuffix(name, "_test.go") || name == out+".go" {
			continue
		}
		f, err := parser.ParseFile(fset, filepath.Join(dir, name), nil, parser.ParseComments)
		if err != nil {
			return nil, err
		}
		pkg.name = f.Name.Name
		pkg.files = append(pkg.files, f)
		pkg.binding[f] = ""
		for _, imp := range f.Imports {
			path, _ := strconv.Unquote(imp.Path.Value)
			if path == bindingPath {
				if imp.Name != nil && imp.Name.Name != "guatiao" {
					return nil, fmt.Errorf("%s imports the binding as %s; the generated code names it guatiao, so import it under that name",
						name, imp.Name.Name)
				}
				pkg.binding[f] = "guatiao"
			}
		}
		for _, decl := range f.Decls {
			gd, ok := decl.(*ast.GenDecl)
			if !ok || gd.Tok != token.TYPE {
				continue
			}
			for _, spec := range gd.Specs {
				ts := spec.(*ast.TypeSpec)
				it, ok := ts.Type.(*ast.InterfaceType)
				if !ok {
					pkg.types[ts.Name.Name] = ts.Type
					continue
				}
				doc := ts.Doc
				if doc == nil && len(gd.Specs) == 1 {
					doc = gd.Doc
				}
				gi := &goIface{name: ts.Name.Name, doc: doc, file: f, pos: ts.Pos()}
				pkg.ifaces[gi.name] = gi
				var decl *kindDecl
				if doc != nil {
					for _, c := range doc.List {
						if rest, ok := strings.CutPrefix(c.Text, "//guatiao:kind "); ok {
							parts := strings.Fields(rest)
							if len(parts) != 1 && len(parts) != 2 {
								return nil, fmt.Errorf("%s: //guatiao:kind takes the kind's name, and its C table", ts.Name.Name)
							}
							// The table's name, which only a header needs, is <kind>_vtable unless said.
							parts = append(parts, parts[0]+"_vtable")
							decl = &kindDecl{iface: ts.Name.Name, kind: parts[0], table: parts[1],
								required: map[string]*goMethod{}, file: f, doc: doc}
						}
					}
				}
				for _, m := range it.Methods.List {
					ft, ok := m.Type.(*ast.FuncType)
					if !ok || len(m.Names) == 0 {
						if decl != nil {
							return nil, fmt.Errorf("%s: a kind's interface embeds nothing; list its methods", ts.Name.Name)
						}
						continue
					}
					gm := &goMethod{iface: ts.Name.Name, name: m.Names[0].Name, file: f, doc: m.Doc, pos: m.Pos()}
					i := 0
					for _, p := range ft.Params.List {
						names := p.Names
						if len(names) == 0 {
							names = []*ast.Ident{{Name: fmt.Sprintf("a%d", i)}}
						}
						for _, n := range names {
							gm.params = append(gm.params, goParam{name: n.Name, typ: pkg.exprText(p.Type), expr: p.Type,
								unnamed: len(p.Names) == 0})
							i++
						}
					}
					if ft.Results != nil {
						for _, r := range ft.Results.List {
							count := max(1, len(r.Names))
							for j := 0; j < count; j++ {
								gm.results = append(gm.results, pkg.exprText(r.Type))
								gm.resultExprs = append(gm.resultExprs, r.Type)
							}
						}
					}
					pkg.methods[gm.name] = append(pkg.methods[gm.name], gm)
					gi.methods = append(gi.methods, gm)
					if decl != nil {
						decl.required[gm.name] = gm
						decl.order = append(decl.order, gm)
					}
				}
				if decl != nil {
					pkg.kinds = append(pkg.kinds, decl)
					pkg.byIface[decl.iface] = decl
					for _, imp := range f.Imports {
						path, _ := strconv.Unquote(imp.Path.Value)
						local := path[strings.LastIndex(path, "/")+1:]
						if imp.Name != nil {
							local = imp.Name.Name
						}
						pkg.imports[local] = path
					}
				}
			}
		}
	}
	// The imports of every file declaring an optional slot's interface too.
	for _, f := range pkg.files {
		for _, imp := range f.Imports {
			path, _ := strconv.Unquote(imp.Path.Value)
			local := path[strings.LastIndex(path, "/")+1:]
			if imp.Name != nil {
				local = imp.Name.Name
			}
			if _, set := pkg.imports[local]; !set {
				pkg.imports[local] = path
			}
		}
	}
	return pkg, nil
}

// camel is a C slot name in Go: greet_into is GreetInto.
func camel(name string) string {
	var b strings.Builder
	for _, part := range strings.Split(name, "_") {
		if part == "" {
			continue
		}
		b.WriteString(strings.ToUpper(part[:1]) + part[1:])
	}
	return b.String()
}

func ident(name string) string {
	b := []byte(name)
	for i, c := range b {
		if !(c == '_' || c >= '0' && c <= '9' || c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z') {
			b[i] = '_'
		}
	}
	return string(b)
}

// ---- planning --------------------------------------------------------------------------

type argKind int

const (
	aStr argKind = iota
	aBytes
	aBytesMut
	aScalar
	aBorrow
	aLend
	aTake
	aObjTake
	aObjLend
)

type ansKind int

const (
	nNone ansKind = iota
	nTree
	nText
	nScalar
	nObject
)

type arg struct {
	kind    argKind
	cType   string // the slot's C type for the argument
	cName   string
	goName  string
	goType  string
	class   string // ValueClass, MapClass, ListClass
	view    string // "", Ref, MapRef, ListRef, Value
	objKind string
}

type answer struct {
	kind    ansKind
	cType   string
	goType  string
	class   string
	view    string
	objKind string
}

type slotPlan struct {
	c        cSlot
	index    int
	method   *goMethod
	required bool
	fallible bool
	args     []arg
	ans      answer
}

type kindPlan struct {
	decl  *kindDecl
	table *cTable
	slots []*slotPlan
}

var scalars = map[string]string{
	"bool": "bool", "int8_t": "int8", "uint8_t": "uint8", "int16_t": "int16", "uint16_t": "uint16",
	"int32_t": "int32", "uint32_t": "uint32", "int64_t": "int64", "uint64_t": "uint64",
	"size_t": "uint", "intptr_t": "int", "uintptr_t": "uintptr", "float": "float32", "double": "float64",
}

var cgoScalars = map[string]string{
	"bool": "C.bool", "int8_t": "C.int8_t", "uint8_t": "C.uint8_t", "int16_t": "C.int16_t", "uint16_t": "C.uint16_t",
	"int32_t": "C.int32_t", "uint32_t": "C.uint32_t", "int64_t": "C.int64_t", "uint64_t": "C.uint64_t",
	"size_t": "C.size_t", "intptr_t": "C.intptr_t", "uintptr_t": "C.uintptr_t", "float": "C.float", "double": "C.double",
}

var trees = map[string]string{"guatiao_value": "ValueClass", "guatiao_map": "MapClass", "guatiao_list": "ListClass"}

func (p *pkgInfo) gt(f *ast.File) string {
	if name := p.binding[f]; name != "" {
		return name
	}
	return "guatiao"
}

// objectKindOf is the descriptor expression of the object kind a Go type
// names, or "".
func (p *pkgInfo) objectKindOf(t string) string {
	if d, ok := p.byIface[t]; ok {
		_ = d
		return t + "Kind"
	}
	if i := strings.IndexByte(t, '.'); i > 0 && !strings.HasPrefix(t, "*") && p.imports[t[:i]] != bindingPath {
		return t + "Kind"
	}
	return ""
}

func (p *pkgInfo) treeView(t string, f *ast.File) string {
	gt := p.gt(f)
	switch t {
	case gt + ".Ref":
		return "Ref"
	case gt + ".MapRef":
		return "MapRef"
	case gt + ".ListRef":
		return "ListRef"
	case "*" + gt + ".Value":
		return "Value"
	}
	return ""
}

func (p *pkgInfo) plan(k *kindDecl, t *cTable) (*kindPlan, error) {
	plan := &kindPlan{decl: k, table: t}
	where := func(s string) string { return k.iface + " (" + t.name + "." + s + ")" }
	if t.object {
		closer, ok := k.required["Close"]
		if !ok {
			return nil, fmt.Errorf("%s is an object kind (destroy first): its interface has Close() error", k.iface)
		}
		if len(closer.params) != 0 || len(closer.results) != 1 || closer.results[0] != "error" {
			return nil, fmt.Errorf("%s.Close is Close() error", k.iface)
		}
	}
	for i, cs := range t.slots {
		goName := camel(cs.name)
		sp := &slotPlan{c: cs, index: i}
		if m, ok := k.required[goName]; ok {
			sp.method, sp.required = m, true
		} else {
			found := p.methods[goName]
			var outside []*goMethod
			for _, m := range found {
				if _, isKind := p.byIface[m.iface]; !isKind {
					outside = append(outside, m)
				}
			}
			switch len(outside) {
			case 0:
				return nil, fmt.Errorf("%s: no interface in the package has %s, and every slot needs one", where(cs.name), goName)
			case 1:
				sp.method = outside[0]
			default:
				return nil, fmt.Errorf("%s: %d interfaces have %s; say it once", where(cs.name), len(outside), goName)
			}
		}
		if cs.ret != "guatiao_status" {
			return nil, fmt.Errorf("%s answers %s, not a guatiao_status", where(cs.name), cs.ret)
		}
		params := cs.params[1:]
		if n := len(params); n > 0 && params[n-1].typ == "guatiao_provider_error *" {
			sp.fallible = true
			params = params[:n-1]
		}
		var out *cParam
		if n := len(params); n > 0 && params[n-1].name == "out" {
			out = &params[n-1]
			params = params[:n-1]
		}
		m := sp.method
		if len(m.params) != len(params) {
			return nil, fmt.Errorf("%s: %s takes %d arguments, the slot %d", where(cs.name), m.name, len(m.params), len(params))
		}
		for j, cp := range params {
			a, err := p.argOf(cp, m.params[j], cs.lent[cp.name], m.file)
			if err != nil {
				return nil, fmt.Errorf("%s, %s: %w", where(cs.name), cp.name, err)
			}
			sp.args = append(sp.args, a)
		}
		wantResults := 1
		if out != nil {
			wantResults = 2
			a, err := p.answerOf(*out, m.results[0], m.file)
			if err != nil {
				if len(m.results) < 2 {
					return nil, fmt.Errorf("%s: %s answers (%s, error)", where(cs.name), m.name, out.typ)
				}
				return nil, fmt.Errorf("%s, the answer: %w", where(cs.name), err)
			}
			sp.ans = a
		}
		if len(m.results) != wantResults || m.results[len(m.results)-1] != "error" {
			return nil, fmt.Errorf("%s: %s returns %s, and every slot's method returns an error last",
				where(cs.name), m.name, map[bool]string{true: "(answer, error)", false: "error"}[out != nil])
		}
		plan.slots = append(plan.slots, sp)
	}
	return plan, nil
}

func (p *pkgInfo) argOf(cp cParam, gp goParam, lent bool, f *ast.File) (arg, error) {
	a := arg{cType: cp.typ, cName: cp.name, goName: gp.name, goType: gp.typ}
	switch cp.typ {
	case "guatiao_str":
		a.kind = aStr
		return a, want(gp.typ, "string")
	case "guatiao_bytes":
		a.kind = aBytes
		return a, want(gp.typ, "[]byte")
	case "guatiao_bytes_mut":
		a.kind = aBytesMut
		return a, want(gp.typ, "[]byte")
	case "guatiao_object":
		a.kind = aObjTake
		if a.objKind = p.objectKindOf(gp.typ); a.objKind == "" {
			return a, fmt.Errorf("an object handed over is an object kind's interface, not %s", gp.typ)
		}
		return a, nil
	case "const guatiao_object *":
		a.kind = aObjLend
		if a.objKind = p.objectKindOf(gp.typ); a.objKind == "" {
			return a, fmt.Errorf("an object lent is an object kind's interface, not %s", gp.typ)
		}
		return a, nil
	}
	if goT, ok := scalars[cp.typ]; ok {
		a.kind = aScalar
		return a, want(gp.typ, goT)
	}
	base, constant := strings.CutPrefix(cp.typ, "const ")
	base, isPtr := strings.CutSuffix(base, " *")
	class, isTree := trees[base]
	if !isTree || !isPtr {
		return a, fmt.Errorf("no Go form crosses as %s", cp.typ)
	}
	a.class = class
	a.view = p.treeView(gp.typ, f)
	switch {
	case constant:
		a.kind = aBorrow
		if a.view == "Value" {
			return a, fmt.Errorf("a borrowed tree is a view or a Go type it decodes into, not a *Value")
		}
	case lent:
		a.kind = aLend
		if a.view == "" || a.view == "Value" {
			return a, fmt.Errorf("a lent tree is a Ref, MapRef or ListRef, written where it stands; not %s", gp.typ)
		}
	default:
		a.kind = aTake
		if a.view != "" && a.view != "Value" {
			return a, fmt.Errorf("a tree handed over is a *Value the method owns or a Go type it decodes into; not a view (%s)", gp.typ)
		}
	}
	if a.view != "" && a.view != "Value" && a.view != classView(class) && a.view != "Ref" {
		return a, fmt.Errorf("a %s is a %s or a Ref, not %s", strings.TrimSuffix(class, "Class"), classView(class), gp.typ)
	}
	return a, nil
}

func classView(class string) string {
	switch class {
	case "MapClass":
		return "MapRef"
	case "ListClass":
		return "ListRef"
	}
	return "Ref"
}

func (p *pkgInfo) answerOf(cp cParam, goType string, f *ast.File) (answer, error) {
	a := answer{cType: cp.typ, goType: goType}
	switch cp.typ {
	case "guatiao_string *":
		a.kind = nText
		return a, want(goType, "string")
	case "guatiao_object *":
		a.kind = nObject
		if a.objKind = p.objectKindOf(goType); a.objKind == "" {
			return a, fmt.Errorf("an object answered is an object kind's interface, not %s", goType)
		}
		return a, nil
	}
	base, isPtr := strings.CutSuffix(cp.typ, " *")
	if !isPtr {
		return a, fmt.Errorf("the out argument %s is not a pointer", cp.typ)
	}
	if goT, ok := scalars[base]; ok {
		a.kind = nScalar
		a.cType = base
		return a, want(goType, goT)
	}
	class, ok := trees[base]
	if !ok {
		return a, fmt.Errorf("no Go form is answered as %s", cp.typ)
	}
	a.kind, a.class = nTree, class
	a.view = p.treeView(goType, f)
	if a.view != "" && a.view != "Value" {
		return a, fmt.Errorf("a tree answered is a *Value or a Go type, not a view (%s)", goType)
	}
	return a, nil
}

func want(got, wanted string) error {
	if got == wanted || (wanted == "uint8" && got == "byte") {
		return nil
	}
	return fmt.Errorf("the Go type is %s; this argument crosses as %s", got, wanted)
}

// ---- emitting --------------------------------------------------------------------------

func (p *pkgInfo) cFunc(k *kindPlan, s *slotPlan, what string) string {
	return fmt.Sprintf("%s_%s_%s_%s", p.prefix, what, ident(k.decl.kind), ident(s.c.name))
}

func (p *pkgInfo) export(k *kindPlan, s *slotPlan) string {
	return fmt.Sprintf("%s_%s_%s", p.prefix, ident(k.decl.kind), ident(s.c.name))
}

// callParams are a call trampoline's parameters after fn and ctx, as C.
func callParams(s *slotPlan) []string {
	var out []string
	for i, a := range s.args {
		switch a.kind {
		case aStr, aBytes:
			out = append(out, fmt.Sprintf("const void *a%d", i), fmt.Sprintf("size_t a%dn", i))
		case aBytesMut:
			out = append(out, fmt.Sprintf("void *a%d", i), fmt.Sprintf("size_t a%dn", i))
		case aScalar:
			out = append(out, fmt.Sprintf("%s a%d", a.cType, i))
		case aObjTake:
			out = append(out, fmt.Sprintf("const void *a%dt", i), fmt.Sprintf("size_t a%ds", i), fmt.Sprintf("uintptr_t a%dc", i))
		default:
			out = append(out, fmt.Sprintf("void *a%d", i))
		}
	}
	if s.ans.kind != nNone {
		out = append(out, "void *out")
	}
	if s.fallible {
		out = append(out, "void *err")
	}
	return out
}

func (p *pkgInfo) emitH(plans []*kindPlan, header, out string) []byte {
	var b bytes.Buffer
	fmt.Fprintf(&b, "// Code generated by guatiao-kind from %s. DO NOT EDIT.\n\n", filepath.ToSlash(header))
	guard := strings.ToUpper(ident(p.prefix + "_" + out + "_h"))
	fmt.Fprintf(&b, "#ifndef %s\n#define %s\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n\n", guard, guard)
	for _, k := range plans {
		for _, s := range k.slots {
			params := append([]string{"void *fn", "uintptr_t ctx"}, callParams(s)...)
			fmt.Fprintf(&b, "uint32_t %s(%s);\n", p.cFunc(k, s, "call"), strings.Join(params, ", "))
			fmt.Fprintf(&b, "void *%s(void);\n", p.cFunc(k, s, "serve"))
		}
	}
	fmt.Fprintf(&b, "\n#endif\n")
	return b.Bytes()
}

func (p *pkgInfo) emitC(plans []*kindPlan, header, out string) []byte {
	var b bytes.Buffer
	fmt.Fprintf(&b, "// Code generated by guatiao-kind from %s. DO NOT EDIT.\n\n", filepath.ToSlash(header))
	b.WriteString(`#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "` + out + `.h"
#include "_cgo_export.h"

/* The header's types, declared with its tags and members, so a slot is
 * called through the type it has. */
typedef uint32_t guatiao_status;
typedef struct guatiao_str { const uint8_t *ptr; size_t len; } guatiao_str;
typedef struct guatiao_bytes { const uint8_t *ptr; size_t len; } guatiao_bytes;
typedef struct guatiao_bytes_mut { uint8_t *ptr; size_t len; } guatiao_bytes_mut;
typedef struct guatiao_object { const void *table; size_t size; void *ctx; } guatiao_object;
typedef struct guatiao_value guatiao_value;
typedef struct guatiao_map guatiao_map;
typedef struct guatiao_list guatiao_list;
typedef struct guatiao_string guatiao_string;
typedef struct guatiao_provider_error guatiao_provider_error;
`)
	for _, k := range plans {
		for _, s := range k.slots {
			// The slot's own type, from the header.
			var real []string
			for _, cp := range s.c.params {
				real = append(real, cp.typ+" "+cp.name)
			}
			slotType := fmt.Sprintf("guatiao_status (*)(%s)", strings.Join(real, ", "))
			fmt.Fprintf(&b, "\n/* %s.%s */\n", k.table.name, s.c.name)
			// Calling a table's slot.
			params := append([]string{"void *fn", "uintptr_t ctx"}, callParams(s)...)
			fmt.Fprintf(&b, "uint32_t %s(%s) {\n", p.cFunc(k, s, "call"), strings.Join(params, ", "))
			var callArgs []string
			callArgs = append(callArgs, "(void *)ctx")
			for i, a := range s.args {
				switch a.kind {
				case aStr:
					fmt.Fprintf(&b, "  guatiao_str s%d = {(const uint8_t *)a%d, a%dn};\n", i, i, i)
					callArgs = append(callArgs, fmt.Sprintf("s%d", i))
				case aBytes:
					fmt.Fprintf(&b, "  guatiao_bytes s%d = {(const uint8_t *)a%d, a%dn};\n", i, i, i)
					callArgs = append(callArgs, fmt.Sprintf("s%d", i))
				case aBytesMut:
					fmt.Fprintf(&b, "  guatiao_bytes_mut s%d = {(uint8_t *)a%d, a%dn};\n", i, i, i)
					callArgs = append(callArgs, fmt.Sprintf("s%d", i))
				case aObjTake:
					fmt.Fprintf(&b, "  guatiao_object s%d = {a%dt, a%ds, (void *)a%dc};\n", i, i, i, i)
					callArgs = append(callArgs, fmt.Sprintf("s%d", i))
				case aScalar:
					callArgs = append(callArgs, fmt.Sprintf("a%d", i))
				default:
					callArgs = append(callArgs, fmt.Sprintf("(%s)a%d", a.cType, i))
				}
			}
			if s.ans.kind != nNone {
				callArgs = append(callArgs, fmt.Sprintf("(%s)out", s.c.params[len(s.args)+1].typ))
			}
			if s.fallible {
				callArgs = append(callArgs, "(guatiao_provider_error *)err")
			}
			fmt.Fprintf(&b, "  return ((%s)fn)(%s);\n}\n", slotType, strings.Join(callArgs, ", "))
			// Serving a slot: unpack and hand to the Go export.
			tramp := p.cFunc(k, s, "tramp")
			fmt.Fprintf(&b, "static guatiao_status %s(%s) {\n", tramp, strings.Join(real, ", "))
			goArgs := []string{"(uintptr_t)" + s.c.params[0].name}
			for i, a := range s.args {
				n := s.c.params[i+1].name
				switch a.kind {
				case aStr, aBytes, aBytesMut:
					goArgs = append(goArgs, fmt.Sprintf("(void *)%s.ptr", n), n+".len")
				case aObjTake:
					goArgs = append(goArgs, fmt.Sprintf("(void *)%s.table", n), n+".size", fmt.Sprintf("(uintptr_t)%s.ctx", n))
				case aScalar:
					goArgs = append(goArgs, n)
				default:
					goArgs = append(goArgs, "(void *)"+n)
				}
			}
			if s.ans.kind != nNone {
				goArgs = append(goArgs, "(void *)"+s.c.params[len(s.args)+1].name)
			}
			if s.fallible {
				goArgs = append(goArgs, "(void *)"+s.c.params[len(s.c.params)-1].name)
			}
			fmt.Fprintf(&b, "  return %s(%s);\n}\n", p.export(k, s), strings.Join(goArgs, ", "))
			fmt.Fprintf(&b, "void *%s(void) { return (void *)%s; }\n", p.cFunc(k, s, "serve"), tramp)
		}
	}
	return b.Bytes()
}

func (p *pkgInfo) emitGo(plans []*kindPlan, header, out string) ([]byte, error) {
	var b bytes.Buffer
	gt := "guatiao"
	fmt.Fprintf(&b, "// Code generated by guatiao-kind from %s. DO NOT EDIT.\n\n", filepath.ToSlash(header))
	fmt.Fprintf(&b, "package %s\n\n", p.name)
	fmt.Fprintf(&b, "/*\n#include %q\n*/\nimport \"C\"\n\n", out+".h")
	// Imports: the binding, reflect, unsafe, and whatever the signatures name.
	used := map[string]bool{}
	for _, k := range plans {
		for _, s := range k.slots {
			for _, prm := range s.method.params {
				collectSelectors(prm.expr, used)
			}
			collectText(s.ans.goType, used)
		}
	}
	var imports []string
	for local := range used {
		path, ok := p.imports[local]
		if !ok || path == bindingPath {
			continue
		}
		name := path[strings.LastIndex(path, "/")+1:]
		if name == local {
			imports = append(imports, strconv.Quote(path))
		} else {
			imports = append(imports, local+" "+strconv.Quote(path))
		}
	}
	sort.Strings(imports)
	b.WriteString("import (\n\t\"reflect\"\n\t\"unsafe\"\n\n")
	fmt.Fprintf(&b, "\t%s %q\n", gt, bindingPath)
	for _, imp := range imports {
		fmt.Fprintf(&b, "\t%s\n", imp)
	}
	b.WriteString(")\n\n")
	b.WriteString("var _ unsafe.Pointer\n\n")
	for _, k := range plans {
		p.emitKind(&b, k)
	}
	src, err := format.Source(b.Bytes())
	if err != nil {
		return b.Bytes(), fmt.Errorf("the generated Go does not parse (%v); this is a bug in guatiao-kind", err)
	}
	return src, nil
}

func collectSelectors(e ast.Expr, used map[string]bool) {
	ast.Inspect(e, func(n ast.Node) bool {
		if sel, ok := n.(*ast.SelectorExpr); ok {
			if id, ok := sel.X.(*ast.Ident); ok {
				used[id.Name] = true
			}
		}
		return true
	})
}

func collectText(t string, used map[string]bool) {
	if e, err := parser.ParseExpr(t); err == nil {
		collectSelectors(e, used)
	}
}

func (p *pkgInfo) emitKind(b *bytes.Buffer, k *kindPlan) {
	iface := k.decl.iface
	remote := iface + "Remote"
	kindVar := iface + "Kind"
	gt := "guatiao"
	fmt.Fprintf(b, "// %s is the %s kind (%s), as a host binds it and a provider names it.\n", kindVar, k.decl.kind, k.table.name)
	fmt.Fprintf(b, "var %s = %s.NewKind(%s.KindSpec{\n\tName: %q,\n\tFloorHash: %d,\n\tObject: %t,\n\tSlots: []%s.SlotSpec{\n",
		kindVar, gt, gt, k.decl.kind, k.table.floorHash, k.table.object, gt)
	for _, s := range k.slots {
		fmt.Fprintf(b, "\t\t{Name: %q, Required: %t, Fallible: %t, Serve: C.%s(), Iface: reflect.TypeFor[%s]()},\n",
			s.c.name, s.required, s.fallible, p.cFunc(k, s, "serve"), s.method.iface)
	}
	fmt.Fprintf(b, "\t},\n}, func(r %s.Remote) *%s { return &%s{Remote: r} })\n\n", gt, remote, remote)
	fmt.Fprintf(b, "// %s is a caller's handle on a %s table: a method per slot. A slot\n// the table lacks answers guatiao.ErrUnsupported.\n", remote, k.table.name)
	fmt.Fprintf(b, "type %s struct{ %s.Remote }\n\n", remote, gt)
	ifaces := map[string]bool{iface: true}
	for _, s := range k.slots {
		ifaces[s.method.iface] = true
	}
	var names []string
	for n := range ifaces {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		fmt.Fprintf(b, "var _ %s = (*%s)(nil)\n", n, remote)
	}
	b.WriteString("\n")
	for _, s := range k.slots {
		p.emitCall(b, k, s)
		p.emitServe(b, k, s)
	}
}

// generatedName reports whether a parameter's name is one the generated
// method uses for itself, and so must be spelled otherwise there.
func generatedName(name string) bool {
	switch name {
	case "r", "c", "err", "out", "errp", "st", "res", "zero", "rem", "_":
		return true
	}
	if len(name) > 1 && strings.ContainsRune("pntsx", rune(name[0])) {
		_, err := strconv.Atoi(name[1:])
		return err == nil
	}
	return false
}

func (p *pkgInfo) emitCall(b *bytes.Buffer, k *kindPlan, s *slotPlan) {
	gt := "guatiao"
	m := *s.method
	m.params = append([]goParam(nil), m.params...)
	for i := range m.params {
		for generatedName(m.params[i].name) {
			m.params[i].name += "_"
		}
	}
	var params []string
	for _, prm := range m.params {
		params = append(params, prm.name+" "+prm.typ)
	}
	results := "error"
	if s.ans.kind != nNone {
		results = "(" + s.ans.goType + ", error)"
	}
	fail := "return err"
	if s.ans.kind != nNone {
		fail = "return zero, err"
	}
	fmt.Fprintf(b, "// %s calls the %s slot.\n", m.name, s.c.name)
	fmt.Fprintf(b, "func (r *%sRemote) %s(%s) %s {\n", k.decl.iface, m.name, strings.Join(params, ", "), results)
	if s.ans.kind != nNone {
		fmt.Fprintf(b, "\tvar zero %s\n", s.ans.goType)
	}
	fmt.Fprintf(b, "\tc, err := r.Begin(%d)\n\tif err != nil {\n\t\t%s\n\t}\n\tdefer c.End()\n", s.index, fail)
	var cargs []string
	conv := func(i int, a arg) {
		name := m.params[i].name
		switch a.kind {
		case aStr:
			fmt.Fprintf(b, "\tp%d, n%d := c.Str(%s)\n", i, i, name)
			cargs = append(cargs, fmt.Sprintf("p%d", i), fmt.Sprintf("C.size_t(n%d)", i))
		case aBytes, aBytesMut:
			fmt.Fprintf(b, "\tp%d, n%d := c.Bytes(%s)\n", i, i, name)
			cargs = append(cargs, fmt.Sprintf("p%d", i), fmt.Sprintf("C.size_t(n%d)", i))
		case aScalar:
			cargs = append(cargs, fmt.Sprintf("%s(%s)", cgoScalars[a.cType], name))
		case aBorrow, aLend, aTake:
			fn := map[argKind]string{aBorrow: "Borrow", aLend: "Lend", aTake: "Take"}[a.kind]
			fmt.Fprintf(b, "\tp%d, err := c.%s(%s, %s.%s)\n\tif err != nil {\n\t\t%s\n\t}\n", i, fn, name, gt, a.class, fail)
			cargs = append(cargs, fmt.Sprintf("p%d", i))
		case aObjLend:
			fmt.Fprintf(b, "\tp%d, err := c.LendObject(%s, %s)\n\tif err != nil {\n\t\t%s\n\t}\n", i, name, a.objKind, fail)
			cargs = append(cargs, fmt.Sprintf("p%d", i))
		case aObjTake:
			fmt.Fprintf(b, "\tt%d, s%d, x%d, err := c.TakeObject(%s, %s)\n\tif err != nil {\n\t\t%s\n\t}\n", i, i, i, name, a.objKind, fail)
			cargs = append(cargs, fmt.Sprintf("t%d", i), fmt.Sprintf("C.size_t(s%d)", i), fmt.Sprintf("C.uintptr_t(x%d)", i))
		}
	}
	// What is handed over last: a conversion that fails before it leaves
	// the caller's trees and objects where they were.
	order := make([]int, 0, len(s.args))
	for i, a := range s.args {
		if a.kind != aTake && a.kind != aObjTake {
			order = append(order, i)
		}
	}
	for i, a := range s.args {
		if a.kind == aTake || a.kind == aObjTake {
			order = append(order, i)
		}
	}
	slots := make([][]string, len(s.args))
	for _, i := range order {
		before := len(cargs)
		conv(i, s.args[i])
		slots[i] = append([]string(nil), cargs[before:]...)
	}
	cargs = cargs[:0]
	for _, sl := range slots {
		cargs = append(cargs, sl...)
	}
	switch s.ans.kind {
	case nTree:
		fmt.Fprintf(b, "\tout := c.Out(%s.%s)\n", gt, s.ans.class)
	case nText:
		b.WriteString("\tout := c.OutText()\n")
	case nScalar:
		b.WriteString("\tout := c.OutScalar()\n")
	case nObject:
		b.WriteString("\tout := c.OutObject()\n")
	}
	if s.ans.kind != nNone {
		cargs = append(cargs, "out")
	}
	if s.fallible {
		b.WriteString("\terrp := c.Err()\n")
		cargs = append(cargs, "errp")
	}
	fmt.Fprintf(b, "\tst := C.%s(c.Fn(), C.uintptr_t(c.Ctx())", p.cFunc(k, s, "call"))
	for _, a := range cargs {
		b.WriteString(", " + a)
	}
	b.WriteString(")\n")
	fmt.Fprintf(b, "\tif err := c.Status(uint32(st)); err != nil {\n\t\t%s\n\t}\n", fail)
	switch s.ans.kind {
	case nNone:
		b.WriteString("\treturn nil\n")
	case nTree:
		fmt.Fprintf(b, "\tvar res %s\n\tif err := c.Answer(out, %s.%s, &res); err != nil {\n\t\treturn zero, err\n\t}\n\treturn res, nil\n", s.ans.goType, gt, s.ans.class)
	case nText:
		b.WriteString("\treturn c.AnswerText(out), nil\n")
	case nScalar:
		fmt.Fprintf(b, "\treturn %s(*(*%s)(out)), nil\n", s.ans.goType, cgoScalars[s.ans.cType])
	case nObject:
		kindVar := s.ans.objKind
		wrapper := strings.TrimSuffix(kindVar, "Kind")
		fmt.Fprintf(b, "\trem, err := c.AnswerObject(out, %s)\n\tif err != nil {\n\t\treturn zero, err\n\t}\n", kindVar)
		fmt.Fprintf(b, "\treturn &%sRemote{Remote: rem}, nil\n", wrapper)
	}
	b.WriteString("}\n\n")
}

func (p *pkgInfo) emitServe(b *bytes.Buffer, k *kindPlan, s *slotPlan) {
	gt := "guatiao"
	m := s.method
	var params []string
	params = append(params, "ctx C.uintptr_t")
	for i, a := range s.args {
		switch a.kind {
		case aStr, aBytes, aBytesMut:
			params = append(params, fmt.Sprintf("a%d unsafe.Pointer", i), fmt.Sprintf("a%dn C.size_t", i))
		case aScalar:
			params = append(params, fmt.Sprintf("a%d %s", i, cgoScalars[a.cType]))
		case aObjTake:
			params = append(params, fmt.Sprintf("a%dt unsafe.Pointer", i), fmt.Sprintf("a%ds C.size_t", i), fmt.Sprintf("a%dc C.uintptr_t", i))
		default:
			params = append(params, fmt.Sprintf("a%d unsafe.Pointer", i))
		}
	}
	if s.ans.kind != nNone {
		params = append(params, "out unsafe.Pointer")
	}
	errArg := "nil"
	if s.fallible {
		params = append(params, "errp unsafe.Pointer")
		errArg = "errp"
	}
	name := p.export(k, s)
	fmt.Fprintf(b, "//export %s\nfunc %s(%s) C.uint32_t {\n", name, name, strings.Join(params, ", "))
	fmt.Fprintf(b, "\treturn C.uint32_t(%sKind.Serve(uintptr(ctx), %s, %d, func(s *%s.Served) error {\n", k.decl.iface, errArg, s.index, gt)
	recv := func(i int, a arg) {
		v := fmt.Sprintf("v%d", i)
		switch a.kind {
		case aStr:
			fmt.Fprintf(b, "\t\t%s, err := s.Str(a%d, uintptr(a%dn))\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n", v, i, i)
		case aBytes:
			fmt.Fprintf(b, "\t\t%s := s.Bytes(a%d, uintptr(a%dn))\n", v, i, i)
		case aBytesMut:
			fmt.Fprintf(b, "\t\t%s := s.BytesMut(a%d, uintptr(a%dn))\n", v, i, i)
		case aScalar:
			fmt.Fprintf(b, "\t\t%s := %s(a%d)\n", v, a.goType, i)
		case aBorrow, aLend:
			fn := map[argKind]string{aBorrow: "Borrowed", aLend: "Lent"}[a.kind]
			if a.view == "" {
				fmt.Fprintf(b, "\t\tvar %s %s\n\t\tif err := s.Decode(a%d, %s.%s, false, &%s); err != nil {\n\t\t\treturn err\n\t\t}\n", v, a.goType, i, gt, a.class, v)
				return
			}
			fmt.Fprintf(b, "\t\tr%d, err := s.%s(a%d, %s.%s)\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n", i, fn, i, gt, a.class)
			switch a.view {
			case "Ref":
				fmt.Fprintf(b, "\t\t%s := r%d\n", v, i)
			default:
				fmt.Fprintf(b, "\t\t%s := %s.%s{Ref: r%d}\n", v, gt, a.view, i)
			}
		case aTake:
			if a.view == "Value" {
				fmt.Fprintf(b, "\t\t%s, err := s.Taken(a%d, %s.%s)\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n", v, i, gt, a.class)
				return
			}
			fmt.Fprintf(b, "\t\tvar %s %s\n\t\tif err := s.Decode(a%d, %s.%s, true, &%s); err != nil {\n\t\t\treturn err\n\t\t}\n", v, a.goType, i, gt, a.class, v)
		case aObjTake:
			wrapper := strings.TrimSuffix(a.objKind, "Kind")
			fmt.Fprintf(b, "\t\tr%d, err := s.TakenObject(a%dt, uintptr(a%ds), uintptr(a%dc), %s)\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\t%s := &%sRemote{Remote: r%d}\n", i, i, i, i, a.objKind, v, wrapper, i)
		case aObjLend:
			wrapper := strings.TrimSuffix(a.objKind, "Kind")
			fmt.Fprintf(b, "\t\tr%d, err := s.LentObject(a%d, %s)\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\t%s := &%sRemote{Remote: r%d}\n", i, i, a.objKind, v, wrapper, i)
		}
	}
	// What was handed over is received first, so a refusal below still
	// owns it, and frees it, rather than leaking it.
	for i, a := range s.args {
		if a.kind == aTake || a.kind == aObjTake {
			recv(i, a)
		}
	}
	fmt.Fprintf(b, "\t\tt, err := s.Target()\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n")
	fmt.Fprintf(b, "\t\ttarget, ok := t.(%s)\n\t\tif !ok {\n\t\t\treturn %s.Errorf(%s.Unsupported, \"%%T does not implement %s\", t)\n\t\t}\n", m.iface, gt, gt, m.iface)
	for i, a := range s.args {
		if a.kind != aTake && a.kind != aObjTake {
			recv(i, a)
		}
	}
	var call []string
	for i := range s.args {
		call = append(call, fmt.Sprintf("v%d", i))
	}
	b.WriteString("\t\ts.Ran()\n")
	invoke := fmt.Sprintf("target.%s(%s)", m.name, strings.Join(call, ", "))
	switch s.ans.kind {
	case nNone:
		fmt.Fprintf(b, "\t\treturn %s\n", invoke)
	case nTree:
		fmt.Fprintf(b, "\t\tres, err := %s\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\treturn s.Answer(out, %s.%s, res)\n", invoke, gt, s.ans.class)
	case nText:
		fmt.Fprintf(b, "\t\tres, err := %s\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\treturn s.AnswerText(out, res)\n", invoke)
	case nScalar:
		fmt.Fprintf(b, "\t\tres, err := %s\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\tif out == nil {\n\t\t\treturn %s.Errorf(%s.NullArgument, \"no slot for the answer\")\n\t\t}\n\t\t*(*%s)(out) = %s(res)\n\t\treturn nil\n",
			invoke, gt, gt, cgoScalars[s.ans.cType], cgoScalars[s.ans.cType])
	case nObject:
		fmt.Fprintf(b, "\t\tres, err := %s\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}\n\t\treturn s.AnswerObject(out, %s, res)\n", invoke, s.ans.objKind)
	}
	b.WriteString("\t}))\n}\n\n")
}
