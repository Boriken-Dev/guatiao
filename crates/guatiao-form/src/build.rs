// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a form as a value.
//!
//! A form is a map written with the keys in [`crate::vocab`], so it can be
//! built by hand with `set` and `push`. These builders read as a
//! declaration instead, and cannot misspell a key.
//!
//! # The Rust names are the schema builders' names
//!
//! A section has a `label` and `help`, written as `title` and
//! `description` — exactly what `guatiao::schema::FormBuilder` calls the
//! same two things, so one vocabulary covers both halves of a screen.
//!
//! # Building names no allocator; errors are collected
//!
//! The same two rules the schema builders keep. `Form::new()` uses the
//! crate's allocator and the `_in` forms name one. A step that fails is
//! remembered, and [`Form::finish`] hands back the first failure, which is
//! the one that explains the rest.

#![forbid(unsafe_code)]

use guatiao::value::alloc::Alloc;
use guatiao::value::mutate::ValueError;
use guatiao::value::types::Value;

use crate::vocab;

/// Builds a form.
pub struct Form {
    alloc: Alloc,
    state: Result<Value, ValueError>,
}

/// Builds one section: what a group of fields is called.
pub struct Section {
    alloc: Alloc,
    state: Result<Value, ValueError>,
}

/// Builds the hints for one field.
pub struct Hints {
    alloc: Alloc,
    state: Result<Value, ValueError>,
}

impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "Form", &self.state)
    }
}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "Section", &self.state)
    }
}

impl std::fmt::Debug for Hints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_state(f, "Hints", &self.state)
    }
}

/// Whether a builder has failed, and with what — never what it has built
/// so far, because a builder is printed when something already went wrong.
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
        Err(_) => return,
    };
    if let Err(e) = value.and_then(|v| node.set(key, v)) {
        *state = Err(e);
    }
}

impl Form {
    /// An empty form, through the crate's own allocator.
    ///
    /// Empty is complete: it shows the schema the way the schema alone
    /// says to.
    pub fn new() -> Form {
        Form::new_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc) -> Form {
        Form {
            alloc,
            state: Ok(Value::map_in(alloc)),
        }
    }

    /// Declares a section. **The order of declaration is the order a
    /// person sees.**
    pub fn section(mut self, section: Section) -> Form {
        let node = match &mut self.state {
            Ok(n) => n,
            Err(_) => return self,
        };
        if let Err(e) = section
            .state
            .and_then(|s| node.push_into(vocab::SECTIONS, s))
        {
            self.state = Err(e);
        }
        self
    }

    /// Gives the field at `path` its hints.
    ///
    /// `path` is a field's key, or `owner.member` for a field an arm adds.
    /// Naming a path twice replaces the first hints, the way setting a key
    /// twice does anywhere else.
    pub fn field(mut self, path: &str, hints: Hints) -> Form {
        let alloc = self.alloc;
        let node = match &mut self.state {
            Ok(n) => n,
            Err(_) => return self,
        };
        let written = hints.state.and_then(|hints| {
            if !node.contains_key(vocab::FIELDS) {
                node.set(vocab::FIELDS, Value::map_in(alloc))?;
            }
            // Present, because it was just ensured; a caller who put
            // something that is not a map there through `option` gets
            // `WrongKind` from the `set` below rather than a silent loss.
            match node.get_mut(vocab::FIELDS) {
                Some(fields) => fields.set(path, hints),
                None => Err(ValueError::WrongKind),
            }
        });
        if let Err(e) = written {
            self.state = Err(e);
        }
        self
    }

    /// Sets any key at all: one from [`vocab`], or an annotation nobody
    /// here interprets.
    pub fn option(mut self, key: &str, value: impl Into<Value>) -> Form {
        put(&mut self.state, key, Ok(value.into()));
        self
    }

    /// The form, or the first error that stopped it.
    pub fn finish(self) -> Result<Value, ValueError> {
        self.state
    }
}

/// The same as [`Form::new`].
impl Default for Form {
    fn default() -> Form {
        Form::new()
    }
}

impl Section {
    /// The section a field's `x-section` names as `id`.
    ///
    /// `""` is the default section; declaring it gives the ungrouped
    /// fields a title and a place in the order.
    pub fn new(id: &str) -> Section {
        Section::new_in(Alloc::rust(), id)
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc, id: &str) -> Section {
        let mut state = Ok(Value::map_in(alloc));
        put(&mut state, vocab::ID, Value::string_in(alloc, id));
        Section { alloc, state }
    }

    /// What a person sees the section called: its `title`.
    pub fn label(mut self, label: &str) -> Section {
        put(
            &mut self.state,
            vocab::TITLE,
            Value::string_in(self.alloc, label),
        );
        self
    }

    /// Longer prose under the title: its `description`.
    pub fn help(mut self, help: &str) -> Section {
        put(
            &mut self.state,
            vocab::DESCRIPTION,
            Value::string_in(self.alloc, help),
        );
        self
    }

    /// Sets any key: an annotation on the section.
    pub fn option(mut self, key: &str, value: impl Into<Value>) -> Section {
        put(&mut self.state, key, Ok(value.into()));
        self
    }
}

impl Hints {
    /// No hints: the field is drawn the way its kind is drawn by default.
    pub fn new() -> Hints {
        Hints::new_in(Alloc::rust())
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc) -> Hints {
        Hints {
            alloc,
            state: Ok(Value::map_in(alloc)),
        }
    }

    /// Which control to draw. A name from an open set; see
    /// [`vocab::widget`].
    pub fn widget(mut self, widget: &str) -> Hints {
        put(
            &mut self.state,
            vocab::WIDGET,
            Value::string_in(self.alloc, widget),
        );
        self
    }

    /// Text shown in the empty control. Never stored, and not a default.
    pub fn placeholder(mut self, text: &str) -> Hints {
        put(
            &mut self.state,
            vocab::PLACEHOLDER,
            Value::string_in(self.alloc, text),
        );
        self
    }

    /// Shows the field only while the field at `path` holds `equals`.
    ///
    /// For a variant named by its own key, `equals` is the name of an arm.
    pub fn visible_when(mut self, path: &str, equals: impl Into<Value>) -> Hints {
        let alloc = self.alloc;
        let mut condition = Value::map_in(alloc);
        let built = Value::string_in(alloc, path)
            .and_then(|p| condition.set(vocab::FIELD, p))
            .and_then(|()| condition.set(vocab::EQUALS, equals.into()))
            .map(|()| condition);
        put(&mut self.state, vocab::VISIBLE_WHEN, built);
        self
    }

    /// Sets any key: an annotation on this field's hints.
    pub fn option(mut self, key: &str, value: impl Into<Value>) -> Hints {
        put(&mut self.state, key, Ok(value.into()));
        self
    }
}

/// The same as [`Hints::new`].
impl Default for Hints {
    fn default() -> Hints {
        Hints::new()
    }
}
