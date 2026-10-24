// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"strings"
	"sync/atomic"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Registry is a host's table of loaded libraries and the providers they
// offer. Every answer that is data is plain Go data (see Ref.Any),
// already freed. A Registry is not safe for concurrent use: a host
// sharing one between goroutines guards it, as the C surface asks. What
// it hands out (a remote, an instance) may be used from any goroutine.
type Registry struct {
	handle unsafe.Pointer
	alloc  unsafe.Pointer
	kept   []*Library
	closed bool
}

// NewRegistry is a registry for a host named id at version. InAllocator
// names the allocator its answers are built in, and which it offers every
// library.
func NewRegistry(id, version string, opts ...Option) (*Registry, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	if err := lib.Need("guatiao_registry_new"); err != nil {
		return nil, lift(err)
	}
	var o options
	for _, opt := range opts {
		opt(&o)
	}
	alloc := o.alloc
	if alloc == nil {
		alloc = lib.Alloc()
	}
	handle := native.RegistryNew(id, version, alloc)
	if handle == nil {
		return nil, &Error{Status: BadValue, Message: "guatiao_registry_new refused: an id or version not UTF-8, or an incomplete allocator"}
	}
	return &Registry{handle: handle, alloc: alloc}, nil
}

// Close frees the registry; a second call does nothing. The libraries it
// loaded stay mapped, as the C surface keeps them.
func (r *Registry) Close() error {
	if r.closed {
		return nil
	}
	r.closed = true
	native.RegistryFree(r.handle)
	return nil
}

func (r *Registry) open() (unsafe.Pointer, error) {
	if r == nil || r.closed {
		return nil, ErrClosed
	}
	return r.handle, nil
}

// answer runs one (reg, ..., alloc, out) call and decodes the answer into
// dst, freeing it.
func (r *Registry) answer(name, text string, dst any) error {
	h, err := r.open()
	if err != nil {
		return err
	}
	out := native.Calloc(native.SizeValue)
	v := adopt(out, r.alloc)
	defer v.Close()
	if err := check(native.RegistryAnswer(name, h, text, r.alloc, out), ""); err != nil {
		return err
	}
	return v.Decode(dst)
}

func (r *Registry) keyCall(name, key string) error {
	h, err := r.open()
	if err != nil {
		return err
	}
	return check(native.RegistryKeyCall(name, h, key), "")
}

func mapsOf(list []any) []map[string]any {
	out := make([]map[string]any, 0, len(list))
	for _, x := range list {
		if m, ok := x.(map[string]any); ok {
			out = append(out, m)
		}
	}
	return out
}

// ---- loading ------------------------------------------------------------------

// LoadFile maps the library at path, answering what was loaded or
// skipped: {"loaded": {...}}, {"skipped": why, ...} or {"failed": why}.
// Mapping a library runs its static initialisers: name files you are
// willing to run.
func (r *Registry) LoadFile(path string) (map[string]any, error) {
	var out map[string]any
	return out, r.answer("load_file", path, &out)
}

// RegisterEntry registers a library the host links, from its entry point
// (a guatiao_library_entry), under name.
func (r *Registry) RegisterEntry(name string, entry unsafe.Pointer) (map[string]any, error) {
	h, err := r.open()
	if err != nil {
		return nil, err
	}
	out := native.Calloc(native.SizeValue)
	v := adopt(out, r.alloc)
	defer v.Close()
	if err := check(native.RegistryRegisterEntry(h, name, entry, r.alloc, out), ""); err != nil {
		return nil, err
	}
	var report map[string]any
	return report, v.Decode(&report)
}

// RegisterLibrary registers a library written here under its id, and
// keeps it alive as long as this registry. It is linked, not mapped:
// Unload answers ErrWrongKind, Retire takes it out.
func (r *Registry) RegisterLibrary(lib *Library) (map[string]any, error) {
	entry, err := lib.Entry()
	if err != nil {
		return nil, err
	}
	r.kept = append(r.kept, lib)
	return r.RegisterEntry(lib.ID(), entry)
}

// ScanOption says how a scan goes.
type ScanOption func(*scanOptions)

type scanOptions struct {
	descending bool
	rules      []string
}

// Descending visits names highest first.
func Descending() ScanOption { return func(o *scanOptions) { o.descending = true } }

// Rules admits only libraries whose declarations match: each rule is
// KEY=VALUE to require one, !KEY=VALUE to refuse one (`kind=greeter`).
func Rules(rules ...string) ScanOption {
	return func(o *scanOptions) { o.rules = append(o.rules, rules...) }
}

