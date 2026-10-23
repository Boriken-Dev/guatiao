# Configuration files

A program that reads its configuration with serde and describes it with
guatiao carries two sets of attributes on one type, one for each, and
the two drift: a field renamed for one is missed by the other. Read the
file **through guatiao** instead. The text becomes a value, the value
becomes the type through `FromValue`, and the schema and the form come
from the same derive: one set of attributes, nothing to keep in step,
and no serde derive on the type.

```toml
[dependencies]
guatiao = { version = "=0.0.0-alpha.1", features = ["derive"] }
guatiao-serde = { version = "=0.0.0-alpha.1", features = ["toml"] }
guatiao-intake = { version = "=0.0.0-alpha.1", features = ["derive"] }   # for a form
```

## The type

```rust
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

/// The certificate the daemon presents.
#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
#[map(untagged, rename_all = "lowercase")]
pub enum Tls {
    /// The machine's own identity.
    Identity,
    /// A certificate and its key, from files.
    Files { certificate: PathBuf, key: PathBuf },
}

#[derive(ToValue, FromValue, Schema, Form, Debug, PartialEq)]
#[form(section(id = "net", label = "Network"))]
pub struct Daemon {
    /// Where to listen.
    #[schema(section = "net")]
    #[form(placeholder = "0.0.0.0:7000")]
    pub listen: SocketAddr,
    /// The address to advertise, when it is not the one listened on.
    #[schema(section = "net")]
    pub advertise: Option<IpAddr>,
    pub tls: Tls,
    /// Where state is kept.
    #[map(default = PathBuf::from("/var/lib/daemon"))]
    pub state: PathBuf,
    /// Each plugin's own settings, which the plugin describes.
    #[map(default, with = guatiao_serde::adapt::object)]
    pub plugins: BTreeMap<String, toml::Table>,
}
```

Every other field type is one guatiao already reads and describes: an
address is its text with a `pattern`, a path its text, an `Option` may
be left out. `Tls` is written either as the word `identity` or as a table with
two keys; an enum whose variants differ in **kind** needs no tag, and
reads the same way in every language
([Encodings](encodings.md#variants-told-apart-by-kind)).

## Reading the file

```rust
use guatiao::schema::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::{Alloc, FromValue, Schema};
use guatiao_serde::Presentation;
use guatiao_serde::text::toml;

let value = toml::from_str(&text, Alloc::rust(), Presentation::new())?;

// The schema refuses what the type does not declare, a misspelled key
// included, naming the key; reading alone would skip it.
let schema = Daemon::schema()?;
validate_map(SchemaRef::new(&schema).ok_or("not a schema")?, &value)?;

let daemon = Daemon::from_value(&value)?;
```

A refusal names where it happened, `tls`, `listen` or `peers[0]`, and
never quotes the value: a configuration can hold a secret. A TOML date
or time reads as the text it was written as.

## Describing it

```rust
use guatiao_intake::{FormRef, Screen, check};
use guatiao_serde::text::json;

let schema = Daemon::schema()?;
let form = Daemon::form(Alloc::rust())?;
check(
    SchemaRef::new(&schema).ok_or("not a schema")?,
    FormRef::new(&form).ok_or("not a form")?,
)?;

// The schema is a JSON Schema: an editor that reads one completes and
// checks the file as it is written.
let document = json::to_string_pretty(&schema, Presentation::new())?;
```

## From serde's attributes

| serde | guatiao |
| --- | --- |
| `#[serde(rename = "..")]`, `rename_all` | `#[map(rename = "..")]`, `#[map(rename_all = "..")]` |
| `#[serde(default)]`, `default = ".."` | `#[map(default)]`, `#[map(default = <expr>)]` |
| `#[serde(skip)]`, `skip_serializing_if` | `#[map(skip)]`, `#[map(skip_serializing_if = path)]` |
| `#[serde(flatten)]`, `transparent`, `deny_unknown_fields` | the same names under `#[map(..)]` |
| `#[serde(tag = "..")]` | `#[map(tag = "..")]` |
| `#[serde(untagged)]` | `#[map(untagged)]`, when no two variants are of one kind |
| `serde_with::DisplayFromStr` | `#[map(as_text)]`: `Display` out, `FromStr` in, a string in the schema |
| `#[serde(with = "..")]` | `#[map(with = path)]`, below |

`#[map(as_text)]` suits a type with a text form and no value traits of
its own; `#[schema(format = "..")]` or `#[schema(pattern = "..")]` beside
it say what the text holds.

## A shape of your own: `with`

`with` names a module that writes, reads and describes the field, for a
shape nothing above gives: here a timeout written as whole milliseconds.
Its `kind` is the field's schema, so a field described by hand needs
nothing more.

```rust
mod millis {
    use std::time::Duration;

    use guatiao::schema::KindBuilder;
    use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError};

    pub fn to_value_in(timeout: &Duration, alloc: Alloc) -> Result<Value, ValueError> {
        let ms = u64::try_from(timeout.as_millis()).map_err(|_| ValueError::OutOfRange)?;
        ms.to_value_in(alloc)
    }

    pub fn from_value(value: &Value) -> Result<Duration, MapError> {
        u64::from_value(value).map(Duration::from_millis)
    }

    pub fn kind(alloc: Alloc) -> KindBuilder {
        u64::kind(alloc).description("milliseconds")
    }
}

#[derive(ToValue, FromValue, Schema)]
struct Retry {
    #[map(with = millis)]
    timeout: std::time::Duration,
}
```

## A type from serde

`plugins` above holds `toml::Table`s, a type guatiao has no impls for.
Any type serde reads and writes can be a field: `guatiao-serde`
converts it through serde.

- **A bare field** names a module: `#[map(with =
  guatiao_serde::adapt::object)]` describes it as an open object,
  `adapt::any` as any value. A `BTreeMap` of tables is one field, as
  `plugins` is.
- **A wrapper**, `guatiao_serde::Serde<T>`, describes it as any value;
  `Serde<T, guatiao::Map>` as an open object, and `Serde<T, S>` as any
  type `S` of yours that implements `Schema`. The value is `.0`.

```rust
use guatiao::{FromValue, Map, Schema, ToValue};
use guatiao_serde::Serde;

#[derive(ToValue, FromValue, Schema)]
pub struct Plugin {
    #[map(with = guatiao_serde::adapt::any)]
    pub extra: toml::Value,
    pub settings: Serde<toml::Table, Map>,
}
```

A value crosses in serde's data model, a variant externally tagged, and
a TOML date or time is the text it was written as. Writing keeps a
number's spelling when serde hands over its text (`serde_json` with its
`arbitrary_precision`, which `guatiao-serde`'s `exact-numbers` turns
on). Reading hands serde an integer of up to 128 bits or the nearest
`f64`, so `1.10` reads as `1.1`, and a wider integer is refused. A
module of your own reuses the two conversions and gives its own schema:

```rust
mod plugin_settings {
    pub use guatiao_serde::adapt::{from_value, to_value_in};

    pub fn kind(alloc: guatiao::Alloc) -> guatiao::schema::KindBuilder {
        guatiao::schema::KindBuilder::any_in(alloc).description("read by the plugin")
    }
}
```
