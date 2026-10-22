// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"maps"
	"reflect"
	"runtime"
	"runtime/debug"
	"strings"
	"sync"
	"sync/atomic"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Library is a library written in Go: its providers and the descriptor a
// host reads, built on first use in memory kept for the process. A host
// in this process registers it with Registry.RegisterLibrary; a library
// built with -buildmode=c-shared answers its guatiao_library_entry with
// Answer, and is then a file any host's scan finds.
type Library struct {
	id, version string
	providers   []*providerSpec
	ids         []string
	meta        map[string]any
	cshared     bool

	mu        sync.Mutex
	built     bool
	buildErr  error
	info      unsafe.Pointer
	index     int
	values    []*Value
	defaults  []uintptr
	instances map[uintptr]bool
	closed    atomic.Bool
}

// LibraryPart is a provider, or something said about the library: a
// Provider is one, and so are LibraryID, LibraryVersion, Summary,
// License, Authors, URL, Notice and LibraryMeta.
type LibraryPart interface{ applyLibrary(*libSpec) }

type libSpec struct {
	id, version string
	providers   []Provider
	meta        map[string]any
	notices     []any
	urls        map[string]any
}

func (p Provider) applyLibrary(s *libSpec) { s.providers = append(s.providers, p) }

type libraryFunc func(*libSpec)

func (f libraryFunc) applyLibrary(s *libSpec) { f(s) }

func metaSet(key string, v any) LibraryPart {
	return libraryFunc(func(s *libSpec) {
		if s.meta == nil {
			s.meta = map[string]any{}
		}
		s.meta[key] = v
	})
}

// LibraryID is the library's id, in place of the one LibraryOf reads.
func LibraryID(id string) LibraryPart { return libraryFunc(func(s *libSpec) { s.id = id }) }

// LibraryVersion is the library's version (semver), in place of the one
// LibraryOf reads from the build.
func LibraryVersion(v string) LibraryPart {
	return libraryFunc(func(s *libSpec) { s.version = v })
}

// Summary is a one-line description, the library's meta `summary`.
func Summary(text string) LibraryPart { return metaSet("summary", text) }

// License is the library's licence, as an SPDX expression.
func License(spdx string) LibraryPart { return metaSet("license", spdx) }

// Authors are who wrote it, the library's meta `authors`.
func Authors(names ...string) LibraryPart { return metaSet("authors", names) }

// URL adds one of the library's links under label (Homepage, Repository,
// Issues), the library's meta `urls`.
func URL(label, url string) LibraryPart {
	return libraryFunc(func(s *libSpec) {
		if s.urls == nil {
			s.urls = map[string]any{}
		}
		s.urls[label] = url
	})
}

// Notice adds a legal notice, typically a licence file the package
// embeds, the library's meta `notices`.
func Notice(component, text string) LibraryPart {
	return libraryFunc(func(s *libSpec) {
		s.notices = append(s.notices, map[string]any{"component": component, "text": text, "format": "text"})
	})
}

// LibraryMeta adds every key of meta to what the library says about
// itself.
func LibraryMeta(meta map[string]any) LibraryPart {
	return libraryFunc(func(s *libSpec) {
		if s.meta == nil {
			s.meta = map[string]any{}
		}
		maps.Copy(s.meta, meta)
	})
}

// NewLibrary is a library of the providers among parts, with the id and
// version given: the primitive LibraryOf builds on, for a test or a
// library made at run time. It refuses a provider whose value does not
// implement a kind it names, or two providers sharing an id.
func NewLibrary(id, version string, parts ...LibraryPart) (*Library, error) {
	spec := libSpec{id: id, version: version}
	for _, p := range parts {
		if p != nil {
			p.applyLibrary(&spec)
		}
	}
	return newLibrary(spec, false)
}

func newLibrary(spec libSpec, cshared bool) (*Library, error) {
	if spec.id == "" {
		return nil, fmt.Errorf("guatiao: a library has an id")
	}
	if len(spec.providers) == 0 {
		return nil, fmt.Errorf("guatiao: the library %s offers no provider", spec.id)
	}
	lib := &Library{id: spec.id, version: spec.version, cshared: cshared, instances: map[uintptr]bool{}}
	seen := map[string]bool{}
	for _, p := range spec.providers {
		s := p.spec
		if s == nil {
			return nil, fmt.Errorf("guatiao: the library %s has an empty Provider", spec.id)
		}
		if err := s.check(); err != nil {
			return nil, err
		}
		id := s.id
		if id == "" {
			id = idStem(spec.id) + "_" + s.name
		}
		if seen[id] {
			return nil, fmt.Errorf("guatiao: two providers of %s share the id %s", spec.id, id)
		}
		seen[id] = true
		lib.providers = append(lib.providers, s)
		lib.ids = append(lib.ids, id)
	}
	if spec.meta != nil || spec.urls != nil || spec.notices != nil {
		lib.meta = map[string]any{}
		maps.Copy(lib.meta, spec.meta)
		if spec.urls != nil {
			lib.meta["urls"] = spec.urls
		}
		if spec.notices != nil {
			lib.meta["notices"] = spec.notices
		}
	}
	return lib, nil
}