func (r *Registry) scan(where string, path bool, opts []ScanOption) (map[string]any, error) {
	var o scanOptions
	for _, opt := range opts {
		opt(&o)
	}
	h, err := r.open()
	if err != nil {
		return nil, err
	}
	out := native.Calloc(native.SizeValue)
	v := adopt(out, r.alloc)
	defer v.Close()
	if err := check(native.RegistryScan(h, where, o.descending, strings.Join(o.rules, "\n"), path, r.alloc, out), ""); err != nil {
		return nil, err
	}
	var report map[string]any
	return report, v.Decode(&report)
}

// ScanDir loads every library in dir the rules admit, reading each file's
// export table as data first: {"loaded": [...], "skipped": [...],
// "failed": [...]}.
func (r *Registry) ScanDir(dir string, opts ...ScanOption) (map[string]any, error) {
	return r.scan(dir, false, opts)
}

// ScanPath loads from each place of a search path, as the platform
// separates them (`;` on Windows, `:` elsewhere), with one report.
func (r *Registry) ScanPath(spec string, opts ...ScanOption) (map[string]any, error) {
	return r.scan(spec, true, opts)
}

// ---- what is loaded ---------------------------------------------------------------

// Libraries are every library loaded.
func (r *Registry) Libraries() ([]map[string]any, error) {
	var list []any
	err := r.answer("libraries", "", &list)
	return mapsOf(list), err
}

// Providers are those claiming kind (every one for ""), best first.
func (r *Registry) Providers(kind string) ([]map[string]any, error) {
	var list []any
	err := r.answer("providers", kind, &list)
	return mapsOf(list), err
}

// Available are the providers of kind that can run here, best first.
func (r *Registry) Available(kind string) ([]map[string]any, error) {
	var list []any
	err := r.answer("available", kind, &list)
	return mapsOf(list), err
}

// WhyNot says why nothing, or something, answers for kind.
func (r *Registry) WhyNot(kind string) (map[string]any, error) {
	var out map[string]any
	return out, r.answer("why_not", kind, &out)
}

// Provider is the provider filed under key; ErrNotFound for none.
func (r *Registry) Provider(key string) (map[string]any, error) {
	var out map[string]any
	return out, r.answer("provider", key, &out)
}

// Best is the best provider of kind.
func (r *Registry) Best(kind string) (map[string]any, error) {
	var out map[string]any
	return out, r.answer("best", kind, &out)
}

// ProviderAvailable reports whether the provider under key can run here,
// and when not, its reason.
func (r *Registry) ProviderAvailable(key string) (bool, string, error) {
	h, err := r.open()
	if err != nil {
		return false, "", err
	}
	ok, why := native.RegistryProviderAvailable(h, key)
	return ok, why, nil
}

// ---- keys and priority ---------------------------------------------------------------

// KeyedBy files providers under keys made from template (%id, %name,
// %library, %version).
func (r *Registry) KeyedBy(template string) error { return r.keyCall("keyed_by", template) }

// LibrariesKeyedBy files libraries under keys made from template (%id,
// %version).
func (r *Registry) LibrariesKeyedBy(template string) error {
	return r.keyCall("libraries_keyed_by", template)
}

// SetPriority ranks the provider id: higher first.
func (r *Registry) SetPriority(id string, priority int32) error {
	h, err := r.open()
	if err != nil {
		return err
	}
	return check(native.RegistrySetPriority(h, id, priority), "")
}

// Priority is the provider id's rank.
func (r *Registry) Priority(id string) (int32, error) {
	h, err := r.open()
	if err != nil {
		return 0, err
	}
	return native.RegistryPriority(h, id), nil
}

// ---- a provider's table and configuration ---------------------------------------------

// ProviderTable is the table the provider under key speaks kind through
// (its single table when kind is ""), with its own ctx. ErrNotFound when
// there is none. Valid while the library is.
func (r *Registry) ProviderTable(key, kind string) (Table, error) {
	h, err := r.open()
	if err != nil {
		return Table{}, err
	}
	table, size, ctx := native.RegistryTable(h, key, kind, kind == "")
	if table == nil {
		return Table{}, &Error{Status: NotFound, Message: key + " serves no " + kind + " table"}
	}
	return Table{Pointer: table, Size: size, Ctx: ctx}, nil
}

// ProviderConfig is a read-only view of the provider's configuration
// schema, borrowed from this registry, and false when it declares none.
func (r *Registry) ProviderConfig(key string) (Ref, bool, error) {
	h, err := r.open()
	if err != nil {
		return Ref{}, false, err
	}
	find := func() unsafe.Pointer {
		if r.closed {
			return nil
		}
		return native.RegistryConfig(h, key)
	}
	if find() == nil {
		return Ref{}, false, nil
	}
	return Ref{o: &borrowed{find: find}}, true, nil
}

