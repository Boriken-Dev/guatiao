# guatiao-builder

guatiao's side of [coqui-builder](https://github.com/Boriken-Dev/coqui-builder): a
guatiao schema, a filled `Partial` and a checked value made from the description of a
struct that is built field by field.

> **Status: alpha, not published.** coqui-builder is a git dependency until it is released.

```rust
use coqui_builder::Describe;
use guatiao_builder::{schema, to_value};

#[derive(Describe)]
struct Listener {
    host: String,
    #[map(default = 8080)]
    port: u16,
}

// A guatiao schema from the description, for a form or an editor.
let document = schema(&Listener::description()).unwrap();

// The value a built `Listener` is, and the `Listener` a value holds.
let value = to_value(&Listener { host: "h".into(), port: 1 }).unwrap();
let back: Listener = guatiao_builder::from_value(&value).unwrap();
# let _ = (document, back);
```
