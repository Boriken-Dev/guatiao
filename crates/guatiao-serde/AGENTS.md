# guatiao-serde — shipped API header

serde for guatiao values. One `Serialize` and one `DeserializeSeed`, so
every serde data format can carry a value.

Depends on `guatiao` and `serde`. Nothing else: the formats in
`[dev-dependencies]` are there to test against and are not a position on
which format you should use.

## The whole surface

```rust
// writing
to_serde(&Value) -> Serializable<'_>                 // default presentation
to_serde_with(&Value, Presentation) -> Serializable<'_>
impl Serialize for Serializable<'_>

// reading
ValueSeed::new(Alloc) -> ValueSeed                   // default presentation
ValueSeed::with(Alloc, Presentation) -> ValueSeed
impl DeserializeSeed<'de> for ValueSeed              // ::Value = Value
from_serde<D: Deserializer>(D) -> Result<Value, D::Error>   // crate allocator

// policy
Presentation::new() -> Presentation                  // const
  .bytes(Bytes) -> Presentation                      // const
  .reading_data_uris() -> Presentation               // const
  .bytes_as() -> Bytes  /  .reads_data_uris() -> bool
enum Bytes { DataUri (default), Base64, Array, Refuse }

// the encoding the spellings are built on
base64(&[u8]) -> String        from_base64(&str) -> Option<Vec<u8>>
data_uri(&[u8]) -> String      from_data_uri(&str) -> Option<Vec<u8>>
```

## Why a wrapper and a seed rather than plain impls

**Serialising carries a policy**, because JSON has no byte string and the
spelling is the document author's decision, not this crate's.

**Deserialising carries an allocator**, because every guatiao container
records the one that made it and `Deserialize::deserialize` takes no
arguments. `DeserializeSeed` is serde's own mechanism for that, and it is
what lets a host read straight into its own arena.

(The orphan rule would forbid the plain impls anyway — a consequence, not
the reason.)

## Contracts

- **A map is written in insertion order, never sorted.** A reader that
  wants them sorted can sort them; a writer that sorted them would destroy
  an order somebody chose and no reader could get it back.
- **A repeated key on read REPLACES**, the same thing `Map::set` does.
- **A number goes out verbatim.** `1.10` stays `1.10`; `1e400` and a
  200-digit integer stay numbers. `i64`/`u64` go out native; past that a
  human-readable format gets the raw text spliced in (serde_json's
  reserved-token protocol) and a binary format gets an `f64`, which is all
  it can hold.
- **A number comes back only as exactly as the format hands it over.**
  `serde_json` without `arbitrary_precision` rounds through `f64` before a
  visitor runs. Not enabled here: cargo unifies features across a build.
- **Bytes go native into a format that has them**, whatever `Presentation`
  says — a native byte string round-trips and a spelling cannot.
- **A `data:;base64,` string becomes bytes only with
  `reading_data_uris()`.** On, bytes survive a JSON round trip; also on, a
  string that happens to look like a data URI silently becomes bytes.
- **`GUATIAO_ABSENT` is refused, not written as null.** Absent is the
  answer to a lookup, not something a container holds; a map expresses it
  by not holding the key.
- **A non-UTF-8 map key is refused.** Keys are raw bytes; a lossy
  replacement would silently rename one.
- **An unknown tag is refused**, rather than written as something a reader
  would believe.

## Gotchas

- `base64` is the **standard** alphabet, not URL-safe: inside a JSON
  string `+` and `/` need no escaping, so URL-safe would buy nothing and
  differ from every other producer.
- `from_base64` is strict — wrong length, a character outside the
  alphabet, or padding anywhere but the end is `None`. A lenient decoder
  turns a typo into different bytes.
- Escaping is serde's. This crate has no second answer to it.
