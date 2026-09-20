// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a schema as a value.
//!
//! A schema is a map written with the keys in [`super::vocab`] — JSON
//! Schema's — so it can be built with `map_set` and `list_push` and nothing
//! else. These builders are the typed way to do that: they read as a table
//! of declarations rather than as a wall of string keys, and they cannot
//! misspell one.
//!
//! # A field's name and its requiredness are collected, not written
//!
//! JSON Schema puts a field's name in the [`PROPERTIES`](super::vocab::PROPERTIES)
//! map key and its requiredness in the owner's
//! [`REQUIRED`](super::vocab::REQUIRED) list — two places, neither of them
//! inside the field. So a builder **holds its fields** and writes both at
//! `finish` rather than appending as it goes. That is invisible from the
//! call site, which still reads `FieldBuilder::new(..).required()`.
//!
//! # A schema does not know about forms
//!
//! **There is no way here to DECLARE a section**, and that is deliberate:
//! a section exists only to group controls on a screen, so declaring one
//! is a form's business and not a schema's. [`FormBuilder::section`] says
//! which section a field belongs to, because that is a hint carried
//! alongside the field; what a section is CALLED belongs to whatever
//! draws it.
//!
//! [`FormBuilder::section`]: super::FormBuilder::section
//!
//! # A schema and a form are different questions
//!
//! What a value **is** — its kind, its bounds, whether it is required —
//! is substance, and lives here. How it is **shown** — a title, some prose,
//! which section it sits in, whether it hides behind a disclosure, whether
//! it is a secret — is presentation, and lives in [`super::form`].
//!
//! # Building names no allocator
//!
//! `SchemaBuilder::new()`, `KindBuilder::string()` and the rest use the
//! crate's own allocator, exactly as `Map::new()` and `Value::string()`
//! do — so a schema written by hand reads as a declaration rather than as
//! the same argument threaded through every line.
//!
//! The `_in` forms name one, and that is what a schema built into a
//! host's arena uses. **Use them throughout when you use them at all**: a
//! sub-builder left on the plain form allocates through the crate's
//! allocator, and the tree then holds some of both. That is sound — every
//! container carries the allocator that made it, which is what lets a
//! host free a library's tree — but it is not what somebody building into
//! an arena meant.
//!
//! # Errors are collected, not returned per call
//!
//! The allocator belongs to the caller and can fail, so every step could
//! in principle fail. Returning a `Result` from each would put a `?` on
//! every line of what is meant to read as a declaration. Instead a builder
//! holds its first error and hands it back from [`SchemaBuilder::finish`],
//! which is the one place a caller has to look.
//!
//! Nothing is lost by deferring: a builder that has failed does nothing
//! further, so the error a caller sees is the first one, which is the one
//! that explains the rest.

#![forbid(unsafe_code)]

use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::TryAsMut;
use crate::value::error::ValueError;
use crate::value::types::{List, Map, Number, Text, Value};

/// Builds a schema: the root document, with its dialect declared.
pub struct SchemaBuilder {
    // Reached by `super::form`, which is the other half of this builder
    // rather than a stranger: the presentation setters live there so the
    // schema half and the form half can be read apart.
    pub(super) alloc: Alloc,
    pub(super) state: Result<Value, ValueError>,
    fields: Vec<FieldBuilder>,
}

/// Builds one field: its name, what it accepts, and whether it is
/// required.
///
/// The name and the requiredness are the owner's to write; everything else
/// goes onto the field's own subschema, which is what `state` holds.
pub struct FieldBuilder {
    pub(super) alloc: Alloc,
    pub(super) state: Result<Value, ValueError>,
    key: String,
    required: bool,
}

/// Builds one kind: a subschema with no name of its own.
pub struct KindBuilder {
    state: Result<Value, ValueError>,
}

