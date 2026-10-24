# guatiao-wasm-guest

Providers written in Rust, compiled to WebAssembly and served to any
guatiao host by the [`guatiao-wasm`](../README.md) source, sandboxed.

> **Status: alpha, not published.** Used from this repository by path.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
guatiao-wasm-guest = { path = "../../crates/guatiao-wasm/guest" }

[profile.release]
panic = "abort"
```

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

A library builds its providers in a function and hands it to `export!`;
each provider is a `Handler` called with the kind and slot named and
guatiao's own values as arguments:

```rust,ignore
use guatiao_wasm_guest::{Answer, Call, Handler, Library, Provider, Refusal};

#[derive(Default)]
struct Counter(i64);

impl Handler for Counter {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        match call.method() {
            "count" => {
                self.0 += 1;
                Ok(self.0.into())
            }
            other => Err(Refusal::unsupported(format!("no `{other}`"))),
        }
    }
}

fn library() -> Library {
    Library::new().provider(Provider::new("counter", Counter::default()).serves("counter", ["count"]))
}

guatiao_wasm_guest::export!(library, version = env!("CARGO_PKG_VERSION"));
```

[`examples/wasm_greeter_rust`](../../../examples/wasm_greeter_rust) serves
every slot of the example kinds: lent and handed-over maps, a buffer, a
listener the host lends or hands over, a conversation it answers. The API
is in [`AGENTS.md`](AGENTS.md).

## License

MPL-2.0.