// idStem is the last element of a library id, spelled as an identifier:
// what a provider's default id starts with.
func idStem(id string) string {
	stem := id[strings.LastIndexAny(id, "/\\")+1:]
	b := []byte(stem)
	for i, c := range b {
		if !(c == '_' || c >= '0' && c <= '9' || c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z') {
			b[i] = '_'
		}
	}
	return string(b)
}

// LibraryOf is the library the providers' Go package is: its id is the
// package's import path (the main package's path, as the build records
// it, for package main), its version that of the module holding the
// package, as the build records it, the leading v dropped. A build that
// names no version, (devel) or a test binary, is refused unless
// LibraryVersion says one. Its meta is the module's path and, for the main
// module, the version control revision, time and whether the tree was
// modified, as the build recorded them; Summary, License, Authors, URL,
// Notice and LibraryMeta add the rest. Every provider must come from one
// package.
func LibraryOf(parts ...LibraryPart) (*Library, error) {
	spec := libSpec{}
	for _, p := range parts {
		if p != nil {
			p.applyLibrary(&spec)
		}
	}
	if len(spec.providers) == 0 {
		return nil, fmt.Errorf("guatiao: a library offers a provider")
	}
	pkg := ""
	for _, p := range spec.providers {
		if p.spec == nil {
			return nil, fmt.Errorf("guatiao: an empty Provider")
		}
		path := p.spec.pkgPath()
		if path == "" {
			return nil, fmt.Errorf("guatiao: the provider %s comes from no package this can name", p.spec.name)
		}
		if pkg != "" && path != pkg {
			return nil, fmt.Errorf("guatiao: a library is one package, and these providers come from %s and %s", pkg, path)
		}
		pkg = path
	}
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return nil, fmt.Errorf("guatiao: this build carries no build information; use NewLibrary")
	}
	id := pkg
	module := &info.Main
	if pkg == "main" {
		id = info.Path
	} else {
		module = moduleOf(info, pkg)
	}
	if spec.id == "" {
		spec.id = id
	}
	built := map[string]any{}
	version := ""
	if module != nil {
		built["module"] = module.Path
		version = module.Version
		if module.Replace != nil {
			version = module.Replace.Version
		}
		if module == &info.Main {
			if vcs := vcsOf(info); vcs != nil {
				built["vcs"] = vcs
			}
		}
	}
	if spec.version == "" {
		if version == "" || version == "(devel)" {
			return nil, fmt.Errorf("guatiao: the build of %s names no version (%q); say one with guatiao.LibraryVersion", id, version)
		}
		spec.version = strings.TrimPrefix(version, "v")
	}
	if spec.meta == nil {
		spec.meta = map[string]any{}
	}
	for k, v := range built {
		if _, set := spec.meta[k]; !set {
			spec.meta[k] = v
		}
	}
	cshared := false
	for _, s := range info.Settings {
		if s.Key == "-buildmode" && s.Value == "c-shared" {
			cshared = true
		}
	}
	return newLibrary(spec, cshared)
}

func moduleOf(info *debug.BuildInfo, pkg string) *debug.Module {
	under := func(m *debug.Module) bool {
		return m.Path != "" && (pkg == m.Path || strings.HasPrefix(pkg, m.Path+"/") || strings.HasPrefix(pkg, m.Path+"_test"))
	}
	var best *debug.Module
	if under(&info.Main) {
		best = &info.Main
	}
	for _, m := range info.Deps {
		if under(m) && (best == nil || len(m.Path) > len(best.Path)) {
			best = m
		}
	}
	return best
}

func vcsOf(info *debug.BuildInfo) map[string]any {
	vcs := map[string]any{}
	for _, s := range info.Settings {
		switch s.Key {
		case "vcs":
			vcs["system"] = s.Value
		case "vcs.revision":
			vcs["revision"] = s.Value
		case "vcs.time":
			vcs["time"] = s.Value
		case "vcs.modified":
			vcs["modified"] = s.Value == "true"
		}
	}
	if len(vcs) == 0 {
		return nil
	}
	return vcs
}

// pkgPath is the import path of the package the provider was written in:
// its build function's, or its instance's type's.
func (s *providerSpec) pkgPath() string {
	if s.buildFn != 0 {
		if f := runtime.FuncForPC(s.buildFn); f != nil {
			return funcPackage(f.Name())
		}
	}
	t := s.typ
	for t != nil && t.Kind() == reflect.Pointer {
		t = t.Elem()
	}
	if t == nil {
		return ""
	}
	return t.PkgPath()
}