/// Builds one arm of a tagged kind.
///
/// Holds its discriminant until the variant assembles it, because an arm
/// does not know which key the discriminant is stored under — that is the
/// variant's declaration, not the arm's.
pub struct ArmBuilder {
    pub(super) alloc: Alloc,
    pub(super) state: Result<Value, ValueError>,
    discriminant: Result<Value, ValueError>,
    fields: Vec<FieldBuilder>,
}

impl std::fmt::Debug for SchemaBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "SchemaBuilder", &self.state)
    }
}

impl std::fmt::Debug for FieldBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "FieldBuilder", &self.state)
    }
}

impl std::fmt::Debug for KindBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "KindBuilder", &self.state)
    }
}

impl std::fmt::Debug for ArmBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "ArmBuilder", &self.state)
    }
}

/// Whether a builder has failed, and with what.
///
/// Deliberately says nothing about what has been built so far: a
/// half-built tree is the worst possible thing to walk, and a builder is
/// printed precisely when something has already gone wrong.
fn debug_state(
    f: &mut std::fmt::Formatter<'_>,
    name: &str,
    state: &Result<Value, ValueError>,
) -> std::fmt::Result {
    match state {
        Ok(_) => write!(f, "{name} {{ ok }}"),
        Err(e) => write!(f, "{name} {{ failed: {e} }}"),
    }
}

/// Sets `key` to `value`, keeping the first error rather than the last.
pub(super) fn put(
    state: &mut Result<Value, ValueError>,
    key: &str,
    value: Result<Value, ValueError>,
) {
    let node = match state {
        Ok(n) => n,
        // Already failed: do nothing further, so the error a caller sees
        // is the first one, which is the one that explains the rest.
        Err(_) => return,
    };
    match value.and_then(|v| {
        TryAsMut::<Map>::try_as_mut(node)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set(key, v))
    }) {
        Ok(()) => {}
        Err(e) => *state = Err(e),
    }
}

/// Appends `value` to the list under `key`, creating the list if needed.
fn push(state: &mut Result<Value, ValueError>, key: &str, value: Result<Value, ValueError>) {
    let node = match state {
        Ok(n) => n,
        Err(_) => return,
    };
    match value.and_then(|v| {
        TryAsMut::<Map>::try_as_mut(node)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.push_into(key, v))
    }) {
        Ok(()) => {}
        Err(e) => *state = Err(e),
    }
}

/// Appends one name to a list being built.
fn push_name(list: &mut Result<Value, ValueError>, alloc: Alloc, name: &str) {
    let node = match list {
        Ok(n) => n,
        Err(_) => return,
    };
    match Text::new_in(alloc, name).map(Value::from).and_then(|v| {
        TryAsMut::<List>::try_as_mut(node)
            .ok_or(ValueError::WrongKind)
            .and_then(|l| l.push(v))
    }) {
        Ok(()) => {}
        Err(e) => *list = Err(e),
    }
}

/// Writes `properties` and `required` onto an object schema.
///
/// The one place that knows a field's name and its requiredness live
/// outside the field. `tag` is the discriminant an arm carries: it goes in
/// first, so the arm reads as the thing it selects, and it is always
/// required — an arm without its discriminant is not that arm.
///
/// `properties` is written even when empty, so a reader can tell an object
/// that declares nothing from a kind that forgot to say.
fn seal(
    state: &mut Result<Value, ValueError>,
    alloc: Alloc,
    tag: Option<(&str, Result<Value, ValueError>)>,
    fields: Vec<FieldBuilder>,
) {
    let mut properties = Ok(Map::new_in(alloc).into());
    let mut required = Ok(List::new_in(alloc).into());
    let mut any_required = false;

    if let Some((tag, discriminant)) = tag {
        put(&mut properties, tag, discriminant);
        push_name(&mut required, alloc, tag);
        any_required = true;
    }
    for field in fields {
        put(&mut properties, &field.key, field.state);
        if field.required {
            push_name(&mut required, alloc, &field.key);
            any_required = true;
        }
    }

    put(state, vocab::PROPERTIES, properties);
    // The key set is closed, and the document says so: what `validate`
    // refuses, a general validator now refuses too.
    put(state, vocab::ADDITIONAL_PROPERTIES, Ok(Value::from(false)));
    // Absent means nothing is required, which is what JSON Schema says an
    // absent `required` means. An empty list would say the same thing in
    // more bytes.
    if any_required {
        put(state, vocab::REQUIRED, required);
    }
}

