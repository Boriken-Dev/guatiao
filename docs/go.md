# Go: host and provider

Go plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a library,
Go types implementing a kind's interface, that a host in the same process
calls, or that it builds as a library file any host loads. Both use the
module `github.com/Boriken-Dev/guatiao/bindings/go/guatiao`: cgo over the ABI's C
header, for Go 1.26 and later, loading the guatiao library at run time and
shipping no native code of its own.

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao
```

cgo needs a C compiler: GCC or clang, and on Windows a MinGW-style one
(llvm-mingw, MinGW-w64 GCC). The binding loads the guatiao library on its
first native call: the one `guatiao.UseLibrary(path)` named, else
`GUATIAO_LIBRARY` (a file or a directory), else the platform's library
search.

## Three modules

The binding is a module per crate, released together under one version:

| module | package | library | its own variable |
| --- | --- | --- | --- |
| `github.com/Boriken-Dev/guatiao/bindings/go/guatiao` | `guatiao` (and `civil`) | `guatiao` | |
| `github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde` | `serde` | `guatiao_serde` | `GUATIAO_SERDE_LIBRARY` |
| `github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake` | `intake` | `guatiao_intake` | `GUATIAO_INTAKE_LIBRARY` |

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde   # requires guatiao at the same version
```

```go
import (
	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
)

doc, err := serde.Parse(`{"port": 5900}`, serde.JSON) // a *guatiao.Value

var cfg struct{ Port uint16 `guatiao:"port"` }
err = serde.Unmarshal(`{"port": 5900}`, serde.JSON, &cfg) // straight into Go data
```

`serde.Unmarshal` and `serde.Emit` of Go data build no tree: the document
crosses as wire bytes once, read and written in Go
(`guatiao.UnmarshalWire`, `guatiao.MarshalWire`), where a tree costs a
call per node to build.

The serde and intake modules find their libraries through their own
variable first, then `GUATIAO_LIBRARY` (a directory, or a file that
exports them, else the file beside it). They are built on the core's
`guatiao.Surface`, which opens any other guatiao C library a package binds.
In a checkout, `bindings/go/go.work` builds the three together.

## A kind, as an interface

A kind is the table its C header declares. In Go it is an interface
holding the slots every table has, marked with the kind's name and its C
table; each slot appended later is an interface of its own, as
`io.WriterTo` stands beside `io.Writer`. An object kind's interface has
`Close() error`, its `destroy` slot.

```go
// greeter_kind.go
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
type IntoGreeter interface{ GreetInto(name string, listener Listener) error }
type Checker interface{ Check(config guatiao.MapRef) (bool, error) }

//guatiao:kind listener listener_vtable
type Listener interface {
	Heard(what string) error
	Close() error
}

//guatiao:kind counter counter_vtable
type Counter interface {
	Count() (int64, error)
}
```

`go generate` runs `guatiao-kind`, which reads the kinds' description,
the `<crate>.kinds.json` a kind crate commits beside its header (the slot
order, the floor hash, which argument is handed over, lent or borrowed,
which slot can refuse), and the interfaces, pairs each slot with the Go
method of its name in CamelCase (`greet_into` is `GreetInto`), refuses a
Go type that cannot carry the C one, and writes the rest:

- `GreeterKind`, the descriptor a host binds with and a provider names;
- `GreeterRemote`, a method per slot, implementing `Greeter` and every
  optional interface;
- the cgo that calls each slot of a table and serves each slot over a Go
  value.

Nothing is described twice: the description is the source (`-header`
reads the same from the C header, for a kind with no description), and
`guatiao-kind -check` fails when the glue is not what it would write. A
kind with no header at all, such as one written in TypeScript, gets its
glue from its description alone; the marker's C table name is then left
out (`//guatiao:kind tally`).

**A kind born in Go** has no description to read: `guatiao-kind
-document <file>` reads the interfaces into one, writes it there, and
writes the glue from it. A Go type says most of what a description
holds; a directive in a doc comment says the rest. On the kind's
interface, `//guatiao:appended` names the interfaces of its appended
slots in table order, and `//guatiao:floorhash N` pins a hash, which is
otherwise computed as Rust would compute it for the same required slots.
On a method, `//guatiao:infallible` says the slot cannot fail, and
`//guatiao:lent`, `//guatiao:taken`, `//guatiao:nullable` and
`//guatiao:mut` name the arguments lent for the call, handed over, a
`Ref` that may be null, and bytes written in place. Doc comments are the
description's docs. `examples/tally_kind/go` declares the tally kinds
this way, and they describe byte for byte as the TypeScript declaration
of the same kinds does:

