# guatiao-serde — shipped API header

serde for guatiao values. One `Serialize` and one `DeserializeSeed`, so
every serde data format can carry a value.

Depends on `guatiao` and `serde`. Nothing else: the formats in
`[dev-dependencies]` are there to test against and are not a position on
which format you should use.

## The whole surface

```rust
// writing (src/ser.rs)
impl From<&Value> for Serializable<'_>               // default presentation
Serializable::new(&Value, Presentation) -> Serializable<'_>
Serializable::value() -> &Value  /  .presentation() -> Presentation
impl Serialize for Serializable<'_>

// reading (src/de.rs)
ValueSeed::new(Alloc) -> ValueSeed                   // default presentation
ValueSeed::with(Alloc, Presentation) -> ValueSeed
impl DeserializeSeed<'de> for ValueSeed              // ::Value = Value
from_serde<D: Deserializer>(D) -> Result<Value, D::Error>   // crate allocator

// policy
Presentation::new() -> Presentation                  // const, as is every method
  .bytes(Bytes) / .numbers(Numbers) / .reading_data_uris() -> Presentation
  .bytes_as() -> Bytes  /  .numbers_as() -> Numbers  /  .reads_data_uris() -> bool
enum Bytes { DataUri (default), Base64, Array, Refuse }     // #[non_exhaustive]
  Bytes::read_data_uris(self) -> bool                // const; see the gotchas
enum Numbers { Native (default), RawText }                  // #[non_exhaustive]

// the encoding the spellings are built on
base64(&[u8]) -> String        from_base64(&str) -> Option<Vec<u8>>
data_uri(&[u8]) -> String      from_data_uri(&str) -> Option<Vec<u8>>

// the formats: `pub mod text`, one module per feature; `Error` is re-exported
enum Error { Format { format, detail }, Unsupported { format, detail } }  // #[non_exhaustive]
  Error::format() -> &'static str                    // which format refused
text::json::{NAME, to_string, to_string_pretty, from_str}   // feature `json`
text::toml::{NAME, to_string, from_str}                     // feature `toml`
text::yaml::{NAME, to_string, from_str}                     // feature `yaml`
// each: to_string(&Value, Presentation) -> Result<String, Error>
//       from_str(&str, Alloc, Presentation) -> Result<Value, Error>
```

`text::json` is the one thing that sets `Numbers::RawText`; `text::toml`
refuses a value that is not a map, because a TOML document is a table.

## From C (`include/guatiao_serde.h`)

`pub mod exports`, always compiled — a surface that appears only when
somebody remembers a flag is one a C caller cannot rely on.

```c
guatiao_status guatiao_json_parse       (guatiao_str text, uint32_t how,
                                         const guatiao_alloc *alloc, guatiao_value *out);
guatiao_status guatiao_json_emit        (const guatiao_value *value, uint32_t how,
                                         const guatiao_alloc *alloc, guatiao_value *out);
guatiao_status guatiao_json_emit_pretty (/* as _json_emit  */);
guatiao_status guatiao_toml_parse       (/* as _json_parse */);   // GUATIAO_SERDE_TOML
guatiao_status guatiao_toml_emit        (/* as _json_emit  */);   // GUATIAO_SERDE_TOML
guatiao_status guatiao_yaml_parse       (/* as _json_parse */);   // GUATIAO_SERDE_YAML
guatiao_status guatiao_yaml_emit        (/* as _json_emit  */);   // GUATIAO_SERDE_YAML

#define GUATIAO_BYTES_DATA_URI 0          /* the `how` argument's LOW BYTE */
#define GUATIAO_BYTES_BASE64   1
#define GUATIAO_BYTES_ARRAY    2
#define GUATIAO_BYTES_REFUSE   3
#define GUATIAO_READ_DATA_URIS (1 << 8)   /* and the bits above it, flags */
#define GUATIAO_PRETTY         (1 << 9)   /* JSON only; read by _json_emit */
```

