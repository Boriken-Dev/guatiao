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
pub struct OptionBuilder {
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

impl std::fmt::Debug for OptionBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "OptionBuilder", &self.state)
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

impl SchemaBuilder {
    /// An empty schema.
    pub fn new(alloc: Alloc) -> SchemaBuilder {
        SchemaBuilder {
            alloc,
            state: Ok(Value::map_in(alloc)),
        }
    }

    /// Declares an option. Order of declaration is the order a consumer
    /// sees.
    pub fn option(mut self, option: OptionBuilder) -> SchemaBuilder {
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

impl OptionBuilder {
    /// An option under `key`, accepting `kind`.
    pub fn new(alloc: Alloc, key: &str, kind: KindBuilder) -> OptionBuilder {
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::KEY, Value::string_in(alloc, key));
        put(&mut state, vocab::KIND, kind.state);
        OptionBuilder { alloc, state }
    }

    /// A short human label.
    pub fn label(mut self, label: &str) -> OptionBuilder {
        put(
            &mut self.state,
            vocab::LABEL,
            Value::string_in(self.alloc, label),
        );
        self
    }

    /// Longer human help.
    pub fn help(mut self, help: &str) -> OptionBuilder {
        put(
            &mut self.state,
            vocab::HELP,
            Value::string_in(self.alloc, help),
        );
        self
    }

    /// Which section this belongs to.
    pub fn section(mut self, section: &str) -> OptionBuilder {
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
    pub fn default(mut self, value: Result<Value, ValueError>) -> OptionBuilder {
        put(&mut self.state, vocab::DEFAULT, value);
        self
    }

    /// Declaration position.
    pub fn order(mut self, order: i64) -> OptionBuilder {
        put(
            &mut self.state,
            vocab::ORDER,
            Value::int_in(self.alloc, order),
        );
        self
    }

    /// Hidden behind a disclosure by default.
    pub fn advanced(mut self) -> OptionBuilder {
        put(&mut self.state, vocab::ADVANCED, Ok(Value::bool(true)));
        self
    }

    /// A secret: masked in a form, encrypted in storage.
    pub fn sensitive(mut self) -> OptionBuilder {
        put(&mut self.state, vocab::SENSITIVE, Ok(Value::bool(true)));
        self
    }

    /// Must be given.
    pub fn required(mut self) -> OptionBuilder {
        put(&mut self.state, vocab::REQUIRED, Ok(Value::bool(true)));
        self
    }

    /// An annotation. Carried, never interpreted.
    pub fn extra(mut self, key: &str, value: Result<Value, ValueError>) -> OptionBuilder {
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
    pub fn bool(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_BOOL)
    }

    /// Free text.
    pub fn string(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_STRING)
    }

    /// A whole number, unbounded.
    pub fn int(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_INT)
    }

    /// A whole number between `min` and `max`, both inclusive.
    pub fn int_range(alloc: Alloc, min: i64, max: i64) -> KindBuilder {
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
    pub fn int_bounds(alloc: Alloc, min: Option<i64>, max: Option<i64>) -> KindBuilder {
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
    pub fn float(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_FLOAT)
    }

    /// A real number, bounded on either side or neither.
    pub fn float_bounds(alloc: Alloc, min: Option<f64>, max: Option<f64>) -> KindBuilder {
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
    pub fn bytes(alloc: Alloc) -> KindBuilder {
        KindBuilder::typed(alloc, vocab::TYPE_BYTES)
    }

    /// A sequence, every element of `items`.
    pub fn list(alloc: Alloc, items: KindBuilder) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_LIST);
        put(&mut k.state, vocab::ITEMS, items.state);
        k
    }

    /// A nested object with its own options.
    ///
    /// An object with no fields is ordinary and complete -- a map nothing
    /// further is declared about -- exactly as an arm with no fields is.
    pub fn map(alloc: Alloc, fields: Vec<OptionBuilder>) -> KindBuilder {
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
    pub fn enumeration(alloc: Alloc, choices: &[(&str, &str)]) -> KindBuilder {
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
    pub fn union(alloc: Alloc, arms: Vec<KindBuilder>) -> KindBuilder {
        let mut k = KindBuilder::typed(alloc, vocab::TYPE_UNION);
        for arm in arms {
            push(&mut k.state, vocab::ARMS, arm.state);
        }
        k
    }

    /// One of several alternatives, each with its own fields. Tagged: the
    /// discriminant is stored under `tag`.
    pub fn variant(alloc: Alloc, tag: &str, arms: Vec<ArmBuilder>) -> KindBuilder {
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
    pub fn new(alloc: Alloc, value: &str, label: &str) -> ArmBuilder {
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::VALUE, Value::string_in(alloc, value));
        put(&mut state, vocab::LABEL, Value::string_in(alloc, label));
        ArmBuilder { alloc, state }
    }

    /// Longer human help.
    pub fn help(mut self, help: &str) -> ArmBuilder {
        put(
            &mut self.state,
            vocab::HELP,
            Value::string_in(self.alloc, help),
        );
        self
    }

    /// An option this arm adds when selected.
    pub fn field(mut self, option: OptionBuilder) -> ArmBuilder {
        push(&mut self.state, vocab::FIELDS, option.state);
        self
    }
}
