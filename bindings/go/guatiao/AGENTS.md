# guatiao (Go) — API header

Go bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the module, so it names no
repository path.

Module `github.com/Boriken-Dev/guatiao/bindings/go/guatiao`, Go 1.26 or later,
64-bit platforms, cgo (a C compiler at build time: GCC or clang; on
Windows a MinGW-style one, such as llvm-mingw or MinGW-w64 GCC). No
dependencies outside the standard library, and no native library of its
own: guatiao is loaded at run time.

## Packages

| import | what |
| --- | --- |
| `.../bindings/go/guatiao` (package `guatiao`) | `Value`, `Ref`, `MapRef`, `ListRef`, `Number`, `Tag`, `Status`, the errors, `NewValue`, `SchemaOf`, `Redact`, `UseLibrary`; `FromWire`, `MarshalWire`, `UnmarshalWire`, `WireAnnounce`, `WireSend`, `WireClose`, `Receiver`; `Registry`, `Instance`; `Kind`, `Remote`; `Single`, `Configured`, `Library`, `LibraryOf`; `Load`, `Adopt`, `WithNodes`, `NodeOf`, `Borrow`, `StatusError`, `Surface` for a package over another guatiao library |
| `.../bindings/go/guatiao/civil` | `Date`, `Time`, `DateTime`, `Span`, `Length`, instants as `time.Time` |
| `.../bindings/go/guatiao/cmd/guatiao-kind` | the command writing a kind package's glue, run by `go generate` |

JSON, TOML and YAML, and taking a value in from a person, are modules of
their own that require this one, each over its own library and with its
own `AGENTS.md`: `github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde`
(package `serde`, over `guatiao_serde`) and
`github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake` (package
`intake`, over `guatiao_intake`). The three are released together, under
the same version.

## Finding the native library

`guatiao` and `civil` are over the `guatiao` library. Nothing native is
touched until the first call that needs it; it is then found in this
order:

1. `guatiao.UseLibrary(path)`, called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded is an error; naming the same one is
   not.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file, taken as it is.
3. The platform's own library search, by file name.