- **Emitting answers a VALUE holding the text**, freed with
  `guatiao_value_free` like any other tree. No second thing to free.
- **A format is a build-time choice, so its declarations are guarded.**
  Only `json` is on by default; compile with `-DGUATIAO_SERDE_TOML` and/or
  `-DGUATIAO_SERDE_YAML` to declare the pairs the library was built with.
- Statuses: `GUATIAO_ERR_NULL` for a null pointer the call needs,
  **`GUATIAO_ERR_ALLOC` for an allocator that is null or cannot
  allocate**, `GUATIAO_ERR_BAD_VALUE` for text that is not UTF-8 or a
  document the format refused, `GUATIAO_ERR_WRONG_KIND` for a value the
  format cannot carry, `GUATIAO_ERR_INTERNAL` if a panic was caught.

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
- **A number goes out verbatim under `Numbers::RawText`, and only then.**
  `i64`/`u64` go out native whatever the policy, which is exact either
  way. Past that, `RawText` splices the number's own text in through
  serde_json's reserved token — `1.10` stays `1.10`, `1e400` and a
  200-digit integer stay numbers — and `text::json` is the one thing that
  sets it. The **default** `Presentation`, which `Serializable::from`
  carries, goes through an `f64` instead, so `1.10` goes out `1.1`: the
  token means nothing to TOML or YAML, which would write the struct out
  literally. Use `text::json`, or state the policy, when the spelling
  matters.
- **A number comes back exactly**, because the default **`json`** feature
  turns `serde_json/arbitrary_precision` on and a number arrives as its
  own text rather than as an `f64` resolved before a visitor runs. It
  rides with the format rather than being offered separately: cargo
  unifies features across a build, so it reaches `serde_json::Value`
  everywhere in a consumer's graph. A consumer who does not want that
  turns `json` off and hands a `serde_json::Deserializer` to `ValueSeed`.
- **A document nests at most `MAX_DEPTH` containers deep**, refused past
  that. `serde_json` caps its own recursion; another format need not, and
  this crate reads them all.
- **The raw-number token is read whether or not that feature is on.**
  Any crate in a consumer's graph can enable
  `serde_json/arbitrary_precision`; a reader that did not know the token
  would then turn every number into a map, with no error to show for it.
  Note the two tokens differ: the writer splices raw JSON through
  `…::RawValue`, a number arrives under `…::Number`.
- **Bytes go native into a format that has them**, whatever `Presentation`
  says — a native byte string round-trips and a spelling cannot.
- **A `data:;base64,` string becomes bytes only with
  `reading_data_uris()`.** On, bytes survive a JSON round trip; also on, a
  string that happens to look like a data URI silently becomes bytes.
- **`GUATIAO_ABSENT` is refused, not written as null.** Absent is the
  answer to a lookup, not something a container holds; a map expresses it
  by not holding the key.
- **A map with a key that is not UTF-8 is refused**, as the value model
  refuses it; a lossy replacement would silently rename the key.
- **An unknown tag is refused**, rather than written as something a reader
  would believe — and so is a node whose tag and payload disagree, rather
  than written as a `0`, a `false` or an empty string.

## Gotchas

- `base64` is the **standard** alphabet, not URL-safe: inside a JSON
  string `+` and `/` need no escaping, so URL-safe would buy nothing and
  differ from every other producer.
- `from_base64` is strict — wrong length, a character outside the
  alphabet, or padding anywhere but the end is `None`. A lenient decoder
  turns a typo into different bytes.
- Escaping is serde's. This crate has no second answer to it.
- `Bytes::read_data_uris` is a constant `false` and ignores `self`. The
  live switch is `Presentation::reading_data_uris()`, read back with
  `reads_data_uris()`.
- **`unsafe` lives in `src/exports.rs` alone**, checked by
  `tests/unsafe_stays_in_exports.rs`; every other module carries
  `#![forbid(unsafe_code)]`.
