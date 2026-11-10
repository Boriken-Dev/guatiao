# `guatiao-builder` — API header

guatiao's side of [coqui-builder](https://github.com/Boriken-Dev/coqui-builder): what
a guatiao host does with a type that derives coqui-builder's `Describe`, a struct built
field by field. Every item here is a function or a small type, because both sides' types are
foreign to this crate.

| Dependency | Why |
| --- | --- |
| `guatiao` | the value, the schema builders and the traits `Described` implements |
| `guatiao-intake` | `unflatten`, for a form's flat store |
| `coqui-builder` (git, until it is released) | `Description`, `Source`, `Inspect`, `Partial` |

`publish = false` until coqui-builder is on crates.io. `#![forbid(unsafe_code)]`, no `static`,
no process-global state. A coqui-builder type that also derives guatiao's macros is only a
test fixture; a described type reaches guatiao through `Described<T>`.

## The text of a leaf (`guatiao_builder::text`)

coqui-builder holds every setting as one text read by the leaf's own parser. These two
functions are the whole mapping to a guatiao value; a value's own display text is never used.

```rust
pub fn from_value(value: &Value, ty: &str) -> Result<Option<String>, SourceError>;
pub fn to_value_in(ty: &str, text: &str, alloc: Alloc) -> Result<Value, ValueError>;
```

| a value | its text |
| --- | --- |
| string | as it is |
| boolean | `true` or `false` |
| a number, for an integer type or a `Duration` | its plain decimal expansion, exactly (`1e3` is `1000`, `2.0` is `2`, `-0` is `0`; `u128` and `i128` survive) |
| a number, for any other type | as written (`1.10` stays `1.10`) |
| null | `None`: the key is not given |
| list, map, anything else | `SourceError`: one text was expected |

And a text becomes a value by the leaf's type name: `bool` a boolean when the text is `true` or
`false`; `u8` to `u128`, `usize`, `i8` to `i128`, `isize`, `f32` and `f64` a number when the
text is one; every other type (`Duration`, `Path`, the addresses, a user type, an untagged
enum's text side) and a numeric type whose text is not a number (the text that writes a
`None`) a string as written.

## Schema

```rust
pub fn kind_in(description: &Description, alloc: Alloc) -> KindBuilder;
pub fn schema_in(description: &Description, alloc: Alloc) -> Result<Value, ValueError>;
pub fn schema(description: &Description) -> Result<Value, ValueError>;
pub const DURATION_PATTERN: &str;   // ^[0-9]+(\.[0-9]+)?(ns|us|ms|s|m|h)?$
```

One to one with the description:

| description | schema |
| --- | --- |
| a leaf of a builtin type (`bool`, the integers, `f32`, `f64`, `String`, `char`, `Path`, the `core::net` addresses) | what guatiao's `Schema` gives that Rust type |
| a `Duration` | `anyOf` [a number from 0, a string matching `DURATION_PATTERN`] |
| any other leaf | a string; with choices, an `enum` and `x-enum-labels`; when the leaf also accepts a name outside its choices (`other`), a string offering them as `examples` |
| list, open map | an array, an object whose values are the item's schema |
| group | a sealed object: `properties`, `required`, `additionalProperties: false` (so `deny_unknown_fields` only changes what filling does with an unknown key) |
| variant | an object with `x-variant-tag` and a `oneOf` arm per name |
| untagged enum | `anyOf` its text side (a string: no boolean or number mapping) and its structured arm |
| field with a written `None` | `anyOf` the one text and the field's schema; `default` is the inner value, or the text |
| optional field | the field's kind accepting null as well |