impl SchemaBuilder {
    /// An empty schema, through the crate's own allocator.
    ///
    /// [`new_in`](SchemaBuilder::new_in) names one, which is what a
    /// schema built into a host's arena needs.
    pub fn new() -> SchemaBuilder {
        SchemaBuilder::new_in(Alloc::rust())
    }

    /// An empty schema, through an allocator you name.
    ///
    /// The dialect and the type go in here rather than at `finish`, so
    /// they come first in the document — which is where a reader looks for
    /// them.
    pub fn new_in(alloc: Alloc) -> SchemaBuilder {
        let mut state = Ok(Map::new_in(alloc).into());
        put(
            &mut state,
            vocab::SCHEMA,
            Text::new_in(alloc, vocab::DIALECT).map(Value::from),
        );
        put(
            &mut state,
            vocab::TYPE,
            Text::new_in(alloc, vocab::TYPE_OBJECT).map(Value::from),
        );
        SchemaBuilder {
            alloc,
            state,
            fields: Vec::new(),
        }
    }

    /// Declares a field. Order of declaration is the order a consumer
    /// sees, because a map here is insertion-ordered by contract.
    pub fn field(mut self, field: FieldBuilder) -> SchemaBuilder {
        self.fields.push(field);
        self
    }

    /// Sets any key at all: one from [`vocab`], or an annotation nobody
    /// interprets.
    ///
    /// **The general door.** The named setters are conveniences over this
    /// one, and anything they do not cover goes through here -- a keyword
    /// this build has no method for, or a vendor's own key. A key outside
    /// the vocabulary is carried through every reader, every merge and
    /// every round trip and interpreted by nobody; `x-` prefixed, by
    /// convention.
    ///
    /// Takes a **value**, not a `Result`. Building one names no allocator
    /// and cannot fail, so there is nothing for a caller to have handled;
    /// one building into an arena writes `?` at the call site.
    pub fn option(mut self, key: &str, value: impl Into<Value>) -> SchemaBuilder {
        put(&mut self.state, key, Ok(value.into()));
        self
    }

    /// The schema, or the first error that stopped it.
    ///
    /// Where the fields become `properties` and the `required()` calls
    /// become the `required` list.
    ///
    /// **A field key containing the flat separator is refused here**, as
    /// [`ValueError::WrongKind`]: it would make a payload key
    /// (`auth.password`) ambiguous with a field key, and every reader of
    /// a dotted path downstream would then have two readings to choose
    /// between. A declaration bug, caught at declaration.
    pub fn finish(mut self) -> Result<Value, ValueError> {
        let (alloc, fields) = (self.alloc, std::mem::take(&mut self.fields));
        seal(&mut self.state, alloc, None, fields);
        let schema = self.state?;
        if let Some(read) = super::read::SchemaRef::new(&schema)
            && super::flat::check_keys(read).is_err()
        {
            return Err(ValueError::WrongKind);
        }
        Ok(schema)
    }
}

/// The same as [`SchemaBuilder::new`].
impl Default for SchemaBuilder {
    fn default() -> SchemaBuilder {
        SchemaBuilder::new()
    }
}

impl FieldBuilder {
    /// A field under `key`, accepting `kind`.
    /// One field, through the crate's own allocator.
    pub fn new(key: &str, kind: KindBuilder) -> FieldBuilder {
        FieldBuilder::new_in(Alloc::rust(), key, kind)
    }

