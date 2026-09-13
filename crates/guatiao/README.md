# guatiao

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs with pointer-and-length strings,
a schema through which a provider says what it needs to be initialised, and a
library envelope through which one library registers any number of providers.
Nothing is opaque: a C, C++, Dart or Rust consumer reads a tree, a schema or
a descriptor with no call into any library.

*Guatiao* is the Taíno pact in which two people exchange names and become
kin. An ABI is the same agreement between two sides of a boundary.

## Features

- **Values as C structs** — null, bool, number (kept as text), string, bytes,
  list and map; one node type; insertion-ordered.
- **Views and owned containers** — `{ptr, len}` views for every parameter and
  read; `{ptr, len, cap}` owned containers that grow through the allocator
  that travels with the tree.
- **One allocator, carried** — every owned container records the allocator
  that made it, so a tree built in a library frees correctly in the host.
- **Schema as the init contract** — options, kinds, defaults, sections and
  validity, written as an ordinary value with a documented key vocabulary.
  There is no serialisation here: how a schema is written down is the
  consumer's decision.
- **Rust ergonomics on top** — `#[derive(ToValue, FromValue, Schema)]`,
  `try_into` for reading, and a generated C header for everyone else.

## Quick start

```rust
use guatiao::{Map, ReadValue};

let mut options = Map::new();
options.set("compression", 6)?;

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;
map.set("tls", true)?;
map.set("options", options)?;

// Get the value, then convert it: no per-kind getters on a map, and no
// per-kind readers on a value. `ok_or_missing` is `Option::ok_or` with
// the one error it could be already filled in, and the rest is `TryInto`.
let host: &str = map.get("host").ok_or_missing()?.try_into()?;
let port: u16 = map.get("port").ok_or_missing()?.try_into()?;
let compression: i64 = map
    .get("options")
    .get("compression")
    .ok_or_missing()?
    .try_into()?;
assert_eq!((host, port, compression), ("10.0.0.1", 5900, 6));

// A key that is absent and a value of another kind each say which it was.
assert!(map.get("missing").ok_or_missing().is_err());
assert!(TryInto::<i64>::try_into(map.get("host").ok_or_missing()?).is_err());
```

A struct can do all of that for you:

```rust
use guatiao::{Alloc, FromValue, Schema, ToValue};

#[derive(ToValue, FromValue, Schema, PartialEq, Debug)]
struct Connection {
    /// Where to connect.
    host: String,
    port: u16,
    #[schema(sensitive)]
    password: Option<String>,
}
```

`Connection::schema(alloc)` is what a consumer reads to know it must ask for
a host, may leave out a password, and should not log it. `to_value` and
`from_value` move the struct across the boundary, and the three derives read
one declaration, so the schema cannot describe a value the type refuses.

## Feature flags

| Flag | Adds | Needed for |
| --- | --- | --- |
| `derive` | `guatiao-derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `c-exports` | nothing | exporting the C mutation functions |

A default build pulls in nothing.

## Licence

**Mozilla Public License 2.0.** Per-file copyleft: using this crate imposes
nothing on your code, whether you link it statically or dynamically and
whatever licence your own work carries. Modifying this crate's own files is
what carries the obligation to publish those files under the MPL 2.0.

A combined work may be distributed under terms of your choosing (MPL 2.0
section 3.3), and the licence stays GPL-compatible.