// funcPackage is the package of a function's full name, as
// runtime.FuncForPC spells it: path/to/pkg.Func, path/to/pkg.(*T).Method.
func funcPackage(name string) string {
	slash := strings.LastIndex(name, "/")
	dot := strings.Index(name[slash+1:], ".")
	if dot < 0 {
		return name
	}
	return name[:slash+1+dot]
}

// ID is the library's id.
func (l *Library) ID() string { return l.id }

// Version is the library's version.
func (l *Library) Version() string { return l.version }

// Providers are the ids of its providers, in order.
func (l *Library) Providers() []string { return append([]string(nil), l.ids...) }

// Kinds are the kinds its providers serve, each once, in order.
func (l *Library) Kinds() []string {
	var out []string
	seen := map[string]bool{}
	for _, s := range l.providers {
		for _, d := range s.kinds {
			if !seen[d.spec.Name] {
				seen[d.spec.Name] = true
				out = append(out, d.spec.Name)
			}
		}
	}
	return out
}

// Meta is what its descriptor says about it.
func (l *Library) Meta() map[string]any { return maps.Clone(l.meta) }

// LiveInstances is how many instances hosts built and have not destroyed.
func (l *Library) LiveInstances() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.instances)
}

// Entry is the library's guatiao_library_entry, the descriptor built
// first: what Registry.RegisterEntry, or a C host, calls.
func (l *Library) Entry() (unsafe.Pointer, error) {
	if err := l.build(); err != nil {
		return nil, err
	}
	return native.PoolEntry(l.index), nil
}

// Answer is what the library's guatiao_library_entry answers host: the
// descriptor, built on first use, or nil once the library is closed or
// when it cannot be built (the reason goes to Warn), which declines the
// host. A -buildmode=c-shared library exports one:
//
//	//export guatiao_library_entry
//	func guatiao_library_entry(host unsafe.Pointer) unsafe.Pointer { return library.Answer(host) }
//
// A nil *Library answers nil.
func (l *Library) Answer(host unsafe.Pointer) unsafe.Pointer {
	if l == nil || l.closed.Load() {
		return nil
	}
	if err := l.build(); err != nil {
		warn(fmt.Errorf("the library %s declines its host: %w", l.id, err))
		return nil
	}
	return l.info
}

// Close lets go of each provider's own instance (closing one that is an
// io.Closer) and answers nil to any host asking for the descriptor from
// now on. Call it once no host will call the library again.
func (l *Library) Close() error {
	if !l.closed.CompareAndSwap(false, true) {
		return nil
	}
	l.mu.Lock()
	defaults := l.defaults
	l.defaults = nil
	l.mu.Unlock()
	for _, ctx := range defaults {
		if x, ok := release(ctx); ok {
			closeHeld(x, "the provider instance of "+l.id)
		}
	}
	return nil
}

// ---- the descriptor -------------------------------------------------------------------

type keptText struct {
	ptr unsafe.Pointer
	n   uintptr
}

type pooled struct {
	lib     *Library
	spec    *providerSpec
	reasons sync.Map // string -> reason text in C memory
}

var (
	poolMu        sync.Mutex
	poolLibraries []*Library
	poolProviders []*pooled
)

func (l *Library) build() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.built {
		return l.buildErr
	}
	l.built = true
	l.buildErr = l.describe()
	return l.buildErr
}