    /// The same, through an allocator you name.
    ///
    /// **The kind's map IS the field's subschema.** There is no nesting
    /// under a `kind` key, because JSON Schema has none: a field's schema
    /// says what it accepts, and `title`, `default` and the rest sit on the
    /// same map.
    pub fn new_in(alloc: Alloc, key: &str, kind: KindBuilder) -> FieldBuilder {
        FieldBuilder {
            alloc,
            state: kind.state,
            key: key.to_string(),
            required: false,
        }
    }

    /// The default value, of whatever kind the field accepts.
    ///
    /// Setting it to null is different from not setting it: the first says
    /// the default is nothing, the second that there is no default.
    pub fn default(mut self, value: impl Into<Value>) -> FieldBuilder {
        put(&mut self.state, vocab::DEFAULT, Ok(value.into()));
        self
    }

    /// The same, for a caller holding a `Result`.
    ///
    /// **For generated code**, which converts a Rust expression through
    /// `ToValue` and has nowhere to put a failure: the builder keeps the
    /// first error and [`SchemaBuilder::finish`] reports it, which is the
    /// whole reason a builder collects rather than returns. A caller
    /// writing by hand wants [`default`](FieldBuilder::default) and an
    /// infallible constructor.
    pub fn default_checked(mut self, value: Result<Value, ValueError>) -> FieldBuilder {
        put(&mut self.state, vocab::DEFAULT, value);
        self
    }

    /// Must be given.
    ///
    /// Recorded on the builder rather than written onto the field: the
    /// name lands in the **owner's** `required` list when the owner
    /// finishes, which is where JSON Schema keeps it.
    pub fn required(mut self) -> FieldBuilder {
        self.required = true;
        self
    }

    /// Sets any key at all: one from [`vocab`], or an annotation nobody
    /// interprets. See [`SchemaBuilder::option`].
    pub fn option(mut self, key: &str, value: impl Into<Value>) -> FieldBuilder {
        put(&mut self.state, key, Ok(value.into()));
        self
    }
}

impl KindBuilder {
    fn typed(alloc: Alloc, ty: &str) -> KindBuilder {
        let mut state = Ok(Map::new_in(alloc).into());
        put(
            &mut state,
            vocab::TYPE,
            Text::new_in(alloc, ty).map(Value::from),
        );
        KindBuilder { state }
    }

