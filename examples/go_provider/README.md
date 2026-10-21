# Providers written in Go

`go_greeter/` is a guatiao library written in Go: a package serving the
`greeter` and `counter` kinds:

- `Hello`, its one instance, serving both kinds and every optional slot
  of the greeter; its `Start` hands back a conversation (`Chat`), an
  object that tells the listener it was handed and closes it when it is
  closed.
- `Shouter`, built per configuration from a `ShouterConfig`, a struct
  whose tags give the schema the host checks first.

The kinds are the interfaces in
[`../greeter_kind/go`](../greeter_kind/go/greeter_kind.go). Each provider
type implements them, and a `var _ greeterkind.Greeter = (*Hello)(nil)`
line has the compiler check it.

The package is a library as the binding reads one: `guatiao.LibraryOf`
names it by the package's import path and takes its version from the
build, or from `guatiao.LibraryVersion` when the build records none (a
checkout's does not; a module installed at a tag does).

## In a Go host

```go
lib, err := gogreeter.Library()
reg.RegisterLibrary(lib)
```

The example host, [`../go_host`](../go_host/), does so.

## As a library file

`cshared/` is the same library as a file any host's scan finds: a package
`main` exporting `guatiao_library_entry`, answered by the library, and a
C file declaring `guatiao_declares`, which a scan reads before it maps the
file.

```bash
cd examples
go build -buildmode=c-shared -o go-greeter/libgo_greeter.so ./go_provider/cshared   # go_greeter.dll on Windows
GUATIAO_LIBRARY=../target/debug GREETER_HOST_PATH=go-greeter ../target/debug/greeter_host
```

The Rust example host loads it and calls every slot through the kind's
Rust trait. The library loads its own guatiao through `GUATIAO_LIBRARY`
when its entry is first called; without one it declines the host rather
than aborting it. Its `unload` always refuses, so the host leaves it
mapped: the Go runtime inside it cannot be unloaded.

`go test ./go_provider/cshared` builds the file and runs the Rust host
against it; `examples/greeter_host/tests/go_library.rs` does the same from
Rust when `GUATIAO_GO_LIBRARY` names the directory the file is in, and
checks that the host cannot unmap it.
