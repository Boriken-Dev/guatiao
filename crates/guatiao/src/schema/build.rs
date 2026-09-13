// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a schema as a value.
//!
//! A schema is a map written with the keys in [`super::vocab`], so it can
//! be built with `map_set` and `list_push` and nothing else. These
//! builders are the typed way to do that: they read as a table of
//! declarations rather than as a wall of string keys, and they cannot
//! misspell one.
//!
//! # A schema and a form are different questions
//!
//! What a value **is** — its kind, its bounds, whether it is required —
//! is substance, and lives on the builders themselves. How it is
//! **shown** — a label, some help — is presentation, and lives on
//! [`FormBuilder`], which all three builders implement.
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
use crate::value::mutate::ValueError;
use crate::value::types::Value;

/// Builds a schema.
pub struct SchemaBuilder {
    alloc: Alloc,
    state: Result<Value, ValueError>,
}

/// Builds one option.
pub struct FieldBuilder {
    alloc: Alloc,
    state: Result<Value, ValueError>,
}

/// Builds one kind.
pub struct KindBuilder {
    state: Result<Value, ValueError>,
}

/// Builds one arm of a tagged kind.
pub struct ArmBuilder {
    alloc: Alloc,
    state: Result<Value, ValueError>,
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
fn put(state: &mut Result<Value, ValueError>, key: &str, value: Result<Value, ValueError>) {
    let node = match state {
        Ok(n) => n,
        // Already failed: do nothing further, so the error a caller sees
        // is the first one, which is the one that explains the rest.
        Err(_) => return,
    };
    match value.and_then(|v| node.set(key, v)) {
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
    match value.and_then(|v| node.push_into(key, v)) {
        Ok(()) => {}
        Err(e) => *state = Err(e),
    }
}

/// The presentation half: what a person is SHOWN.
///
/// A schema says what a value **is** — its kind, its bounds, whether it
/// is required. A form says how to **show** it — a label, some help, an
/// order, a section. The two have always lived in one builder here, and
/// the presentation half repeats across three of them, which is what a
/// trait is for.
///
/// It is also the seam a form generator wants: code that decorates a
/// declaration can be generic over this and never know whether it is
/// labelling a whole schema, one field, or one arm of a variant.
///
/// ```
/// use guatiao::schema::{FieldBuilder, FormBuilder, KindBuilder, SchemaBuilder};
///
/// let schema = SchemaBuilder::new()
///     .label("Connection")
///     .help("Where to connect, and how.")
///     .option(
///         FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
///             .label("Port")
///             .required(),
///     )
///     .finish()
///     .expect("a schema this small does not exhaust an allocator");
/// ```
///
/// **Presentation is optional and substance is not.** Every key here may
/// be missing and the schema is still correct and still usable; a
/// consumer with no user interface ignores all of them. Never make a
/// validation or type decision depend on one.
pub trait FormBuilder: Sized {
    /// Sets one presentation key to some text.
    ///
    /// The only thing an implementer writes; everything else here is
    /// provided over it.
    fn presentation(&mut self, key: &str, text: &str);

    /// A short human label.
    #[must_use]
    fn label(mut self, label: &str) -> Self {
        self.presentation(vocab::LABEL, label);
        self
    }

    /// Longer human help: a sentence under the control, or a tooltip.
    #[must_use]
    fn help(mut self, help: &str) -> Self {
        self.presentation(vocab::HELP, help);
        self
    }
}

impl FormBuilder for SchemaBuilder {
    fn presentation(&mut self, key: &str, text: &str) {
        put(&mut self.state, key, Value::string_in(self.alloc, text));
    }
}

impl FormBuilder for FieldBuilder {
    fn presentation(&mut self, key: &str, text: &str) {
        put(&mut self.state, key, Value::string_in(self.alloc, text));
    }
}

impl FormBuilder for ArmBuilder {
    fn presentation(&mut self, key: &str, text: &str) {
        put(&mut self.state, key, Value::string_in(self.alloc, text));
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
    pub fn new_in(alloc: Alloc) -> SchemaBuilder {
        SchemaBuilder {
            alloc,
            state: Ok(Value::map_in(alloc)),
        }
    }

    /// Declares an option. Order of declaration is the order a consumer
    /// sees.
    pub fn option(mut self, option: FieldBuilder) -> SchemaBuilder {
        push(&mut self.state, vocab::OPTIONS, option.state);
        self
    }

    /// Declares a section a consumer may group options into.
    pub fn section(mut self, id: &str, label: &str, help: &str) -> SchemaBuilder {
        let mut built = Ok(Value::map_in(self.alloc));
        put(&mut built, vocab::ID, Value::string_in(self.alloc, id));
        put(
            &mut built,
            vocab::LABEL,
            Value::string_in(self.alloc, label),
        );
        if !help.is_empty() {
            put(&mut built, vocab::HELP, Value::string_in(self.alloc, help));
        }
        push(&mut self.state, vocab::SECTIONS, built);
        self
    }

    /// Attaches an annotation to the schema as a whole. Carried, never
    /// interpreted.
    pub fn extra(mut self, key: &str, value: Result<Value, ValueError>) -> SchemaBuilder {
        put(&mut self.state, key, value);
        self
    }

    /// The schema, or the first error that stopped it.
    pub fn finish(self) -> Result<Value, ValueError> {
        self.state
    }
}

/// The same as [`SchemaBuilder::new`].
impl Default for SchemaBuilder {
    fn default() -> SchemaBuilder {
        SchemaBuilder::new()
    }
}

impl FieldBuilder {
    /// An option under `key`, accepting `kind`.
    /// One option, through the crate's own allocator.
    pub fn new(key: &str, kind: KindBuilder) -> FieldBuilder {
        FieldBuilder::new_in(Alloc::rust(), key, kind)
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc, key: &str, kind: KindBuilder) -> FieldBuilder {
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::KEY, Value::string_in(alloc, key));
        put(&mut state, vocab::KIND, kind.state);
        FieldBuilder { alloc, state }
    }

    /// Which section this belongs to.
    pub fn section(mut self, section: &str) -> FieldBuilder {
        put(
            &mut self.state,
            vocab::SECTION,
            Value::string_in(self.alloc, section),
        );
        self
    }

    /// The default value, of whatever kind the option accepts.
    ///
    /// Setting it to null is different from not setting it: the first says
    /// the default is nothing, the second that there is no default.
    pub fn default(mut self, value: Result<Value, ValueError>) -> FieldBuilder {
        put(&mut self.state, vocab::DEFAULT, value);
        self
    }

    /// Declaration position.
    pub fn order(mut self, order: i64) -> FieldBuilder {
        put(
            &mut self.state,
            vocab::ORDER,
            Value::int_in(self.alloc, order),
        );
        self
    }

    /// Hidden behind a disclosure by default.
    pub fn advanced(mut self) -> FieldBuilder {
        put(&mut self.state, vocab::ADVANCED, Ok(Value::bool(true)));
        self
    }

    /// A secret: masked in a form, encrypted in storage.
    pub fn sensitive(mut self) -> FieldBuilder {
        put(&mut self.state, vocab::SENSITIVE, Ok(Value::bool(true)));
        self
    }

    /// Must be given.
    pub fn required(mut self) -> FieldBuilder {
        put(&mut self.state, vocab::REQUIRED, Ok(Value::bool(true)));
        self
    }

    /// An annotation. Carried, never interpreted.
    pub fn extra(mut self, key: &str, value: Result<Value, ValueError>) -> FieldBuilder {
        put(&mut self.state, key, value);
        self
    }
}

impl KindBuilder {
    fn typed(alloc: Alloc, ty: &str) -> KindBuilder {
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::TYPE, Value::string_in(alloc, ty));
        let _ = alloc;
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
        KindBuilder::typed(alloc, vocab::TYPE_BOOL)
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
        KindBuilder::typed(alloc, vocab::TYPE_INT)
    }

    /// A whole number between `min` and `max`, both inclusive.
    /// Built through the crate's own allocator. `int_range_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn int_range(min: i64, max: i64) -> KindBuilder {
        KindBuilder::int_range_in(Alloc::rust(), min, max)
    }

