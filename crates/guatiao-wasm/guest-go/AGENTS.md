# `guest` (Go) — API header

Header-file-style reference for the Go module
`github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go` (package
`guest`): providers written in Go, compiled to a WebAssembly module that
the `guatiao-wasm` source serves (guatiao ABI 1, in that crate's
`AGENTS.md`). Go 1.24 or later, standard library only.

A library is a `main` package built as a WASI reactor:

```bash
GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared -o mylib.wasm .
```

Its manifest states `version`: Go writes no custom section.

## A library

```go
func init() {                        // a reactor's main never runs
	guest.Serve(&guest.Library{Providers: []guest.Provider{{
		ID: "hello", Name: "Hello, in Go", Handler: &hello{},   // served as <module>_hello
		Serves: []guest.Serves{{Kind: "greeter", Slots: []string{"greet", "start"}}},
	}}})
}
func main() {}
```

```go
type Handler interface{ Call(c *Call) (any, error) }
type HandlerFunc func(c *Call) (any, error)
type Releaser interface{ Release() }       // an object's handler: told when the caller destroyed it
```

**Answer**: `nil` for nothing; `&guest.Object{Handler: h}` for an object
kind's answer (its calls reach `h` until destroyed; a `Releaser` hears
that); anything else a value. **Errors**: a `*Refusal` crosses with its
status (`guest.Refuse(guest.StatusNotFound, "no %s", x)`); any other
error is `StatusBadValue` with its text; a panic is recovered and is
`StatusInternal` (`"panic: ..."`), and the module carries on.

| `*Call` | |
| --- | --- |
| `Kind()`, `Method()`, `Len()`, `Arg(i)` | what is called, an argument as it arrived |
| `Text`, `Bytes`, `Int`, `Uint`, `Float`, `Bool`, `Map` | read as that, else a refusal naming the argument |
| `LentMap(i)`, `SetLent(i, v)` | a lent tree: written back after a successful call |
| `Buffer(i)` | the caller's `bytes_mut`, its length: written back |
| `Object(i)` | an object handed over: `Drop()` it (Go has no destructor; one not dropped goes with the instance) |
| `LentObject(i)` | an object lent for this call |

`(*HostObject).Call(method, args...) (any, error)` answers `guest.Absent`
for a slot that answers nothing; `Handle()` hands the object on.

## Values

`Absent`, `nil` (null), `bool`, `Number` (exact text: `Int()`, `Uint()`,
`Float()`), `string`, `[]byte`, `[]any` (a list), `*Map` (ordered: `NewMap`,
`Set`, `Get`, `Keys`, `Len`). `Encode` also takes Go's integers and floats
(as a `Number`; NaN and the infinities refused) and `map[string]any` (keys
sorted). `Decode` refuses every malformation with a `*WireError` naming
the byte; `ValidNumber` is the number grammar. Checked against the shared
vectors (`crates/guatiao/tests/vectors/wire.json`) by `go test`.

## Gotchas

- **Register in `init`**: a reactor runs package initialisers when the
  host calls `_initialize`, never `main`.
- **Go's runtime starts in about 430 000 fuel**: an `operations` limit
  below that refuses the library at open.
- **No files, no network**: the host grants WASI only stdio, a clock and
  randomness, and preopens no directory, so there is no file to open.
