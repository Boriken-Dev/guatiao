# guest-go

Providers written in Go, compiled to WebAssembly and served to any
guatiao host by the [`guatiao-wasm`](../README.md) source, sandboxed. Go
1.24 or later builds them with its own toolchain (`go:wasmexport`); no
TinyGo, no cgo.

> **Status: alpha.** Used from this repository through a `replace`.

```go
// go.mod
require github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go v0.0.0
replace github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go => ../../crates/guatiao-wasm/guest-go
```

```go
package main

import "github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go"

type counter struct{ n int64 }

func (c *counter) Call(call *guest.Call) (any, error) {
	if call.Method() != "count" {
		return nil, guest.Refuse(guest.StatusUnsupported, "no `%s`", call.Method())
	}
	c.n++
	return c.n, nil
}

func init() {
	guest.Serve(&guest.Library{Providers: []guest.Provider{{
		ID: "counter", Handler: &counter{},
		Serves: []guest.Serves{{Kind: "counter", Slots: []string{"count"}}},
	}}})
}

func main() {}
```

```bash
GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared -o mylib.wasm .
go test ./...      # natively: the wire codec against the shared vectors, and the glue
```

[`examples/wasm_greeter_go`](../../../examples/wasm_greeter_go) serves
every slot of the example kinds. The API is in [`AGENTS.md`](AGENTS.md).

## License

MPL-2.0.