    /// The same, through an allocator you name.
    pub fn int_range_in(alloc: Alloc, min: i64, max: i64) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_INT);
        put(&mut k.state, vocab::MIN, Value::int_in(alloc, min));
        put(&mut k.state, vocab::MAX, Value::int_in(alloc, max));
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
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_INT);
        if let Some(min) = min {
            put(&mut k.state, vocab::MIN, Value::int_in(alloc, min));
        }
        if let Some(max) = max {
            put(&mut k.state, vocab::MAX, Value::int_in(alloc, max));
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
        KindBuilder::typed(alloc, vocab::TYPE_FLOAT)
    }

    /// A real number, bounded on either side or neither.
    /// Built through the crate's own allocator. `float_bounds_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn float_bounds(min: Option<f64>, max: Option<f64>) -> KindBuilder {
        KindBuilder::float_bounds_in(Alloc::rust(), min, max)
    }

    /// The same, through an allocator you name.
    pub fn float_bounds_in(alloc: Alloc, min: Option<f64>, max: Option<f64>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_FLOAT);
        if let Some(min) = min {
            put(&mut k.state, vocab::MIN, Value::float_in(alloc, min));
        }
        if let Some(max) = max {
            put(&mut k.state, vocab::MAX, Value::float_in(alloc, max));
        }
        k
    }

    /// Opaque bytes.
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
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_LIST);
        put(&mut k.state, vocab::ITEMS, items.state);
        k
    }

    /// A nested object with its own options.
    ///
    /// An object with no fields is ordinary and complete -- a map nothing
    /// further is declared about -- exactly as an arm with no fields is.
    /// Built through the crate's own allocator. `map_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn map(fields: Vec<FieldBuilder>) -> KindBuilder {
        KindBuilder::map_in(Alloc::rust(), fields)
    }

    /// The same, through an allocator you name.
    pub fn map_in(alloc: Alloc, fields: Vec<FieldBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_MAP);
        // Written even when empty, so a reader can tell "an object with no
        // declared fields" from "a kind that forgot to say".
        put(&mut k.state, vocab::FIELDS, Ok(Value::list_in(alloc)));
        for field in fields {
            push(&mut k.state, vocab::FIELDS, field.state);
        }
        k
    }

    /// Exactly one of a fixed set of alternatives.
    ///
    /// Each is a row of value and label, never two parallel lists: those
    /// drift in length or order, which shows a person one option while
    /// setting another.
    /// Built through the crate's own allocator. `enumeration_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn enumeration(choices: &[(&str, &str)]) -> KindBuilder {
        KindBuilder::enumeration_in(Alloc::rust(), choices)
    }

    /// The same, through an allocator you name.
    pub fn enumeration_in(alloc: Alloc, choices: &[(&str, &str)]) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_ENUM);
        for (value, label) in choices {
            let mut built = Ok(Value::map_in(alloc));
            put(&mut built, vocab::VALUE, Value::string_in(alloc, value));
            put(&mut built, vocab::LABEL, Value::string_in(alloc, label));
            push(&mut k.state, vocab::CHOICES, built);
        }
        k
    }

    /// Any one of several kinds. Untagged.
    /// Built through the crate's own allocator. `union_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn union(arms: Vec<KindBuilder>) -> KindBuilder {
        KindBuilder::union_in(Alloc::rust(), arms)
    }

    /// The same, through an allocator you name.
    pub fn union_in(alloc: Alloc, arms: Vec<KindBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_UNION);
        for arm in arms {
            push(&mut k.state, vocab::ARMS, arm.state);
        }
        k
    }

    /// One of several alternatives, each with its own fields. Tagged: the
    /// discriminant is stored under `tag`.
    /// Built through the crate's own allocator. `variant_in` names one,
    /// which is what a schema built into a host's arena needs.
    pub fn variant(tag: &str, arms: Vec<ArmBuilder>) -> KindBuilder {
        KindBuilder::variant_in(Alloc::rust(), tag, arms)
    }

    /// The same, through an allocator you name.
    pub fn variant_in(alloc: Alloc, tag: &str, arms: Vec<ArmBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_VARIANT);
        put(&mut k.state, vocab::TAG, Value::string_in(alloc, tag));
        for arm in arms {
            push(&mut k.state, vocab::ARMS, arm.state);
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
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::VALUE, Value::string_in(alloc, value));
        put(&mut state, vocab::LABEL, Value::string_in(alloc, label));
        ArmBuilder { alloc, state }
    }

    /// An option this arm adds when selected.
    pub fn field(mut self, option: FieldBuilder) -> ArmBuilder {
        push(&mut self.state, vocab::FIELDS, option.state);
        self
    }
}
