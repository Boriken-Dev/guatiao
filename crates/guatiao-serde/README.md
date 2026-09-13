# guatiao-serde

serde for [guatiao](../guatiao) values: **one pair of impls, every serde
format**.

```rust
use guatiao::value::types::Value;
use guatiao_serde::{Serializable, ValueSeed};
use serde::de::DeserializeSeed;

let mut map = Value::map();
map.set("port", 5900)?;

let json = serde_json::to_string(&Serializable::from(&map))?;          // {"port":5900}
let packed = rmp_serde::to_vec(&Serializable::from(&map))?;            // MessagePack

let mut de = serde_json::Deserializer::from_str(&json);
let back = ValueSeed::new(Alloc::rust()).deserialize(&mut de)?;
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
| numbers past that | **exact** (`1.10`, `1e400`, 200 digits) | rounded through `f64` |
| bytes, binary format | native | native |
| bytes, JSON | per `Presentation` | string, unless asked |
| absent | refused | n/a |

The one asymmetry is worth knowing before you rely on it: `serde_json`
without `arbitrary_precision` resolves a big number to an `f64` **before a
visitor is ever called**, so the spelling is gone before this crate sees
it. That feature is not enabled here because cargo unifies features across
a build, and turning it on would change `serde_json::Value` for every
other crate in a consumer's graph. Enable it yourself if you need it.

## Licence

MPL-2.0, like the rest of the workspace.