    /// True or false.
    /// Built through the crate's own allocator. `bool_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn bool() -> KindBuilder {
        KindBuilder::bool_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn bool_in(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_BOOLEAN)
    }

    /// Free text.
    /// Built through the crate's own allocator. `string_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn string() -> KindBuilder {
        KindBuilder::string_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn string_in(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_STRING)
    }

    /// A whole number, unbounded.
    /// Built through the crate's own allocator. `int_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn int() -> KindBuilder {
        KindBuilder::int_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn int_in(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_INTEGER)
    }

    /// A whole number between `min` and `max`, both inclusive.
    /// Built through the crate's own allocator. `int_range_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn int_range(min: i64, max: i64) -> KindBuilder {
        KindBuilder::int_range_in(Alloc::rust(), min, max)
    }

    /// The same, through an allocator you name.
    pub fn int_range_in(alloc: Alloc, min: i64, max: i64) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_INTEGER);
        put(
            &mut k.state,
            vocab::MINIMUM,
            Number::new_in(alloc, &min.to_string()).map(Value::from),
        );
        put(
            &mut k.state,
            vocab::MAXIMUM,
            Number::new_in(alloc, &max.to_string()).map(Value::from),
        );
        k
    }

    /// A whole number, bounded on either side or neither.
    ///
    /// The form a generated schema reaches for: a Rust integer width has
    /// bounds that fit an `i64` sometimes and not always, and a bound that
    /// does not fit must be left off rather than clamped -- a clamped
    /// bound enforces a limit nobody declared.
    /// Built through the crate's own allocator. `int_bounds_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn int_bounds(min: Option<i64>, max: Option<i64>) -> KindBuilder {
        KindBuilder::int_bounds_in(Alloc::rust(), min, max)
    }

    /// The same, through an allocator you name.
    pub fn int_bounds_in(alloc: Alloc, min: Option<i64>, max: Option<i64>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_INTEGER);
        if let Some(min) = min {
            put(
                &mut k.state,
                vocab::MINIMUM,
                Number::new_in(alloc, &min.to_string()).map(Value::from),
            );
        }
        if let Some(max) = max {
            put(
                &mut k.state,
                vocab::MAXIMUM,
                Number::new_in(alloc, &max.to_string()).map(Value::from),
            );
        }
        k
    }

    /// A real number, unbounded.
    /// Built through the crate's own allocator. `float_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn float() -> KindBuilder {
        KindBuilder::float_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn float_in(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_NUMBER)
    }

    /// A real number, bounded on either side or neither.
    /// Built through the crate's own allocator. `float_bounds_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn float_bounds(min: Option<f64>, max: Option<f64>) -> KindBuilder {
        KindBuilder::float_bounds_in(Alloc::rust(), min, max)
    }

    /// The same, through an allocator you name.
    pub fn float_bounds_in(alloc: Alloc, min: Option<f64>, max: Option<f64>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_NUMBER);
        if let Some(min) = min {
            put(
                &mut k.state,
                vocab::MINIMUM,
                Number::float_in(alloc, min).map(Value::from),
            );
        }
        if let Some(max) = max {
            put(
                &mut k.state,
                vocab::MAXIMUM,
                Number::float_in(alloc, max).map(Value::from),
            );
        }
        k
    }

    /// Opaque bytes.
    ///
    /// `type: "bytes"`, which is **ours** and not one of JSON Schema's
    /// seven. See [`vocab::TYPE_BYTES`] for what that costs.
    /// Built through the crate's own allocator. `bytes_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn bytes() -> KindBuilder {
        KindBuilder::bytes_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn bytes_in(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_BYTES)
    }

    /// A sequence, every element of `items`.
    /// Built through the crate's own allocator. `list_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn list(items: KindBuilder) -> KindBuilder {
        KindBuilder::list_in(Alloc::rust(), items)
    }

    /// The same, through an allocator you name.
    pub fn list_in(alloc: Alloc, items: KindBuilder) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_ARRAY);
        put(&mut k.state, vocab::ITEMS, items.state);
        k
    }

    /// A nested object with its own fields.
    ///
    /// An object with no fields is ordinary and complete -- an object
    /// nothing further is declared about -- exactly as an arm with no
    /// fields is.
    /// Built through the crate's own allocator. `map_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn map(fields: Vec<FieldBuilder>) -> KindBuilder {
        KindBuilder::map_in(Alloc::rust(), fields)
    }

    /// The same, through an allocator you name.
    pub fn map_in(alloc: Alloc, fields: Vec<FieldBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_OBJECT);
        seal(&mut k.state, alloc, None, fields);
        k
    }

    /// Exactly one of a fixed set of alternatives.
    ///
    /// A `string` with an `enum`, and the labels in a **map** keyed by the
    /// value — never a second list, which would let the two drift in length
    /// or order and show a person one alternative while storing another.
    /// Built through the crate's own allocator. `enumeration_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn enumeration(choices: &[(&str, &str)]) -> KindBuilder {
        KindBuilder::enumeration_in(Alloc::rust(), choices)
    }

    /// The same, through an allocator you name.
    pub fn enumeration_in(alloc: Alloc, choices: &[(&str, &str)]) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_STRING);
        let mut values = Ok(List::new_in(alloc).into());
        let mut labels = Ok(Map::new_in(alloc).into());
        let mut any_label = false;
        for (value, label) in choices {
            push_name(&mut values, alloc, value);
            // A label that says nothing the value does not is left off: a
            // reader shows the value when there is no label, so writing it
            // would only put `"off": ""` or `"off": "off"` in the document.
            if !label.is_empty() && label != value {
                put(
                    &mut labels,
                    value,
                    Text::new_in(alloc, label).map(Value::from),
                );
                any_label = true;
            }
        }
        put(&mut k.state, vocab::ENUM, values);
        if any_label {
            put(&mut k.state, vocab::X_ENUM_LABELS, labels);
        }
        k
    }

    /// Any one of several kinds. Untagged.
    ///
    /// `anyOf`: the question is whether the value is acceptable at all, and
    /// which arm took it is explicitly not the point.
    /// Built through the crate's own allocator. `union_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn union(arms: Vec<KindBuilder>) -> KindBuilder {
        KindBuilder::union_in(Alloc::rust(), arms)
    }

    /// The same, through an allocator you name.
    pub fn union_in(alloc: Alloc, arms: Vec<KindBuilder>) -> KindBuilder {
        // No `type` of its own: `anyOf` alone is what a union is, and a
        // union of an integer and a string has no single type to name.
        let mut k = KindBuilder {
            state: Ok(Map::new_in(alloc).into()),
        };
        put(&mut k.state, vocab::ANY_OF, Ok(List::new_in(alloc).into()));
        for arm in arms {
            push(&mut k.state, vocab::ANY_OF, arm.state);
        }
        k
    }

    /// One of several alternatives, each with its own fields. Tagged: the
    /// discriminant is stored under `tag`.
    ///
    /// An object with `oneOf` arms, each of which pins the discriminant
    /// with a `const` and requires it. The tag itself travels under
    /// [`vocab::X_VARIANT_TAG`], because JSON Schema has no discriminator keyword
    /// and inferring one stops working the moment two properties are
    /// `const`.
    /// Built through the crate's own allocator. `variant_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn variant(tag: &str, arms: Vec<ArmBuilder>) -> KindBuilder {
        KindBuilder::variant_in(Alloc::rust(), tag, arms)
    }

    /// The same, through an allocator you name.
    pub fn variant_in(alloc: Alloc, tag: &str, arms: Vec<ArmBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_OBJECT);
        put(
            &mut k.state,
            vocab::X_VARIANT_TAG,
            Text::new_in(alloc, tag).map(Value::from),
        );
        // Written even when empty, so a variant that declares no arms is a
        // variant with no arms rather than a kind that forgot to say.
        put(&mut k.state, vocab::ONE_OF, Ok(List::new_in(alloc).into()));
        for arm in arms {
            let mut state = arm.state;
            seal(
                &mut state,
                arm.alloc,
                Some((tag, arm.discriminant)),
                arm.fields,
            );
            push(&mut k.state, vocab::ONE_OF, state);
        }
        k
    }

    /// The kind, or the first error that stopped it.
    pub fn finish(self) -> Result<Value, ValueError> {
        self.state
    }
}

