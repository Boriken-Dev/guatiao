# guatiao-serde — shipped API header

serde for guatiao values. One `Serialize` and one `DeserializeSeed`, so
every serde data format can carry a value.

Depends on `guatiao` and `serde`; each format is a feature that adds its
parser (`json` is on by default and adds `serde_json`). The formats in
`[dev-dependencies]` are there to test against and are not a position on
which format you should use.

| Flag | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `json` | `serde_json` (with `raw_value`) | `text::json` | yes |
| `exact-numbers` | `json` and `serde_json/arbitrary_precision` | reading a JSON number as its own text | no |
| `toml` | `toml` | `text::toml` | no |
| `yaml` | `serde-saphyr` | `text::yaml` | no |
| `c-header` | `cbindgen` | regenerating `include/guatiao_serde.h` | no |

**`exact-numbers` is not additive for the rest of your build.** Cargo
unifies `serde_json`'s features, so with it on every crate using
`serde_json` sees `arbitrary_precision`, and `#[serde(untagged)]` and
`#[serde(flatten)]` numbers there stop deserialising. A library crate
should leave it off; a binary, or the C artifacts (built `--all-features`),
may turn it on.

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
enum Error { Format { format, detail },              // a writer refused a value
             Syntax { format, position: Option<Position>, detail },  // not a document
             Unsupported { format, detail } }        // #[non_exhaustive]
struct Position { line: usize, column: usize }       // both from 1; a column counts chars
  Error::format() -> &'static str                    // which format refused
  Error::position() -> Option<Position>              // where a parser stopped, if it said
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

/* where a parse stopped: the same call with one more pointer, which may be NULL */
guatiao_status guatiao_json_parse_ex    (guatiao_str text, uint32_t how, const guatiao_alloc *alloc,
                                         guatiao_value *out, guatiao_parse_error *out_error);
guatiao_status guatiao_toml_parse_ex    (/* as _json_parse_ex */);   // GUATIAO_SERDE_TOML
guatiao_status guatiao_yaml_parse_ex    (/* as _json_parse_ex */);   // GUATIAO_SERDE_YAML
typedef struct guatiao_parse_error {
  uint32_t format;          /* GUATIAO_FORMAT_JSON 0, _TOML 1, _YAML 2 */
  uint32_t line, column;    /* from 1; 0 when the parser did not say */
  guatiao_value message;    /* a string value, freed with guatiao_value_free; ABSENT if none */
} guatiao_parse_error;

#define GUATIAO_BYTES_DATA_URI 0          /* the `how` argument's LOW BYTE */
#define GUATIAO_BYTES_BASE64   1
#define GUATIAO_BYTES_ARRAY    2
#define GUATIAO_BYTES_REFUSE   3
#define GUATIAO_READ_DATA_URIS (1 << 8)   /* and the bits above it, flags */
#define GUATIAO_PRETTY         (1 << 9)   /* JSON only; read by _json_emit */
```

- **Emitting answers a VALUE holding the text**, freed with
  `guatiao_value_free` like any other tree. No second thing to free.
- **`out` is written ABSENT on entry**, before anything can fail, as in
  `guatiao.h` and `guatiao_intake.h`: read after an error it is
  `GUATIAO_ABSENT`, never what it held. It must not hold a tree you still
  own. `out_error` of a `_parse_ex` is written empty on entry too, and is
  filled only for a document that does not parse; its `message` is the
  parser's wording and never quotes the document.
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
- **A repeated key on read depends on the format.** JSON **replaces**, as
  `Map::set` does (`{"a":1,"a":2}` reads `a = 2`); TOML and YAML **refuse
  the document** (`Error::Syntax`).
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
- **A JSON number comes back exactly only with `exact-numbers`**, which
  turns `serde_json/arbitrary_precision` on so a number arrives as its own
  text rather than as an `f64` resolved before a visitor runs. Without it,
  an integer that fits `i64`/`u64` is exact and anything else is an `f64`:
  `1.10` reads `1.1`, a 30-digit integer reads as a float, and `1e400` is
  refused by the parser. Writing is exact either way under `text::json`.
- **A parse error is its message and a position, never the document.**
  `Error::Syntax` carries the parser's own wording and where it stopped;
  the line of the document a parser's `Display` would show (TOML, YAML),
  and a key, value, tag or anchor its message would quote (YAML), are
  dropped, because a document can hold a secret. `Debug`, every field and
  the source chain are free of document text too.
- **A TOML table is read in document order**, and **a TOML date or time**
  (offset date-time, local date-time, local date, local time) **reads as
  the text it was written as** and is written back as a string.
- **A YAML `!!binary` scalar reads as its base64 text.** Reading goes
  through `deserialize_any`, which gives the seed no tag, so bytes are out
  of reach; the text is not decoded behind your back and a payload that is
  not UTF-8 is not an error.
- **A document nests at most `MAX_DEPTH` (128) containers deep**, refused
  past that. `serde_json` caps its own recursion at 128 and `toml` and
  `serde-saphyr` at less (TOML arrays about 80, YAML about 64, flow or
  block), so those refuse first; another format need not cap at all, and
  this crate reads them all.
- **What each format reads as a number.** JSON: an `i64`/`u64` is exact and
  anything else is an `f64`, unless `exact-numbers` keeps the text; `NaN`
  and infinity do not exist in JSON, and `1e400` is refused. TOML: an
  integer is exact (a 30-digit one included), a float goes through `f64`
  (`1.10` reads `1.1`), `inf` and `nan` are refused. YAML 1.2 through
  `serde-saphyr`: `1.10` reads `1.1`, `007` reads `7.0`, an integer past
  `u64` reads as a float, `0x10` and `0o17` and `1_000` read as integers
  (`16`, `15`, `1000`), `.inf`, `.nan` and `1e400` are refused, `-0` reads
  `0`. Alias expansion is bounded by the parser's budget.
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