A field is `required` as the description says, and its `default` is the default's text through
`text::to_value_in` (a list's default is its items). Its notes become `title`, `description`,
`x-sensitive` with `writeOnly`, `readOnly`, `deprecated`, `examples`, `minLength`,
`maxLength`, `minItems`, `maxItems`, `pattern`, `format`, `multipleOf` and `x-section`,
`x-order`, `x-advanced`. A type's own doc comment is the description of every field of that
type. A secret by itself (a `Secret`, an item of a list of them) marks its kind.

`schema_in` writes the document as `Schema::schema_in` does for a derived type: the dialect,
`type: "object"`, the kind's keys, `properties`.

## A value as a source

```rust
pub struct ValueSource<'a>;   // Debug, Clone, Copy
impl<'a> ValueSource<'a> { pub fn new(root: &'a Value) -> ValueSource<'a>; }
impl Source for ValueSource<'_> { .. }
pub fn partial<T: Describe>(value: &Value) -> Result<Partial<T>, FillError>;
pub fn from_value<T: Describe>(value: &Value) -> Result<T, FromError>;
pub fn from_value_over<T: Describe>(base: T, value: &Value) -> Result<T, FromError>;
```

`ValueSource` answers a leaf's text by the table above, a list's length and a map's keys, and
the keys of the value the description lacks. A map's key is a segment; a list position is a
segment too, and an open map reads a position as the key of its decimal text. A list or a map
where a leaf is expected is a `SourceError`, which an untagged enum with a structured arm
tolerates (a table there is its structured arm). Null is a key that is not given.

`partial` is `Partial::fill_from(ValueSource)` on an empty partial: unknown keys are refused
under a group with `deny_unknown_fields` and dropped otherwise. `from_value` builds it;
`from_value_over` lays the value over `base` (`Partial::from(base)`) and builds.

```rust
#[non_exhaustive] pub enum FromError { Fill(FillError), Missing(Missing) }
```

`FromError` is `Display` and `Error`, from either of its parts. `Missing` names every key the
value lacked.

A duration reads a number of seconds (`30`, `0.25`) or its text (`30s`, `250ms`, `1.5h`); a
`[secs, nanos]` pair, guatiao's own encoding of `Duration`, is refused.

## A value as what is inspected

```rust
pub struct ValueView<'a>;   // Debug, Clone, Copy
impl<'a> ValueView<'a> { pub fn new(description: &'a Description, value: &'a Value) -> ValueView<'a>; }
impl Inspect for ValueView<'_> { .. }
```

A leaf is a `Datum` of the leaf's type name and the value's text (marked sensitive where the
description says); a list answers its length and a map its keys. `differences`, a `Report` and
`Profile::judge` take it as they take a built value.

## A described value as a value

```rust
pub fn to_value_in<T: Describe + Inspect>(value: &T, alloc: Alloc) -> Result<Value, ValueError>;
pub fn to_value<T: Describe + Inspect>(value: &T) -> Result<Value, ValueError>;
```

The description walked through `Inspect`, each leaf written by the table above, a secret's text
included (it is the value). A key that holds nothing is left out; the text that writes a `None`
is the value of its key. For a type that derives both libraries' macros, the value is the one
guatiao's own derive writes (a test holds it), except where the two encodings differ by design:
a duration, and an untagged enum's text side.

## A form

```rust
pub fn form_source(description: &Description, store: &BTreeMap<String, String>, alloc: Alloc)
    -> Result<Value, ValueError>;
```

`unflatten` per top-level field of `schema_in(description)`. Every scalar is the stored text,
which the leaf's parser then reads; a duration is one stored text. An untagged enum's
structured arm is a whole value in a form, not keys below the enum, so a store holds its text
side only. The description must be a group.

## `Described<T>` and `with_module!`

```rust
pub struct Described<T>(pub T);   // Debug, Clone, PartialEq, Eq, Default
impl<T: Describe + Inspect> ToValue for Described<T> { .. }
impl<T: Describe> FromValue for Described<T> { .. }     // KINDS left at any kind
impl<T: Describe> Schema for Described<T> { .. }
macro_rules! with_module { ($vis:vis $name:ident, $ty:ty) => { .. } }
```

`Described<T>` writes what `to_value` writes, reads what `from_value` reads (a refusal is a
`MapError` under the key it names and never quotes the input; missing keys are `missing`), and
describes itself with `kind_in`. It cannot be an arm of an enum told apart by kind. A secret
is `Described<Secret<T>>`.

`with_module!(pub name, Type)` writes a module for `#[map(with = name)]`: `to_value_in`,
`from_value` and `kind` for a field of a described type inside a guatiao-derived struct. A
`with` module's `kind` takes no type parameter, so one generic module cannot exist. Call it at
module level.

## Errors

`FromError`, `SourceError`, `FillError` and guatiao's `ValueError` and `MapError`. Nothing
here quotes a setting's value in an error.

## Gotchas

- A group's schema is always sealed; filling tolerates what the schema refuses unless the
  group says `deny_unknown_fields`.
- A changed variant arm is one difference at the tag (`auth.kind`) here and at the field
  (`auth`) in `schema::differences`.
- A boolean and a number written for an untagged enum's text side arrive as their text.
- A value is checked for shape before it fills: a table where the description has keys, a
  list where it has items, and an untagged enum takes a value or a table. A place of the wrong
  shape is refused at its key; a value with no place in the description is dropped (or refused
  under `deny_unknown_fields`).
- Null is a key that is not given: the default applies, and `from_value_over` keeps the base's
  value. guatiao's derive reads a null `Option` as `None`.
- `schema_in` refuses a description that is not a group, a variant or an open map.
- A list position that writes nothing makes `to_value` fail, rather than shift the rest.
- The leaf's type name picks its builtin schema, so an `as_text` field of a builtin type
  (`as_text` on a `u32`) is described by the type where guatiao's derive writes a string, and a
  user leaf named like a builtin is taken for it. coqui-builder exposes no flag for a native
  parser or for `other`, so the closed or open choice of a leaf is found by trying a text no
  choice can be.
- `#[schema(default = ..)]` is optional in coqui-builder and only an annotation in guatiao's
  derive (the field stays required there).
- A leaf with choices is open (an `other` variant) when it accepts a text outside them; the
  schema then offers the choices as `examples` instead of an `enum`.