Another guatiao library (`guatiao_serde`, `guatiao_intake`, a `Surface`
of any package) is found the same way, with its own variable
(`GUATIAO_SERDE_LIBRARY`, ...) consulted first, and a file
`GUATIAO_LIBRARY` names taken when it exports that library (an
application's own library carrying several surfaces), else the library
beside it.

Not found is a `*guatiao.LibraryNotFound` (`Basename`, `Looked`). An
export the loaded build lacks is a `*guatiao.MissingSymbol` (`Path`,
`Symbol`, `Feature`) when it is used; a file missing an export every
build has is refused when it is opened. A library stays loaded for the
process.

## Values

`*Value` owns a root node; `Close()` frees it (and answers nil; a second
call does nothing). A Value dropped unclosed is freed when the garbage
collector finds it and every `Ref` into it unreachable. `Ref` addresses a
node by a path of keys and indices, walked again on every access, never a
cached address: a write beside a node can move the array it sits in.
`MapRef` and `ListRef` embed a `Ref` over a map and a list; `Value`
embeds a `Ref` onto its root. The zero `Ref` names nothing.

```go
v, err := guatiao.NewValue(map[string]any{"host": "10.0.0.1", "port": 5900, "tags": []string{"a", "b"}})
defer v.Close()
m, err := v.Map()
m.Set("port", 5901)
tags, _ := v.Key("tags").List()
tags.Push("c")
v.Key("port").IntOr(22)   // 5901
m.Keys()                  // [host port tags]
var back struct{ Host string `guatiao:"host"` }
v.Decode(&back)
```

**Building**: `NewValue(x, opts...) (*Value, error)` converts Go data (see
below); `Absent() *Value`, `Null()`, `NewMap()`, `NewList()`. Option
`InAllocator(alloc unsafe.Pointer)`: the address of a `guatiao_alloc` the
caller keeps alive while every tree built through it lives. A failed
conversion frees what it built.

**Reading** (on `Ref`, so on `Value` too): `Key(k)` and `Index(i)` extend
the path (negative counts from the end) without checking; `Tag()`,
`IsAbsent()`, `IsNull()`; `BoolOr`, `IntOr(int64)` (a signed 64-bit
integer; a fraction, an exponent, another kind or out of range falls
back, never truncates), `FloatOr`, `StringOr`; `Text() (string, bool)`,
`NumberText() (Number, bool)`, `Bytes() ([]byte, bool)` (a copy);
`Any() (any, error)` (plain data); `Decode(dst any) error`; `Map()
(MapRef, error)` and `List() (ListRef, error)` (`ErrWrongKind` for another
kind, `ErrNotFound` for nothing there); `Clone() (*Value, error)`;
`Path()`, the path in the intake grammar.

**`MapRef`**: `Len`, `Has(k)`, `Get(k) (Ref, bool)`, `Set(k, x) error`
(replaces in place), `Delete(k) (bool, error)`, `Keys()`, `All()
iter.Seq2[string, Ref]`, `Clear()`, `CopyFrom(src Ref)`. Keys compare as
UTF-8 bytes and keep insertion order.

**`ListRef`**: `Len`, `At(i) (Ref, bool)`, `Push(xs ...any) error`,
`RemoveAt(i)`, `Clear()`, `All() iter.Seq2[int, Ref]`, and `Set(i, x)` and
`Insert(i, x)`, which are O(n): the C surface only appends and removes, so
they copy every element, clear and push back.

**`Number`** is a number's exact text (`type Number string`): `Int64()`,
`Uint64()` (strict), `Float64()` (nearest; an error when not finite).

A closed Value, or a view whose source is closed or whose call has
returned, reads as absent and answers `ErrClosed` from every call that
returns an error. A borrowed view answers `ErrReadOnly` from every write.

## Go values and trees

`NewValue`, `Set`, `Push` and every kind call convert Go data; `Decode`
converts back, as `encoding/json` does:

| Go | value |
| --- | --- |
| `nil`, nil pointer or interface | null (a nil pointer *field* is left out) |
| `bool` | boolean |
| `int*`, `uint*`, `float*`, `Number`, `*big.Int` | number, its exact text (a float's shortest; `-0` kept; NaN and infinities refused) |
| `string` | string (UTF-8 or refused) |
| `[]byte` | bytes |
| slice, array | list (decoding into an array refuses another length) |
| `map[K]V`, K string, integer, bool or TextMarshaler | map, keys written sorted (integers by value) |
| struct | map: exported fields by `guatiao` tag, else `json` tag, else name; `-` skips, `omitempty` drops a zero value; untagged embedded structs lend their fields |
| `time.Time` | an instant's text, UTC (`2026-10-21T13:45:00.5Z`) |
| `time.Duration` | an exact length `[seconds, nanoseconds]`; decoding reads its three spellings; negative or past ~292 years refused |
| `Marshaler` (`MarshalGuatiao() (any, error)`), `Unmarshaler` (`UnmarshalGuatiao(Ref) error`) | what it says |
| `encoding.TextMarshaler` / `TextUnmarshaler` | string |
| `*Value`, `Ref`, `MapRef`, `ListRef` | a deep copy; decoding into `*Value` takes one |

Decoding into `any` gives `nil`, `bool`, `Number`, `string`, `[]byte`,
`[]any`, `map[string]any`. Decoding refuses a number that does not fit its
target and never rounds; an absent node leaves the target as it was; keys
a struct has no field for are ignored. Both directions stop past 128
nested containers. A failure is a `*guatiao.Error` with `BadValue` naming
the path (`at agent[1].port: want an integer, got string`).

**`SchemaOf[T]() (*Value, error)`** (and `SchemaFor(reflect.Type)`): the
JSON Schema of what `T` encodes to, properties in field order. A struct
refuses keys it does not declare; a field is required unless it is a
pointer (which may also be null), `omitempty`, or has a default; a
`schema` tag adds keywords, `schema:"minLength=1,default=hey"`, each value
read as JSON when it is JSON and as text otherwise. Integer types state
their bounds; `time.Time` is `date-time`, `time.Duration` and the civil
types their forms; a `GuatiaoSchema() map[string]any` method states a
type's own. A recursive type is refused.

`ValueFromRaw(root unsafe.Pointer) *Value` takes over a root node a C
function answered (a `guatiao_value` in C `malloc` memory); `Ref.Raw()`
is a node's address now, for a C function taking one.

## Registry

```go
reg, err := guatiao.NewRegistry("my-host", "1.0")
defer reg.Close()
reg.ScanDir("/path/to/plugins", guatiao.Rules("kind=greeter"))
providers, err := reg.Providers("greeter")   // []map[string]any, best first
best, err := reg.Best("greeter")             // key, id, version, library, display_name, from, kinds, has_config, vtable_size
why, err := reg.WhyNot("codec")              // {available: false, why: "nothing-claims-it", ...}
greeter, err := greeterkind.GreeterKind.Bind(reg, "acme_hello")   // a remote (see Kinds)
inst, err := reg.Create("acme_shouter", map[string]any{"prefix": "hey"})
defer inst.Close()
shouter, err := greeterkind.GreeterKind.BindInstance(inst)
```

`NewRegistry(id, version, opts...)` (`InAllocator` names the allocator its
answers are built in and that it offers libraries). Every answer that is
data is plain Go data, already freed (numbers as `Number`). Also
`LoadFile(path)`, `RegisterEntry(name, entry unsafe.Pointer)` (a
`guatiao_library_entry`), `RegisterLibrary(lib)` (see Providers),
`ScanDir(dir, opts...)` and `ScanPath(spec, opts...)` with `Rules(...)`
(`[!]KEY=VALUE`, matched against what a library declares) and
`Descending()`, `Libraries()`, `Provider(key)`, `Available(kind)`,
`ProviderAvailable(key) (bool, string, error)`, `KeyedBy(template)` /
`LibrariesKeyedBy`, `SetPriority` / `Priority`, `ProviderTable(key, kind)
(Table, error)` (`ErrNotFound` for none; `kind` "" for a single table),
`ProviderConfig(key) (Ref, bool, error)` (a borrowed, read-only view of the
schema), `ProviderMeta(key) (*Value, bool, error)` (what the provider
declared beyond its schema, its notices or its form under
`intake.ProviderFormKey`: a copy the caller closes, outliving the library;
false when it declares nothing, `ErrNotFound` for an unknown key),
`KindDescriptions(key) (*Value, bool, error)` (the document of the kinds
the provider serves and the object kinds they reach, what `guatiao-kind
-description` reads, a copy the caller closes; false when it carries
none, `ErrNotFound` for an unknown key, `ErrBadValue` for one that is not
a description), `Host() unsafe.Pointer` (the `guatiao_host_info *`), `Close()`.

`Create(key, config)` checks the configuration against the provider's
schema first (`ErrBadValue` with the field), then builds an `*Instance`;
its `Close()` reaches the provider that built it, even after a retire or a
refresh that loaded a new build under the key.

`Retire(libraryKey)` takes a library out and leaves it mapped until nothing
uses it; `Collect()` closes every retired library whose own `unload`
agrees, and answers their keys, as does the next load, retire or unload.
`Unload(libraryKey)` unmaps when the library agrees: `ErrBusy` while
something it handed out lives, `ErrUnsupported` with no `unload` function,
`ErrWrongKind` for a library linked rather than mapped, `ErrNotFound` for
none; `UnloadUnchecked` insists. **Both take the LIBRARY key** (`%id` by
default). No table, ctx or borrowed schema taken from it is used again.

**Sources**: `AddSourceFile(path, config)` maps a file exporting
`guatiao_source` (a bridge to another language) and adds the source it
answers, refusing with its words; `WatchDir(dir, rules...)` watches a
directory; `Refresh()` makes the registry agree with every source,
answering `{loaded, retired, skipped, refused}`. Nothing refreshes on its
own.

A Registry is not safe for concurrent use: a host sharing one between
goroutines guards it, as the C surface asks. An `Instance`'s `Close` is a
call on its registry.

## Kinds: calling a table, serving one

A kind is a Go interface naming its required slots, marked
`//guatiao:kind <name> [<C table>]`; each optional slot is an interface of
its own, as `io.WriterTo` stands beside `io.Writer`; every method returns
an `error` last, and an object kind's interface has `Close() error`, its
`destroy` slot. The command `guatiao-kind` reads the kinds' description
(`-description <crate>.kinds.json`, the document a kind crate commits) or
their C header (`-header`, which needs the C table name; it defaults to
`<name>_vtable`): slot order, floor hash, which argument is handed over,
lent or borrowed, which slot can refuse. It reads the interfaces too, and writes the glue:
`<Interface>Kind` (a `*guatiao.Kind[*<Interface>Remote]`), a remote with
a method per slot implementing every interface, and the cgo calling and
serving each slot.

```go
package greeterkind

//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/guatiao/cmd/guatiao-kind -description ../include/greeter_kind.kinds.json

import guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"

type Greeting struct {
	Greeting string `guatiao:"greeting"`
}

//guatiao:kind greeter greeter_vtable
type Greeter interface {
	Greet(name string) (Greeting, error)
}

type Shouter interface{ Shout(name string) (string, error) }
type Checker interface{ Check(config guatiao.MapRef) (bool, error) }

//guatiao:kind listener listener_vtable
type Listener interface {
	Heard(what string) error
	Close() error
}
```

The binding must be imported as `guatiao`. A slot's method is its C name
in CamelCase (`greet_into` is `GreetInto`); `guatiao-kind -check` fails
when the glue is not what it would write.

**A kind born in Go** (no description, no header): `guatiao-kind -document
<file>` reads the interfaces into the description, writes it there (from
the package's directory; `-check` compares it instead), and the glue from
it. Directives in doc comments say what a Go type cannot: on the kind's
interface `//guatiao:appended <Iface>...` (the appended slots, an
interface of one method each, in table order) and `//guatiao:floorhash
<n>` (else computed as `#[guatiao::kind]` would for the same required
slots); on a method `//guatiao:infallible` (else fallible) and
`//guatiao:lent`, `//guatiao:taken`, `//guatiao:nullable`, `//guatiao:mut`
followed by argument names. A `Ref`/`MapRef`/`ListRef` is borrowed (a
list must be lent), `*guatiao.Value` a value handed over, a struct or map
type a map, a slice a list, `any` a value (borrowed unless `taken`),
`[]byte` bytes (`mut`: written in place), an object kind's interface an
object handed over unless `lent`; answered, a struct or map is a map. A
kind's name is its marker's, a slot's its method's in snake_case
(capitals in a row are refused), docs its doc comments, an appended
slot's falling back to its interface's; the document lists the kinds in
the order the package declares them (files by name). Types from another
package are refused, object kinds of another package included, as are
two kinds of one name, two slots of one name, an appended slot's
method declared in a second interface, a slot named `destroy` or a method
named as one of the remote's own (`Begin`, `Has`, `Kind`, `Table`,
`Closed`), and `mut` bytes on a provider kind (object kinds only). A
refusal writes nothing.

| C | a method takes / returns |
| --- | --- |
| `guatiao_str` | `string` (`ErrBadValue` served when not UTF-8) |
| `guatiao_bytes` | `[]byte` (a copy) |
| `guatiao_bytes_mut` | `[]byte` over the caller's memory, written in place, valid for the call: never keep it |
| `bool`, `int8_t`..`uint64_t`, `size_t`, `intptr_t`, `uintptr_t`, `float`, `double` | `bool`, `int8`..`uint64`, `uint`, `int`, `uintptr`, `float32`, `float64` |
| `const guatiao_value *`, `const guatiao_map *`, `const guatiao_list *` | `guatiao.Ref`, `MapRef`, `ListRef`, read-only and valid for the call (a null value is the zero `Ref`), or any Go type it decodes into |
| the same without `const`, the slot's comment calling it lent | `Ref`, `MapRef`, `ListRef`, writable, valid for the call; what is added is built in the allocator the tree records, and what a callee built elsewhere, or wrote over it, is moved, once the call returns, into the allocator the tree recorded when it was lent; a refused move is the call's error (the slot's own refusal first), what it answered freed |
| the same without `const`, handed over | `*guatiao.Value` the method owns (and closes), moved into this binding's allocator on arrival (copied only where the caller built it elsewhere), so it may be kept past the caller's library; or any Go type it decodes into (the tree is freed where it stands) |
| `guatiao_value/map/list *out` | the answer: a `*guatiao.Value` or any Go type, built in the allocator the carrier names; a `*Value` answered is handed over, moved into this binding's allocator on arrival when the slot built it in its own |
| `guatiao_string *out` | `string` |
| `T *out` | the Go scalar |
| `guatiao_object` (handed over) | the object kind's interface; the method owns it and closes it |
| `const guatiao_object *` (lent) | the object kind's interface, valid for the call; its `Close` does nothing |
| `guatiao_object *out` | the object kind's interface: a Go value serving it, or a remote this side owns, which the caller then owns |

**A remote** (`*GreeterRemote`) has a method per slot, `Has(slot)` (the
header's slot name), `Kind()`, `Table()`, `Close()` (destroys an object
this side owns, once; nothing for a provider's table or a lent object) and
`Closed()`. A slot past the table's size or null answers `ErrUnsupported`
(a Rust caller would run the kind's default body, which Go cannot); a
refusal is the status's `*Error` with the provider's words; a slot that
cannot fail and fails answers its status. Valid while the library is.
`Kind.BindTable(table, size, ctx)` makes one over any table, a
`*FloorMismatch` when the table is smaller than the kind's required slots
or has another floor hash. An object this side owns and drops unclosed is
destroyed when the collector finds it.

**A Go value as an object of a kind**: any value implementing the object
kind's interface. Its table is built once per kind and Go type (a slot for
each optional interface it implements, null for the rest) and kept for the
process. What C holds of it is a `cgo.Handle`, shaped like an address, so a
call naming one released answers `ErrGone`. It is held until its holder
destroys it (handed over or answered), when its `Close` runs, or until the
call returns (lent). `guatiao.LiveObjects()` counts what is held.

**A refusal**: a method returns an error; a `*guatiao.Error` (or one an
`errors.As` finds) crosses as its status and words, anything else as
`Internal` with its text, a panic as `Internal` with `panic: ...`. A slot
that cannot fail reports the failure to `guatiao.Warn` (`log.Printf`
unless set) and its status crosses alone.

## Providers written in Go

```go
type Hello struct{ asked atomic.Int64 }

var _ greeterkind.Greeter = (*Hello)(nil) // the compiler checks it

func (h *Hello) Greet(name string) (greeterkind.Greeting, error) {
	if name == "" {
		return greeterkind.Greeting{}, guatiao.Errorf(guatiao.BadValue, "nobody to greet")
	}
	return greeterkind.Greeting{Greeting: "hello, " + name}, nil
}

type ShouterConfig struct {
	Prefix string `guatiao:"prefix" schema:"minLength=1"`
}

func NewShouter(c ShouterConfig) (*Shouter, error) { return &Shouter{prefix: c.Prefix}, nil }

lib, err := guatiao.LibraryOf(
	guatiao.Single("hello", &Hello{}, greeterkind.GreeterKind, greeterkind.CounterKind,
		guatiao.DisplayName("Hello, in Go")),
	guatiao.Configured("shouter", NewShouter, greeterkind.GreeterKind),
)
reg.RegisterLibrary(lib)
```

**A provider** is any Go value implementing the kinds it names:
`Single(name, instance, opts...)` is its one instance; `Configured(name,
build func(C) (P, error), opts...)` builds an instance per configuration,
the host's configuration checked against `SchemaOf[C]()` and decoded into
a `C`. Options: a kind (`*Kind` is one); `ID` (default `{last element of
the library id}_{name}`), `DisplayName`, `ProviderVersion` (empty: the
library's), `ProviderMeta`, `AvailableIf(func() error)` (nil when it can
run here, else why not; asked every time a host asks), `Default(instance)`
(offered beside a Configured provider's instances; a `P`), `ConfigSchema`
(a raw schema in place of `C`'s). A value that does not implement a kind it
names, an object kind among its kinds, or two providers sharing an id is
refused when the library is made. An instance (or a Single's instance)
that implements `io.Closer` is closed when its host destroys it (when
`Library.Close` runs).

**A library**: `LibraryOf(parts...)` is the providers' Go package: id its
import path (the build's main package path for package `main`), version
that of the module holding it, as the build records it, the leading `v`
dropped. **A build that names no version** (`(devel)`: a checkout, a test
binary) is refused unless `LibraryVersion` says one. Meta: `module`, and
`vcs` (`system`, `revision`, `time`, `modified`) as the build recorded
them for the main module; `Summary`, `License`, `Authors`, `URL(label,
url)`, `Notice(component, text)` and `LibraryMeta` add the rest;
`LibraryID` overrides the id. `NewLibrary(id, version, parts...)` is the
same with both given. A `*Library` has `ID()`, `Version()`, `Providers()`,
`Kinds()`, `Meta()`, `LiveInstances()`, `Entry() (unsafe.Pointer, error)`
(its `guatiao_library_entry`), `Answer(host) unsafe.Pointer` and
`Close()`. Its descriptor is built on first use and kept for the process,
with its `unload` answering `ErrBusy` while an instance lives; a process
builds at most 64 libraries and 256 providers. `Registry.RegisterLibrary`
files it under its id and keeps it: linked, not mapped, so `Unload`
answers `ErrWrongKind` and `Retire` takes it out.

**A library file**: built with `-buildmode=c-shared`, a package `main`
exports the entry, and a C file `guatiao_declares` for a scan by kind:

```go
var library, _ = gogreeter.Library()

//export guatiao_library_entry
func guatiao_library_entry(host unsafe.Pointer) unsafe.Pointer { return library.Answer(host) }

func main() {}
```

```c
__declspec(dllexport) /* or __attribute__((visibility("default"))) */
const char guatiao_declares[] = "kind=greeter\0";
```

`Answer` builds the descriptor on first use and answers nil (declining
the host, with the reason sent to `Warn`) when it cannot, so a library
whose own guatiao is missing never aborts the process that mapped it. The
library finds guatiao as any program using this module does
(`GUATIAO_LIBRARY` or the platform's search). **A c-shared Go library is
never unmapped**: its `unload` always answers `ErrBusy`, since the Go
runtime inside it cannot be unloaded. A Go host may load such a file like
any other; the file then runs a Go runtime of its own beside the host's,
so a Go host usually links the provider's package instead.

## civil: dates, times, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>, and every text is read
and written by the library's own functions.

```go
d, err := civil.NewDate(2026, time.October, 21)      // checked
d2, err := d.Add(civil.Span{Months: 1})
t, err := civil.ParseTime("13:45:00.5")
dt, err := civil.ParseDateTime("2026-10-21T13:45:00.5")
at, err := civil.ParseTimestamp("2026-10-21T19:15:00+05:30") // time.Time, UTC
text, err := civil.FormatTimestamp(at)               // "...Z"
s, err := civil.ParseSpan("1h 30m")                  // ISO 8601 or friendly
text, err = civil.Normalize(civil.SpanForm, "1h 30m") // "PT1H30M"
l, err := civil.ReadLength(value)                    // [s, ns], seconds, {secs, nanos}
```

- **`Date`** (`Year`, `Month time.Month`, `Day`), **`Time`** (`Hour`,
  `Minute`, `Second`, `Nanosecond`), **`DateTime`** (`Date`, `Time`): a
  civil date and time to the nanosecond, no zone. `New*` and every
  `MarshalText` check through the library: `ErrBadValue` for a day the
  month lacks, a time off the clock, or a field past its C width.
  `Compare`, `String`, `MarshalText`/`UnmarshalText`, `Add(Span)` on
  `Date` and `DateTime`.
- **An instant is a `time.Time`**, nanoseconds kept; read in UTC, the
  offset it was written with not kept. Outside the years 0 to 9999 is
  `ErrBadValue`.
- **`Length`** (`Seconds uint64`, `Nanoseconds uint32`): an exact length
  to 18446744073709551615 seconds; `LengthOf(time.Duration)`,
  `Length.Duration()` (`ErrBadValue` past ~292 years); written `[s, ns]`.
- **`Span`**: eight signed fields with one sign for the whole; `==`
  compares unit by unit (`P1D` is not `PT24H`); `ParseSpan`, `Negated`,
  `IsZero`, `IsNegative`, `String`; fields of both signs or a unit out of
  range are `ErrBadValue` when written.
- **Text the grammar refuses is a `*civil.Unparsed`** (`Form`, `At` the
  byte offset in the UTF-8 text, `Expected`), an `ErrBadValue`; it never
  quotes the text.

## Values on the wire, and channels

```go
func (r Ref) ToWire() ([]byte, error)                     // any Ref, a *Value included
func FromWire(data []byte, opts ...Option) (*Value, error) // ErrBadValue: not exactly one value

func MarshalWire(x any) ([]byte, error)     // Go data to wire bytes, no tree built
func UnmarshalWire(data []byte, dst any) error // wire bytes into what dst points at, no tree built

func WireAnnounce(channel uint64, schema any) ([]byte, error) // a Value, a view or plain data
func WireSend(channel uint64, value any) ([]byte, error)
func WireClose(channel uint64) ([]byte, error)

func NewReceiver() (*Receiver, error)
func (r *Receiver) Accept(frame []byte) (Event, error)    // ErrClosed after Close
func (r *Receiver) Close() error
type Event struct { Kind Frame; Channel uint64; Value *Value } // Value: close it; nil for a close
type Frame uint32      // FrameSchema 0, FrameValue 1, FrameClose 2 (GUATIAO_FRAME_*)
type Refusal uint32    // RefusalMalformed, RefusalNoSchema, RefusalNotASchema, RefusalInvalid
type Refused struct {
	Refusal  Refusal
	Status   Status   // BadValue, NotFound, WrongKind, BadValue respectively
	Channel  uint64   // 0 when the frame ended before one
	Kind     Frame    // the frame's kind byte, an unknown one included
	At       int      // RefusalMalformed: the byte of the frame
	Key      string   // RefusalInvalid: the field
	Expected string   // RefusalInvalid: what it accepts
	Known    []string // RefusalInvalid on an unknown field: the fields there are
	Message  string
}
```

The wire encoding is a tag byte per node, LEB128 lengths, a number as its
own text, bytes as bytes, a map in its order: the same bytes in every
language. A frame is `kind:u8 channel:LEB128 payload`; the transport says
where one ends. A `Receiver` keeps each channel's schema and checks every
value against it. A refusal is a `*Refused`: `errors.Is` matches the
sentinel of its status, `errors.As` finds it as an `*Error` too; it
changes nothing the receiver keeps and quotes nothing the frame carried.
`Accept` may be called from several goroutines (calls are serialised); a
receiver dropped unclosed is freed by a cleanup.

`MarshalWire(x)` writes the bytes `NewValue(x)` then `ToWire` would, in
Go, with the same rules and errors; `UnmarshalWire(data, &dst)` reads
them as `FromWire` then `Decode` would. Neither builds a tree or calls the
library, except for a `time.Time`, a `time.Duration`, a `*Value` or an
`Unmarshaler` (which read through it). `UnmarshalWire` refuses bytes cut
short, bytes after the value, an unknown tag, a length not in its
shortest form or past what remains, text that is not UTF-8, a number
outside the grammar and nesting past 128, all `ErrBadValue`; it does not
look for a repeated key (the last is kept). A package whose library
answers a whole document as wire bytes crosses once this way, not once
per node: `serde.Unmarshal` and `serde.Emit` of Go data do.

## Secrets

```go
shown, err := guatiao.Redact(schema, value, "[redacted]")
```

Every field a schema marks (`x-sensitive` or `writeOnly`) is replaced at
any depth, and whatever the schema does not describe is hidden too. Local
`$ref`s are followed. `ErrWrongKind` for a schema that is not a map.

## For a package over another guatiao library

The `guatiao` package exports what the `guatiao-serde` and
`guatiao-intake` modules build on, and what any package binding a C
library that takes and answers guatiao values may use. A tree crosses as the address of its root
`guatiao_value`. Built by one library, it is read and freed by any
other, since a value records its own allocator; the library that built
it stays loaded while the tree lives.

- `Load() error`: opens the guatiao library now, as the first call needing
  it would.
- `Adopt(root, alloc unsafe.Pointer) *Value`: a Value taking over `root`,
  a `guatiao_value` in memory from C's `calloc` that a call wrote, built
  in the `guatiao_alloc` at `alloc`; the Value frees the tree and that
  memory. `Load` must have succeeded.
- `WithNodes(items []any, body func(nodes []unsafe.Pointer) error) error`:
  each item as a node for the call: a `Ref` or `*Value` as it is, plain
  data built for it in the core's allocator and freed after.
- `NodeOf(item any) (unsafe.Pointer, bool, error)`: the node a `Ref` or
  `*Value` names now; false for anything else.
- `Borrow(find func() unsafe.Pointer, keep any) (Ref, error)`: a read-only
  `Ref` onto what `find` answers, asked again on every access, refused
  once `keep`'s value is closed.
- `StatusError(status uint32, message string) error`: nil for OK, else
  the `*Error` for the status, carrying the library's words.

Its own library is a `*Surface`, declared once in a package-level
variable and never copied:

```go
var library = &guatiao.Surface{
	Basename: "guatiao_serde",          // guatiao_serde.dll, libguatiao_serde.so, libguatiao_serde.dylib
	Variable: "GUATIAO_SERDE_LIBRARY",  // its own variable, before GUATIAO_LIBRARY; "" for none
	Probe:    "guatiao_json_parse_ex",  // an export only it has; "" takes a GUATIAO_LIBRARY file as it is
	Exports:  map[string]string{"guatiao_json_parse_ex": "", "guatiao_toml_parse_ex": "toml"},
}
```

`Exports` maps each export the package calls to the feature a build needs
for it: `""` for one every build has, which a file must export to be
opened. It is found as "Finding the native library" says, with
`Variable` as its own variable.

- `(*Surface) Use(where string) error`: the file or directory to open
  instead of looking; another once it is open is an error.
- `(*Surface) Find() (string, error)`: the file the lookup settles on now,
  without opening it; a `*LibraryNotFound` naming every route.
- `(*Surface) Open() error`: opens the guatiao library as `Load` does, then
  this one, resolving `Exports` and `guatiao_alloc_default`; once open it
  stays open and Open does nothing. Not found is a `*LibraryNotFound`,
  kept for the process; a file lacking a required export a
  `*MissingSymbol`.
- `(*Surface) Symbol(name string) (unsafe.Pointer, error)`: opens it, then
  the address of an export `Exports` names, to call from the package's
  own C through a pointer of the export's type; a `*MissingSymbol` naming
  the feature when this build lacks it.
- `(*Surface) Alloc() unsafe.Pointer`: its default `guatiao_alloc`, kept
  for the process; nil before Open. `(*Surface) Path() string`: the file
  it was opened from.

## Errors

A failing `guatiao_status` is a `*guatiao.Error` (`Status`, `Message`: the
other side's own words, empty when it gave none), matched with
`errors.Is` against `ErrBadValue`, `ErrAllocFailed`, `ErrWrongKind`,
`ErrNotFound`, `ErrNullArgument`, `ErrGone`, `ErrUnsupported`,
`ErrBusy`, `ErrInternal`. The `Status` constants are `OK`, `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy`, `Internal`; `guatiao.Errorf(status, format,
args...)` builds one. `ErrClosed`, `ErrReadOnly`, `*LibraryNotFound`,
`*MissingSymbol` and `*FloorMismatch` are this binding's own;
`*civil.Unparsed` (and `guatiao-serde`'s `*serde.ParseError`) unwrap to `ErrBadValue`; a
channel's `*Refused` unwraps to the `*Error` of its status.

## Threads

A Value is not safe for concurrent use; reading one from several
goroutines at once is. A Registry is not; a remote may be called from any
goroutine, as the provider behind it allows.

A slot this module serves (a provider, an object) runs on the thread that
calls it, a thread Go did not start included: nothing is queued, so a
library's own threads may call a Go provider at once, and the provider
guards what it shares as any Go value shared between goroutines does.