```go
// Counts by name.
//
//guatiao:kind tally
//guatiao:appended AllCounter Knower Checksummer Reporter Loader
type Tally interface {
	// Adds `by` to the count under `name`: the new count, or why not.
	Add(name string, by uint32) (int64, error)
	// The count under `name`: 0 when there is none.
	//
	//guatiao:infallible
	Total(name string) (int64, error)
}

// Optional: tells `watcher` each count, for this call alone.
type Reporter interface {
	//guatiao:lent watcher
	Report(watcher Watcher) error
}
```

| Go type | in the description |
| --- | --- |
| `string` | `str`; answered, `text` |
| `[]byte` | `bytes`; marked `mut`, `bytes_mut` |
| `bool`, `int8`..`int64`, `uint8`..`uint64`, `int`, `uint`, `float32`, `float64` | the scalar (`int` is `isize`, `uint` `usize`) |
| `guatiao.Ref`, `MapRef`, `ListRef` | a `value`, `map` or `list` borrowed (a list only lent); marked `lent`, lent; a `Ref` marked `nullable`, one that may be null |
| `*guatiao.Value` | a `value` handed over; answered, a `value` |
| a struct or map type, a slice, `any` | a `map`, a `list`, a `value` the binding converts: borrowed, or marked `taken`; answered, the same |
| an object kind's interface | an `object` handed over; marked `lent`, lent; answered, the object |

Every method returns an `error` last. A tree answered may be any Go type:
the binding converts Go values and trees as `encoding/json` does, structs
by their `guatiao` (or `json`) tags. The binding's
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/go/guatiao/AGENTS.md)
lists every C type and the Go types that carry it.

## As a host

```go
reg, err := guatiao.NewRegistry("my-host", "1.0")
if err != nil {
	return err
}
defer reg.Close()
reg.ScanDir("/opt/plugins", guatiao.Rules("kind=greeter"))

providers, _ := reg.Providers("greeter")
for _, p := range providers {
	greeter, err := greeterkind.GreeterKind.Bind(reg, fmt.Sprint(p["key"]))
	if err != nil {
		continue // a *guatiao.FloorMismatch: not this kind's table
	}
	answer, _ := greeter.Greet("ana")
	fmt.Println(answer.Greeting) // hello, ana
	if greeter.Has("greet_into") {
		greeter.GreetInto("bo", &ear{}) // any Go value with Heard and Close
	}
}

inst, err := reg.Create("acme_shouter", map[string]any{"prefix": "hey"})
defer inst.Close()
shouter, _ := greeterkind.GreeterKind.BindInstance(inst)
```

`Bind` checks the table against the kind (its size and its floor hash)
before anything in it runs. A slot an older table lacks answers
`guatiao.ErrUnsupported`, and `Has(name)` asks first. A refusal is a
`*guatiao.Error` whose status `errors.Is` matches against `ErrBadValue`,
`ErrNotFound` and the rest, with the provider's own words. A tree answered
is decoded into the method's Go type, or kept as a `*guatiao.Value` the
caller closes; an object answered is a remote whose `Close` destroys it.

Sources work as in every binding: `reg.AddSourceFile(path, config)` adds a
source such as the guatiao-source-py bridge, and `reg.Refresh()` loads what it
serves. A Go host calls greeters written in Python that way.

## As a provider

A provider is any Go type implementing its kinds' interfaces:

```go
type Hello struct{ asked atomic.Int64 }

var _ greeterkind.Greeter = (*Hello)(nil) // the compiler checks it

func (h *Hello) Greet(name string) (greeterkind.Greeting, error) {
	if name == "" {
		return greeterkind.Greeting{}, guatiao.Errorf(guatiao.BadValue, "nobody to greet")
	}
	return greeterkind.Greeting{Greeting: "hello, " + name}, nil
}

func (h *Hello) Count() (int64, error) { return h.asked.Add(1), nil }

type ShouterConfig struct {
	Prefix string `guatiao:"prefix" schema:"minLength=1"`
}

type Shouter struct{ prefix string }

func NewShouter(c ShouterConfig) (*Shouter, error) { return &Shouter{prefix: c.Prefix}, nil }

func (s *Shouter) Greet(name string) (greeterkind.Greeting, error) {
	return greeterkind.Greeting{Greeting: s.prefix + " " + strings.ToUpper(name)}, nil
}

lib, err := guatiao.LibraryOf(
	guatiao.Single("hello", &Hello{}, greeterkind.GreeterKind, greeterkind.CounterKind,
		guatiao.DisplayName("Hello, in Go")),
	guatiao.Configured("shouter", NewShouter, greeterkind.GreeterKind),
)
reg.RegisterLibrary(lib)
```

