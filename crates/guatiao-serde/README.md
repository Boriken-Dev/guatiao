# guatiao-serde

serde for [guatiao](../guatiao) values: **one pair of impls, every serde
format**.

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

JSON, MessagePack, CBOR, TOML, YAML, RON — and a format this crate has
never heard of works too, because it speaks serde rather than any one
format.

## Two things it does that a derive could not

**Serialising carries a policy.** JSON has no byte string, so something
has to decide how one is spelled. `Presentation` is that decision, and it
belongs to whoever writes the document: a `data:;base64,` URI by default,
or bare base64, or an array of numbers, or a refusal. A format that *has*
a byte string gets one natively whatever the policy says.

**Deserialising carries an allocator.** Every guatiao container records
the allocator that made it, and `Deserialize::deserialize` takes no
arguments to name one with. `ValueSeed` is serde's own answer —
`DeserializeSeed` exists to carry state into a deserialisation — and it is
what lets a host read a document straight into its own arena.

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

## Badges

None yet, deliberately: the crate is `publish = false`, so a crates.io or
docs.rs badge would link to a page that does not exist. They go in with
the first release.

## Licence

MPL-2.0, like the rest of the workspace.
