# Writing a bridge to another format

guatiao reads and writes JSON, TOML and YAML through `guatiao-serde`,
and any serde format through the same crate. A format serde does not
reach, or one a program wants to read with nothing in between, plugs
into guatiao's own data model instead: a handful of traits in
`guatiao::value::visit`, with no dependency.

The model is the eight kinds of a value, as events: absent, null, a
bool, a number as its text, a string, bytes, a list and a map with text
keys. A type that **walks** hands its events to a visitor; a type that is
**read from a walk** pulls them from a walker. Every derived type, every
standard type `ToValue` and `FromValue` cover, `Value` itself and the
wire encoding already do both, so a bridge written once reaches all of
them, and no `Value` is built on the way.

## Writing: a sink

A writer for a format is a `Sink`: one `&mut self` call per event, with
`list_begin` and `list_end` (`map_begin`, `key`, `map_end`) around a
container's elements. `Flat` turns a sink into the visitor that
`walk` takes. This one writes `dotted.path=value` lines:

```rust
use guatiao::value::visit::{Flat, Sink};
use guatiao::{ToValue, ValueError, Walk};

/// Writes any walk as `dotted.path=value` lines: a map's entries
/// under their keys, a list's elements under their indexes.
#[derive(Default)]
struct Lines {
    out: String,
    path: Vec<String>,
    // Per open container: a list's next index, or `None` for a map.
    open: Vec<Option<usize>>,
}

impl Lines {
    fn line(&mut self, text: &str) -> Result<(), ValueError> {
        self.out.push_str(&self.path.join("."));
        self.out.push('=');
        self.out.push_str(text);
        self.out.push('\n');
        self.next();
        Ok(())
    }

    /// After an element: a list's next one goes under the next index.
    fn next(&mut self) {
        if let Some(Some(index)) = self.open.last_mut() {
            *index += 1;
            let index = index.to_string();
            if let Some(last) = self.path.last_mut() {
                *last = index;
            }
        }
    }

    fn close(&mut self) -> Result<(), ValueError> {
        self.open.pop();
        self.path.pop();
        self.next();
        Ok(())
    }
}

impl Sink for Lines {
    type Error = ValueError;

    fn absent(&mut self) -> Result<(), ValueError> {
        self.next();
        Ok(())
    }
    fn null(&mut self) -> Result<(), ValueError> {
        self.line("")
    }
    fn bool(&mut self, value: bool) -> Result<(), ValueError> {
        self.line(if value { "true" } else { "false" })
    }
    fn number(&mut self, text: &str) -> Result<(), ValueError> {
        self.line(text)
    }
    fn string(&mut self, text: &str) -> Result<(), ValueError> {
        self.line(text)
    }
    fn bytes(&mut self, _bytes: &[u8]) -> Result<(), ValueError> {
        Err(ValueError::WrongKind)
    }
    fn list_begin(&mut self, _len: Option<usize>) -> Result<(), ValueError> {
        self.open.push(Some(0));
        self.path.push("0".into());
        Ok(())
    }
    fn list_end(&mut self) -> Result<(), ValueError> {
        self.close()
    }
    fn map_begin(&mut self, _len: Option<usize>) -> Result<(), ValueError> {
        self.open.push(None);
        self.path.push(String::new());
        Ok(())
    }
    fn key(&mut self, key: &str) -> Result<(), ValueError> {
        if let Some(last) = self.path.last_mut() {
            *last = key.into();
        }
        Ok(())
    }
    fn map_end(&mut self) -> Result<(), ValueError> {
        self.close()
    }
}

#[derive(ToValue)]
struct Server {
    host: String,
    port: u16,
}

#[derive(ToValue)]
struct Fleet {
    name: String,
    servers: Vec<Server>,
}

let fleet = Fleet {
    name: "edge".into(),
    servers: vec![
        Server {
            host: "10.0.0.1".into(),
            port: 5900,
        },
        Server {
            host: "10.0.0.2".into(),
            port: 5901,
        },
    ],
};
let mut lines = Lines::default();
fleet.walk(Flat(&mut lines))?;
assert_eq!(
    lines.out,
    "name=edge\n\
     servers.0.host=10.0.0.1\nservers.0.port=5900\n\
     servers.1.host=10.0.0.2\nservers.1.port=5901\n"
);
```

The same sink takes a `Value` (`value.walk(Flat(&mut lines))`) and wire
bytes (`wire::Encoded(&bytes).walk(...)`) with no change. Two rules hold
for every sink:

- **A number is its text**, in the JSON number grammar. An integer or a
  float arrives as the text its `Display` writes, which is what a value
  stores, so a format writes one number one way whichever path reached it.
- **A length is a promise when given.** `list_begin(Some(n))` says
  exactly `n` elements follow; `None` says the count is not known. A
  format that writes the count first refuses a walk that breaks its
  promise; one that does not, as above, can ignore it.