- **The compiler checks it**: a `var _ greeterkind.Greeter = (*Hello)(nil)`
  line refuses a type that misspells `Greet`, takes a number for the name
  or answers the wrong type. An optional slot is served when the type
  implements its interface. What a provider serves is said where it is
  declared, since Go cannot list the interfaces a type implements; a
  value that does not implement a kind it names is refused.
- **A configuration is a struct.** `Configured` builds an instance per
  configuration with its build function, whose argument's type gives the
  schema the host checks a configuration against first
  (`guatiao.SchemaOf[ShouterConfig]()`): names from the tags, keywords from
  a `schema` tag, every field required unless it is a pointer,
  `omitempty` or defaulted. `Single` is a provider that is its one
  instance.
- **An error is a refusal**: a `*guatiao.Error` (`guatiao.Errorf`) crosses
  with its status and words, any other error as `GUATIAO_ERR_INTERNAL`
  with its text, and a panic as `GUATIAO_ERR_INTERNAL` too.
- **An instance that implements `io.Closer`** is closed when its host
  destroys it.

**The package is the library.** `LibraryOf` names it by the providers'
package import path and versions it from the module that holds it, as
the build records it (`runtime/debug.ReadBuildInfo`): a module installed
at a tag records its version, and its VCS revision goes into the
library's meta. A build that records none (a checkout, a test binary) is
refused unless `guatiao.LibraryVersion` says one, so a host never files
providers under an invented version. `NewLibrary(id, version, ...)` builds
one with both given. A host in the same process, written in any language,
then reaches these providers through the registry's tables, or the host's
services from inside a library; the repository's `examples/greeter_caller`
is such a caller written in Rust, and the examples' tests have it call
every slot of a Go greeter.

## As a library file

Go compiles to a native library, so a Go library can be an ordinary file
a scan finds, with no bridge: a package `main` built with
`-buildmode=c-shared` exports the entry, and a C file declares what it
serves, which a scan reads before it maps the file.

```go
package main

import "C"

import "unsafe"

var library, _ = gogreeter.Library()

//export guatiao_library_entry
func guatiao_library_entry(host unsafe.Pointer) unsafe.Pointer { return library.Answer(host) }

func main() {}
```

```c
#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const char guatiao_declares[] = "kind=greeter\0kind=counter\0";
```

```bash
go build -buildmode=c-shared -o plugins/libgo_greeter.so ./cshared   # go_greeter.dll on Windows
```

Any host then loads it: the repository's Rust example host scans it and
calls every slot through the kind's Rust trait. The library finds its own
guatiao as any Go program using the binding does, through
`GUATIAO_LIBRARY` or the platform's search, when its entry is first
called; without one it declines the host rather than aborting it. Its
`unload` always refuses, so a host never unmaps it: the Go runtime inside
it cannot be unloaded.

## Threads

A slot a Go provider or object serves runs on the thread that calls it,
a thread Go did not start included: nothing is queued, so a library's own
threads call Go at once, and a provider guards what it shares as any Go
value shared between goroutines does. A `Value` is not safe for
concurrent use, nor is a `Registry`; a remote may be called from any
goroutine, as its provider allows. What C holds of a Go value is a
`cgo.Handle`, never a Go pointer.

## What Go does not do that Python does

- **Describe a kind while the program runs.** Serving a slot needs a C
  function, which Go can only get from an `//export` written at build
  time: a kind's glue comes from `go generate`, where Python reads a
  `Protocol` class when it runs.
- **Serve another process's modules.** guatiao-source-py serves Python modules
  to any host; nothing serves Go packages from source, though a Go library
  built as a file is loaded by any host, which Python's cannot be.
- **Build without limit.** A library's entry and its providers' `create`,
  `destroy` and `available` take no context naming them, so each comes
  from a fixed pool: 64 libraries and 256 providers per process.
- **Be unloaded.** A Go library file stays mapped once loaded.
- **Run a kind's default body** for a slot an older table lacks, as a
  Rust caller does: a remote answers `ErrUnsupported`, as in Python and
  Node.

## The examples

- `examples/go_host/` scans for greeters in any language, registers the
  Go library, optionally adds the guatiao-source-py bridge, and calls every slot
  of the kind; its README has the commands, and its tests call the Rust,
  C, Python and Go greeters both ways.
- `examples/go_provider/go_greeter/` is the library above, and
  `examples/go_provider/cshared/` the same library as a file.
- `examples/greeter_kind/go/` holds the greeter kinds as interfaces, and
  the glue `guatiao-kind` wrote for them.
- `examples/tally_kind/go/` holds the tally kinds born in Go, the glue
  written from the document they describe as, and a test serving them
  from Go and calling every slot.
