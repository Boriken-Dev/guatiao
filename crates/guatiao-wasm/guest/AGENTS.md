# `guatiao-wasm-guest` — API header

Header-file-style reference for `guatiao-wasm-guest`: providers written in
Rust, compiled to a WebAssembly module that the `guatiao-wasm` source
serves (guatiao ABI 1, in that crate's `AGENTS.md`). It ships inside the
package and is self-contained. Development documentation lives with the
source at <https://github.com/Boriken-Dev/guatiao>.

No features. MSRV 1.89. Depends on `guatiao` (default features: values and
their wire encoding, the same code the host reads them with). Not
published; a workspace of its own. A library is a `cdylib` built with
`cargo build --release --target wasm32-unknown-unknown` and `panic =
"abort"`.

The root re-exports `Answer`, `Call`, `Handler`, `HostObject`, `Refusal`,
`Library`, `Provider`, the macro `export!`, `guatiao` itself and its
`Value`, `Map`, `List`, `Status`. The root is `deny(unsafe_code)`; `host.rs`
(the imports) and `glue.rs` (what the exports call) allow it.

## A library

```rust
fn library() -> Library {
    Library::new()
        .provider(Provider::new("hello", Hello::default())     // served as <module>_hello
            .name("Hello, in Rust")                           // default: the id
            .serves("greeter", ["greet", "check", "start"])   // the slots it answers
            .serves("counter", ["count"]))
}
guatiao_wasm_guest::export!(library, version = env!("CARGO_PKG_VERSION"));
// or export!(library): the manifest then states `version`
```

`export!` writes `guatiao_guest_abi`, `_alloc`, `_declare`, `_call` and
`_release` (`#[unsafe(no_mangle)]`, so the crate cannot
`forbid(unsafe_code)`) and, with `version`, a `guatiao.version` custom
section. The library is made on the first call, once per instance: a
trapped module is made afresh, its state with it.

## A provider (`Handler`)

```rust
pub trait Handler: 'static {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal>;
}
// also any FnMut(&mut Call) -> Result<Answer, Refusal> + 'static
```

| `Call` | |
| --- | --- |
| `kind()`, `method()`, `len()`, `is_empty()` | what is called |
| `arg(i) -> &Value` | as it arrived |
| `text(i)`, `bytes(i)`, `int(i)`, `uint(i)`, `float(i)`, `flag(i)`, `map(i)` | read as that, else a `bad_value` refusal naming the argument |
| `take(i) -> Value`, `take_map(i) -> Map` | a tree handed over (`taken`) |
| `lent(i) -> &mut Value`, `lent_map(i) -> &mut Map` | a lent tree: written back after a successful call |
| `buffer(i) -> &mut [u8]` | the caller's `bytes_mut`, its length: written back |
| `object(i) -> HostObject` | an object handed over: dropping it lets the host destroy it |
| `lent_object(i) -> HostObject` | an object lent for this call: refused (`GONE`) after |

| `Answer` | for |
| --- | --- |
| `Answer::Nothing`, `().into()` | a slot that answers nothing |
| `Answer::Value(v)`, `.into()` from `Value`, `Map`, `List`, `bool`, `i64`, `u64`, `&str`, `String`; `Answer::value(x)` | a value: the host narrows it to the slot's answer, refusing what does not fit |
| `Answer::object(handler)` | an object kind's answer: its calls reach `handler` until the caller destroys it, which drops it |

`Refusal::new(Status, words)`, `bad_value`, `not_found`, `unsupported`;
`From<&str>` and `From<String>` are `bad_value`. **A panic** aborts the
module: the host names its message and location in the error
(`GUATIAO_ERR_INTERNAL`) and makes the library afresh.

## Host objects (`HostObject`)

```rust
listener.call("heard", [Value::from("hi")])? -> Option<Value>   // None: the slot answers nothing
listener.into_handle() -> i64                                   // hand it on: Value::from(handle)
```

The arguments are the slot's, in order: scalars, text, bytes, borrowed or
taken trees, a taken object as its handle. A refusal comes back as a
`Refusal` with the host's status.

## Gotchas

- **It re-exports guatiao's C functions**: a `cdylib` exports every
  `#[no_mangle]` function of its dependencies, so a module carries
  guatiao's C surface (about 270 KB). The host ignores them.
- **A host object cannot call this library back**: the host refuses a
  call into a library already in a call on that thread
  (`GUATIAO_ERR_BUSY`).
- **Off wasm32 the imports are stand-ins** that keep what they are given,
  for this crate's own tests: a library is only useful built for wasm32.
