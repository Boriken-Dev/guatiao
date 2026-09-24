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
//! `description` — exactly what `guatiao::schema` calls the
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
use guatiao::value::convert::{TryAsMut, TryAsRef};
use guatiao::value::error::ValueError;
use guatiao::value::types::{Map, Text, Value};

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
    if let Err(e) = value.and_then(|v| {
        TryAsMut::<Map>::try_as_mut(node)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set(key, v))
    }) {
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
            state: Ok(Map::new_in(alloc).into()),
        }
    }

    /// The allocator this form is built through, for hints built beside
    /// it.
    pub fn alloc(&self) -> Alloc {
        self.alloc
    }

    /// Declares a section. **The order of declaration is the order a
    /// person sees.**
    pub fn section(mut self, section: Section) -> Form {
        let node = match &mut self.state {
            Ok(n) => n,
            Err(_) => return self,
        };
        if let Err(e) = section.state.and_then(|s| {
            TryAsMut::<Map>::try_as_mut(node)
                .ok_or(ValueError::WrongKind)
                .and_then(|m| m.push_into(vocab::SECTIONS, s))
        }) {
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
            if !TryAsRef::<Map>::try_as_ref(node).is_some_and(|m| m.contains_key(vocab::FIELDS)) {
                TryAsMut::<Map>::try_as_mut(node)
                    .ok_or(ValueError::WrongKind)
                    .and_then(|m| m.set(vocab::FIELDS, Map::new_in(alloc)))?;
            }
            // Present, because it was just ensured; a caller who put
            // something that is not a map there through `option` gets
            // `WrongKind` from the `set` below rather than a silent loss.
            match TryAsMut::<Map>::try_as_mut(node).and_then(|m| m.get_mut(vocab::FIELDS)) {
                Some(fields) => TryAsMut::<Map>::try_as_mut(fields)
                    .ok_or(ValueError::WrongKind)
                    .and_then(|m| m.set(path, hints)),
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
        let mut state = Ok(Map::new_in(alloc).into());
        put(
            &mut state,
            vocab::ID,
            Text::new_in(alloc, id).map(Value::from),
        );
        Section { alloc, state }
    }

    /// What a person sees the section called: its `title`.
    pub fn label(mut self, label: &str) -> Section {
        put(
            &mut self.state,
            vocab::TITLE,
            Text::new_in(self.alloc, label).map(Value::from),
        );
        self
    }

    /// Longer prose under the title: its `description`.
    pub fn help(mut self, help: &str) -> Section {
        put(
            &mut self.state,
            vocab::DESCRIPTION,
            Text::new_in(self.alloc, help).map(Value::from),
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
            state: Ok(Map::new_in(alloc).into()),
        }
    }

    /// Which control to draw. A name from an open set; see
    /// [`vocab::widget`].
    pub fn widget(mut self, widget: &str) -> Hints {
        put(
            &mut self.state,
            vocab::WIDGET,
            Text::new_in(self.alloc, widget).map(Value::from),
        );
        self
    }

    /// Text shown in the empty control. Never stored, and not a default.
    pub fn placeholder(mut self, text: &str) -> Hints {
        put(
            &mut self.state,
            vocab::PLACEHOLDER,
            Text::new_in(self.alloc, text).map(Value::from),
        );
        self
    }

    /// Shows the field only while the field at `path` holds `equals`.
    ///
    /// For a variant named by its own key, `equals` is the name of an arm.
    pub fn visible_when(mut self, path: &str, equals: impl Into<Value>) -> Hints {
        let alloc = self.alloc;
        let mut condition = Map::new_in(alloc);
        let built = Text::new_in(alloc, path)
            .and_then(|p| condition.set(vocab::FIELD, p))
            .and_then(|()| condition.set(vocab::EQUALS, equals.into()))
            .map(|()| Value::from(condition));
        put(&mut self.state, vocab::VISIBLE_WHEN, built);
        self
    }

    /// A form of this field's own, for a field whose kind is an object.
    ///
    /// Its paths are **relative to this field**: a form given to
    /// `connection` names `tls.ca`, not `connection.tls.ca`. Pair it
    /// with [`widget`](Hints::widget) of
    /// [`DIALOG`](vocab::widget::DIALOG) for a window of its own, or
    /// leave the widget off and a renderer groups the members where the
    /// field is.
    ///
    /// The form is finished here rather than by the caller, so its first
    /// error travels with this one rather than being unwrapped at the
    /// call site.
    pub fn form(mut self, form: Form) -> Hints {
        let built = form.finish();
        put(&mut self.state, vocab::FORM, built);
        self
    }

    /// The same, for a form already finished.
    ///
    /// **For generated code**, which has a `Result` and nowhere to put a
    /// failure -- `#[derive(Form)]` composes a member's form this way.
    pub fn form_value(mut self, form: Result<Value, ValueError>) -> Hints {
        put(&mut self.state, vocab::FORM, form);
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
