# guatiao-serde

[![Crate](https://img.shields.io/crates/v/guatiao-serde.svg)](https://crates.io/crates/guatiao-serde)
[![docs.rs](https://img.shields.io/docsrs/guatiao-serde)](https://docs.rs/guatiao-serde)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

serde for [guatiao](https://crates.io/crates/guatiao) values: **one pair of
impls, every serde format**. JSON, MessagePack, CBOR, TOML, YAML, RON — and
a format this crate has never heard of works too, because it speaks serde
rather than any one format.

> **Status: alpha.** Released in lockstep with `guatiao`.

## Features

- **Serialising carries a policy.** JSON has no byte string, so something
  has to decide how one is spelled. `Presentation` is that decision, and it
  belongs to whoever writes the document: a `data:;base64,` URI by default,
  or bare base64, or an array of numbers, or a refusal. A format that *has*
  a byte string gets one natively whatever the policy says.
- **Deserialising carries an allocator.** Every guatiao container records
  the allocator that made it, and `Deserialize::deserialize` takes no
  arguments to name one with. `ValueSeed` is serde's own answer —
  `DeserializeSeed` exists to carry state into a deserialisation — and it is
  what lets a host read a document straight into its own arena.
- **Numbers stay exact through `text::json`**, spelling included — `1.10`,
  `1e400`, a 200-digit integer.
- **A C surface** with its own `include/guatiao_serde.h`.

## Installation

```toml
[dependencies]
guatiao-serde = { version = "0.0.0-alpha.0", features = ["toml", "yaml"] }
```

| Flag | Adds | Needed for |
| --- | --- | --- |
| `json` (default) | `serde_json`, with `arbitrary_precision` | `text::json`, and numbers read exactly |
| `toml` | `toml` | `text::toml` |
| `yaml` | `serde-saphyr` | `text::yaml` |
| `c-header` | `cbindgen` | regenerating the committed C header |

## Quick start

```rust
use guatiao::value::alloc::Alloc;
use guatiao::{Map, Value};
use guatiao_serde::{Serializable, ValueSeed, text::json};
use serde::de::DeserializeSeed;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# use guatiao_serde::Presentation;

let mut map = Map::new();
map.set("port", 5900)?;
let map = Value::from(map);

let text = json::to_string(&map, Presentation::new())?;        // {"port":5900}
let packed = rmp_serde::to_vec(&Serializable::from(&map))?;    // MessagePack

let mut de = serde_json::Deserializer::from_str(&text);
let back = ValueSeed::new(Alloc::rust()).deserialize(&mut de)?;
# assert_eq!(text, r#"{"port":5900}"#);
# assert!(!packed.is_empty());
# let back: &Map = (&back).try_into()?;
# let port: u32 = back.required("port")?.try_into()?;
# assert_eq!(port, 5900);
# Ok(())
# }
```

## What survives, and what does not

| | out | in |
|---|---|---|
| numbers within `i64`/`u64` | exact | exact |
| numbers past that, via `text::json` | **exact** (`1.10`, `1e400`, 200 digits) | **exact**, spelling included |
| numbers past that, default `Presentation` | rounded through `f64` | — |
| bytes, binary format | native | native |
| bytes, JSON | per `Presentation` | string, unless asked |
| absent | refused | n/a |

Two asymmetries are worth knowing before you rely on them.

**Verbatim is a policy, not the default.** `Numbers::RawText` splices a
number's own text into the document through serde_json's reserved token,
and `text::json` sets it. The default `Presentation` — what
`Serializable::from(&value)` carries — does not, because the token means
nothing to TOML or YAML, which would write it out literally. Under the
default, anything past `i64`/`u64` goes out through an `f64`, so `1.10`
becomes `1.1`.

**Reading is exact because `json` turns `arbitrary_precision` on.** That
is a default feature of this crate: a number arrives as its own text
rather than as an `f64` resolved before a visitor ever runs. Cargo unifies
features across a build, so it reaches `serde_json::Value` everywhere in a
consumer's graph — turn the `json` feature off and hand a
`serde_json::Deserializer` to `ValueSeed` yourself if that is not wanted.

## API overview

| Item | Purpose |
| --- | --- |
| `Serializable` | a value as a `serde::Serialize`, with its `Presentation` |
| `ValueSeed` | a `DeserializeSeed` that builds a value through a named allocator |
| `Presentation`, `Bytes`, `Numbers` | how bytes and out-of-range numbers are written |
| `text::{json, toml, yaml}` | `to_string` / `from_str` for each, behind its feature |

## Development

Part of the [guatiao](https://github.com/Boriken-Dev/guatiao) workspace;
its README has the commands.

## License

**Mozilla Public License 2.0**, like the rest of the workspace — see
[LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