// ProviderMeta is what the provider under key declared beyond its schema
// (its legal notices, or its form under the intake package's
// ProviderFormKey) as a Value the caller owns and closes, and false when it
// declares nothing. A copy, unlike ProviderConfig: it outlives the library.
// ErrNotFound when nothing answers to key.
func (r *Registry) ProviderMeta(key string) (*Value, bool, error) {
	h, err := r.open()
	if err != nil {
		return nil, false, err
	}
	out := native.Calloc(native.SizeValue)
	v := adopt(out, r.alloc)
	if err := check(native.RegistryAnswer("provider_meta", h, key, r.alloc, out), ""); err != nil {
		v.Close()
		return nil, false, err
	}
	if v.Tag() == TagAbsent {
		v.Close()
		return nil, false, nil
	}
	return v, true, nil
}

// Instance is one a registry built from a configuration: what each call
// through its tables takes. Close destroys it through the provider that
// built it, even after a retire or a refresh loaded a new build.
type Instance struct {
	reg    *Registry
	key    string
	ctx    uintptr
	closed atomic.Bool
}

// Create builds an instance of the provider under key from config (a
// Value, a view or Go data), which the registry checks against the
// provider's schema first.
func (r *Registry) Create(key string, config any) (*Instance, error) {
	var inst *Instance
	err := withNodes([]any{config}, func(nodes []unsafe.Pointer) error {
		h, err := r.open()
		if err != nil {
			return err
		}
		errp := native.Calloc(native.SizeProviderErr)
		defer native.Free(errp)
		ctx, st := native.RegistryCreate(h, key, nodes[0], errp)
		if err := check(st, native.TakeProviderError(errp)); err != nil {
			return err
		}
		inst = &Instance{reg: r, key: key, ctx: ctx}
		return nil
	})
	return inst, err
}

// Ctx is the instance, as every slot of its tables takes it.
func (i *Instance) Ctx() uintptr { return i.ctx }

// Close destroys the instance; a second call does nothing.
func (i *Instance) Close() error {
	if i == nil || !i.closed.CompareAndSwap(false, true) {
		return nil
	}
	if h, err := i.reg.open(); err == nil {
		native.RegistryDestroy(h, i.key, i.ctx)
	}
	return nil
}

// ---- retiring and unloading ----------------------------------------------------------

// Retire takes the library under key (the LIBRARY key) out of this
// registry and leaves it mapped until nothing uses it.
func (r *Registry) Retire(key string) error { return r.keyCall("retire", key) }

// Collect closes every retired library nothing uses, and answers their
// keys.
func (r *Registry) Collect() ([]string, error) {
	var keys []string
	return keys, r.answer("collect", "", &keys)
}

// Unload unmaps the library under key when it agrees: ErrBusy while
// something it handed out lives, ErrUnsupported when it has no unload
// slot, ErrWrongKind for a library linked rather than mapped, ErrNotFound
// for none. No table, ctx or schema taken from it is used again.
func (r *Registry) Unload(key string) error { return r.keyCall("unload", key) }

// UnloadUnchecked unmaps the library under key past the two refusals
// only the caller can overrule: a lease still held, and no unload slot.
func (r *Registry) UnloadUnchecked(key string) error { return r.keyCall("unload_unchecked", key) }

// ---- sources ---------------------------------------------------------------------------

// AddSourceFile maps a file exporting guatiao_source (a bridge to another
// language, say) and adds the source it answers for config, a map (nil is
// an empty one), refusing with its words. Nothing it serves loads until
// Refresh.
func (r *Registry) AddSourceFile(path string, config any) error {
	if config == nil {
		config = map[string]any{}
	}
	return withNodes([]any{config}, func(nodes []unsafe.Pointer) error {
		h, err := r.open()
		if err != nil {
			return err
		}
		errp := native.Calloc(native.SizeProviderErr)
		defer native.Free(errp)
		st := native.RegistryAddSourceFile(h, path, nodes[0], errp)
		return check(st, native.TakeProviderError(errp))
	})
}

// WatchDir watches a directory: each Refresh loads, retires and reloads
// its libraries as they change, by rules.
func (r *Registry) WatchDir(dir string, rules ...string) error {
	h, err := r.open()
	if err != nil {
		return err
	}
	return check(native.RegistryWatchDir(h, dir, strings.Join(rules, "\n")), "")
}

// Refresh asks every source and watched directory what it serves now and
// makes this registry agree: {"loaded", "retired", "skipped", "refused"}.
func (r *Registry) Refresh() (map[string]any, error) {
	var out map[string]any
	return out, r.answer("refresh", "", &out)
}

// Host is the guatiao_host_info this registry hands its libraries.
func (r *Registry) Host() unsafe.Pointer {
	h, err := r.open()
	if err != nil {
		return nil
	}
	return native.RegistryHost(h)
}