impl ArmBuilder {
    /// An arm storing `value` when selected.
    ///
    /// **An arm with no fields is complete.** It is the common case, so a
    /// design making it exceptional would make the exception the thing
    /// everyone must remember.
    /// One arm, through the crate's own allocator.
    pub fn new(value: &str, label: &str) -> ArmBuilder {
        ArmBuilder::new_in(Alloc::rust(), value, label)
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc, value: &str, label: &str) -> ArmBuilder {
        let mut state = Ok(Map::new_in(alloc).into());
        // Left off when it says nothing the value does not, for the reason
        // `enumeration_in` gives: a reader shows the value when there is no
        // title.
        if !label.is_empty() && label != value {
            put(
                &mut state,
                vocab::TITLE,
                Text::new_in(alloc, label).map(Value::from),
            );
        }
        // Held rather than written: an arm does not know which key its
        // discriminant is stored under, because that is the variant's
        // declaration.
        let mut discriminant = Ok(Map::new_in(alloc).into());
        put(
            &mut discriminant,
            vocab::CONST,
            Text::new_in(alloc, value).map(Value::from),
        );
        ArmBuilder {
            alloc,
            state,
            discriminant,
            fields: Vec::new(),
        }
    }

    /// A field this arm adds when selected.
    pub fn field(mut self, field: FieldBuilder) -> ArmBuilder {
        self.fields.push(field);
        self
    }
}