func (l *Library) describe() error {
	if _, err := core(); err != nil {
		return err
	}
	poolMu.Lock()
	if len(poolLibraries) >= native.PoolLibraries() {
		poolMu.Unlock()
		return fmt.Errorf("guatiao: this process has built %d libraries, as many as it can", len(poolLibraries))
	}
	if len(poolProviders)+len(l.providers) > native.PoolProviders() {
		poolMu.Unlock()
		return fmt.Errorf("guatiao: this process has built %d providers; %d more is past the %d it can", len(poolProviders), len(l.providers), native.PoolProviders())
	}
	l.index = len(poolLibraries)
	poolLibraries = append(poolLibraries, l)
	first := len(poolProviders)
	for _, s := range l.providers {
		poolProviders = append(poolProviders, &pooled{lib: l, spec: s})
	}
	poolMu.Unlock()

	keep := func(x any) (*Value, error) {
		v, err := NewValue(x)
		if err != nil {
			return nil, err
		}
		l.values = append(l.values, v)
		return v, nil
	}
	descs := make([]native.Provider, len(l.providers))
	for i, s := range l.providers {
		d := native.Provider{ID: l.ids[i], DisplayName: s.displayName, Version: s.version, Pool: first + i,
			Builds: s.build != nil, AsksAvailable: s.available != nil}
		for _, k := range s.kinds {
			table, err := k.tableFor(s.typ)
			if err != nil {
				return err
			}
			d.Kinds = append(d.Kinds, k.spec.Name)
			d.Tables = append(d.Tables, native.KindTable{Kind: k.spec.Name, Table: table, Size: k.size})
		}
		if s.meta != nil {
			v, err := keep(s.meta)
			if err != nil {
				return fmt.Errorf("guatiao: the provider %s's meta: %w", l.ids[i], err)
			}
			if v.Tag() != TagMap {
				return fmt.Errorf("guatiao: the provider %s's meta is not a map", l.ids[i])
			}
			d.Meta = v.o.node
		}
		if s.build != nil {
			schema := s.schema
			if schema == nil {
				sv, err := SchemaFor(s.config)
				if err != nil {
					return fmt.Errorf("guatiao: the provider %s's configuration: %w", l.ids[i], err)
				}
				l.values = append(l.values, sv)
				d.Config = sv.o.node
			} else {
				v, err := keep(schema)
				if err != nil {
					return fmt.Errorf("guatiao: the provider %s's schema: %w", l.ids[i], err)
				}
				d.Config = v.o.node
			}
		}
		if s.instance != nil {
			d.Ctx = hold(s.instance, false)
			l.defaults = append(l.defaults, d.Ctx)
		}
		descs[i] = d
	}
	var meta unsafe.Pointer
	if l.meta != nil {
		v, err := keep(l.meta)
		if err != nil {
			return fmt.Errorf("guatiao: the library %s's meta: %w", l.id, err)
		}
		meta = v.o.node
	}
	l.info = native.NewLibraryInfo(l.id, l.version, descs, meta, native.PoolUnload(l.index))
	return nil
}

func poolLibrary(i int) *Library {
	poolMu.Lock()
	defer poolMu.Unlock()
	if i < 0 || i >= len(poolLibraries) {
		return nil
	}
	return poolLibraries[i]
}

func poolProvider(i int) *pooled {
	poolMu.Lock()
	defer poolMu.Unlock()
	if i < 0 || i >= len(poolProviders) {
		return nil
	}
	return poolProviders[i]
}

func init() {
	native.OnEntry = func(i int, host unsafe.Pointer) unsafe.Pointer {
		return poolLibrary(i).Answer(host)
	}
	native.OnUnload = func(i int) uint32 {
		lib := poolLibrary(i)
		if lib == nil {
			return native.OK
		}
		// The Go runtime in a c-shared library cannot be unloaded.
		if lib.cshared || lib.LiveInstances() > 0 {
			return native.ErrBusy
		}
		return native.OK
	}
	native.OnCreate = func(i int, ctx uintptr, config, out, errp unsafe.Pointer) (status uint32) {
		p := poolProvider(i)
		fail := func(err error) uint32 {
			st, message := statusOf(err)
			native.WriteProviderError(errp, uint32(st), message)
			return uint32(st)
		}
		defer func() {
			if r := recover(); r != nil {
				status = fail(&Error{Status: Internal, Message: fmt.Sprintf("panic: %v", r)})
			}
		}()
		if out == nil {
			return fail(&Error{Status: NullArgument, Message: "no slot for the instance"})
		}
		native.SetUintptr(out, 0)
		if p == nil || p.spec.build == nil {
			return fail(&Error{Status: Unsupported, Message: "this provider builds no instances"})
		}
		if config == nil {
			return fail(&Error{Status: NullArgument, Message: "no configuration"})
		}
		x, err := p.spec.build(config)
		if err != nil {
			return fail(err)
		}
		made := hold(x, false)
		p.lib.mu.Lock()
		p.lib.instances[made] = true
		p.lib.mu.Unlock()
		native.SetUintptr(out, made)
		return native.OK
	}
	native.OnDestroy = func(i int, ctx, instance uintptr) {
		p := poolProvider(i)
		if p != nil {
			p.lib.mu.Lock()
			delete(p.lib.instances, instance)
			p.lib.mu.Unlock()
		}
		if x, ok := release(instance); ok {
			closeHeld(x, "an instance its host destroyed")
		}
	}
	native.OnAvailable = func(i int, ctx uintptr, reason unsafe.Pointer) (ok bool) {
		p := poolProvider(i)
		if p == nil || p.spec.available == nil {
			return true
		}
		var why error
		func() {
			defer func() {
				if r := recover(); r != nil {
					why = fmt.Errorf("panic: %v", r)
				}
			}()
			why = p.spec.available()
		}()
		if why == nil {
			return true
		}
		// The reason outlives every reader: one copy per text, kept.
		text := why.Error()
		kept, found := p.reasons.Load(text)
		if !found {
			ptr, n := native.CopyBytesIn(text)
			kept, _ = p.reasons.LoadOrStore(text, keptText{ptr: ptr, n: n})
		}
		r := kept.(keptText)
		native.SetReason(reason, r.ptr, r.n)
		return false
	}
}
