# A plugin host in Go

`main.go` loads greeter libraries written in any language and calls each
one through the `greeter` kind's table: a map back, a map lent and
changed in place, a map handed over, an object handed in and one handed
back (a conversation), an object lent for one call (a listener). A
provider that builds instances from a configuration is given one its
schema refuses, then one it accepts. The greeters written in Go
([`../go_provider`](../go_provider/)) are linked into the program and
registered beside the ones it loads. It closes everything it opened and
prints what it did.

The kinds are the interfaces in
[`../greeter_kind/go`](../greeter_kind/go/greeter_kind.go), their glue
written by `guatiao-kind` from the kind's C header
([`../greeter_kind/include/greeter_kind.h`](../greeter_kind/include/greeter_kind.h)).

## Run it

From `examples/` (a Go module of its own, using the binding in
`bindings/go/guatiao`), with cgo and a C compiler, after building the workspace:

```bash
cargo build --workspace --all-features
cd examples
export GUATIAO_LIBRARY=../target/debug
go run ./go_host ../target/debug
```

`target/debug` holds the Rust greeters (`derived_greeter`, written with
the derive macros) and `hello_library`, whose table is not this kind's,
which the host reports and passes over.

To add the greeter written in C, build it into a directory of its own and
name that directory too:

```bash
mkdir -p c-greeter
cc -shared -fPIC -std=c11 -I ../crates/guatiao/include -I greeter_kind/include \
   -o c-greeter/libc_greeter.so ../crates/guatiao/tests/c_consumer/greeter_in_c.c
go run ./go_host ../target/debug c-greeter
```

On Windows the output is `c-greeter/c_greeter.dll`, built with `clang
-shared` and no `-fPIC`; on macOS, `c-greeter/libc_greeter.dylib`.

To add the greeters written in Python, through the `guatiao-source-py` bridge
(built as [`../python_provider/README.md`](../python_provider/README.md)
says), name the bridge, where its manifests are, and what they import:

```bash
go run ./go_host ../target/debug \
  -python-source ../crates/guatiao-source-py/target/debug/libguatiao_source_py.so \
  -python-dir python_provider \
  -python-path ../bindings/python/guatiao/src -python-path greeter_kind/python
```

`go test ./go_host` runs every greeter (Rust, C, Python when
`GUATIAO_PY_BRIDGE` names a bridge, and Go) through every slot, has the
Rust caller in `greeter_caller` call each of them through the kind's Rust
proxy, and calls the Go greeter from native threads.

## What it shows

- **One description of the kind serves every language.** The Rust, C,
  Python and Go greeters are called through the same `GreeterRemote`; a
  table whose `FLOOR_HASH` is not the kind's is refused before anything
  in it runs (`*guatiao.FloorMismatch`).
- **A provider may leave optional slots out.** The C greeter fills four of
  six slots; `Has(name)` asks before calling, and calling a missing slot
  answers `guatiao.ErrUnsupported`.
- **A refusal crosses with its words**: `Greet("")` answers an
  `ErrBadValue` carrying the provider's message.
- **Objects cross both ways.** The listener is a plain Go type with
  `Heard` and `Close`; the conversation a greeter hands back is closed,
  which calls its `destroy` slot.
- **Unmapped when the library agrees.** A Rust library counts what it
  handed out and agrees once nothing is alive; the C greeter has no say,
  and the library written in Go is linked rather than mapped, so each is
  retired instead.