A writer that wants to recurse rather than keep a stack implements
`Visitor` instead: `list` and `map` answer a `ListVisitor` or
`MapVisitor`, and each element arrives as something that walks itself,
so a pretty printer passes its indentation down. A visitor that only
needs the events uses a sink.

## Reading: a walker

A reader for a format is a `Walker`: given the builder of whatever is
being read, it calls the one method for the value it holds. A list or a
map is handed over as a `ListWalker` or `MapWalker`, from which the
builder pulls each element, so it reads straight into its fields:

```rust
use guatiao::value::visit::{Build, MapWalker, Seed, Walker};
use guatiao::{FromValue, FromWalk, Kinds, MapError, Number};

/// `key=value` lines as one map. The format has no types, so each
/// value goes to the reader as what it accepts: a number when it takes
/// one and the text is one, else a string.
struct Lines<'a>(&'a str);

struct Entries<'a> {
    lines: core::str::Lines<'a>,
    value: &'a str,
}

struct Text<'a>(&'a str);

impl Walker for Lines<'_> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        build.map(Entries {
            lines: self.0.lines(),
            value: "",
        })
    }
}

impl MapWalker for Entries<'_> {
    fn len_hint(&self) -> Option<usize> {
        None
    }

    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        let Some(line) = self.lines.find(|line| !line.is_empty()) else {
            return Ok(None);
        };
        let (key, value) = line.split_once('=').unwrap_or((line, ""));
        self.value = value;
        Ok(Some(read(key)))
    }

    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError> {
        seed.read(Text(self.value))
    }
}

impl Walker for Text<'_> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        if build.kinds().intersects(Kinds::NUMBER) && Number::is_number(self.0) {
            build.number(self.0)
        } else {
            build.string(self.0)
        }
    }
}

#[derive(FromValue, Debug, PartialEq)]
struct Endpoint {
    host: String,
    port: u16,
    label: Option<String>,
}

let endpoint = Endpoint::from_walk(Lines("host=10.0.0.1\nport=5900\n"))?;
assert_eq!(
    endpoint,
    Endpoint {
        host: "10.0.0.1".into(),
        port: 5900,
        label: None
    }
);
```

`Endpoint::from_walk` reads it, as `Value::from_walk` would build a
value, and nothing else changes. What the reader may rely on:

- **The builder says what it accepts.** `build.kinds()` is the set of
  kinds it takes, which is how a format with no types of its own picks
  one, as `Text` does. Calling a method for a kind it does not take is
  refused with the field's key.
- **A refusal is a `MapError`**, named after the key it happened under.
  A walker whose own input is wrong (bytes that are not the format, a
  document that does not parse) keeps that error itself and answers
  `MapError::Stopped`; the function that started the walk reports the
  walker's error, not a refusal nobody made.
- **Depth is the walker's to bound.** A walker over input from outside
  refuses to open more than `guatiao::MAX_DEPTH` containers, so a reader
  that recurses once per container cannot run out of stack.

`Seed` is a reader that carries state (`PhantomData<T>` is the one for
any `FromWalk` type), and `MapWalker::skip_value` passes over a value
nobody reads without building it.

## A serde format

Every serde format is a bridge already, through `guatiao-serde`:

- `Walked::new(&value, how)` is `Serialize` for anything that walks, so
  any serde serializer writes it;
- `from_deserializer::<T, _>(de, how)` reads any `FromWalk` type from
  any serde deserializer;
- `deserialize_into(de, &mut sink, how)` hands a document's events to
  any sink, such as the wire encoder;
- `Serde<T, S>` walks and reads a field through `T`'s own serde impls.

`text::json::to_string_of` and `from_str_as` (and their TOML and YAML
twins) are these, for the three formats the crate names.

## From C, and from any language

The C surface has the same model. A `guatiao_visitor` is a table of one
function per event, as a sink is; a `guatiao_walker` is a function that
hands its own data to a visitor the library provides:

- `guatiao_value_visit` and `guatiao_wire_visit` walk a value or wire
  bytes into a C visitor; `guatiao_value_from_walker` and
  `guatiao_wire_encode_walker` build a value or write wire bytes from a
  C walker;
- `guatiao_json_visit` reads a document into a C visitor,
  `guatiao_json_emit_walker` writes one from a C walker, and
  `guatiao_json_to_wire` and `guatiao_json_from_wire` turn a document
  into wire bytes and back (TOML and YAML alike).

A callback's status stops the walk and is returned, and a slot left null
refuses its event. The header documents each function.

A language whose calls into C are costly crosses once with the wire
bytes instead, the **tape**: the document arrives as wire bytes, and the
binding reads them into its own objects in its own language. That is
what the Python and Dart bindings' `parse` and `render` do, measured
faster there than a callback per event (`benchmarks/README.md`). The
wire encoding is written out in [Encodings](encodings.md#values-on-the-wire).
