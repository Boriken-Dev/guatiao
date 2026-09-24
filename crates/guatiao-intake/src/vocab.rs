// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The keys a form is written with.
//!
//! # The shape
//!
//! ```text
//! form     := { "sections": [section, …], "fields": { <path>: hints, … } }
//! section  := { "id": text, "title": text, "description": text }
//! hints    := { "widget": text, "placeholder": text,
//!               "visibleWhen": { "field": <path>, "equals": <value> } }
//! ```
//!
//! Every key is optional except a section's `id`. **An empty map is a
//! form**: it shows the schema exactly the way the schema alone says to.
//!
//! Three more keys are this crate's and do **not** live in the form
//! document: [`X_SECTION`], [`X_ORDER`] and [`X_ADVANCED`] sit on the
//! **schema**, beside the field they describe, because a field can say
//! them about itself. They are named here rather than in `guatiao`
//! because they are opinions about how to organise controls, and that
//! crate holds none — it carries them as annotations, like any key it
//! does not know. [`declare`](crate::declare) writes them and reads them
//! back.
//!
//! # Rules a reader must follow
//!
//! **A path is a flat key**: a field's own key, or `owner.member` for a
//! field an arm of a variant adds. It is the spelling
//! [`crate::flat`] already resolves, and the one a C caller already
//! passes to `guatiao_intake_resolve`.
//!
//! **Sections are listed in display order.** A section is what a person
//! sees a group of fields called; which section a field belongs to is the
//! schema's `x-section`, declared beside the field, and a form never
//! repeats it.
//!
//! **The default section's id is the empty string**, because that is what
//! a field with no `x-section` reads as. Declaring it gives the ungrouped
//! fields a title and a position; not declaring it puts them first.
//!
//! **A widget name is a hint from an open set.** A renderer that does not
//! know one shows the field the way it shows any other field of that kind.
//! The names in [`widget`] are suggestions that keep two renderers from
//! spelling one control two ways, not a closed list.
//!
//! **Anything not listed here is an annotation**, carried and never
//! interpreted — the same rule the schema has.
//!
//! **A multi-word key is camelCase** (`visibleWhen`), matching the JSON
//! Schema document a form sits beside (`anyOf`, `oneOf`).

#![forbid(unsafe_code)]

// --- the form -----------------------------------------------------------

/// The sections a form declares, as a list in display order.
pub const SECTIONS: &str = "sections";
/// Per-field hints: a map from a field's path to what it is shown with.
pub const FIELDS: &str = "fields";

// --- a section ----------------------------------------------------------

/// A section's identifier: what a field's `x-section` names.
pub const ID: &str = "id";
/// What a section is called. The same key, and the same meaning, as a
/// schema's `title`.
pub const TITLE: &str = guatiao::schema::vocab::TITLE;
/// Longer prose under a section's title. The same key as a schema's
/// `description`.
pub const DESCRIPTION: &str = guatiao::schema::vocab::DESCRIPTION;
/// The id of the default section, which every field without an
/// `x-section` — and every field naming a section the form does not
/// declare — belongs to.
pub const DEFAULT_SECTION: &str = "";

// --- a field's hints ----------------------------------------------------

/// Which control to draw. A name from an open set; see [`widget`].
pub const WIDGET: &str = "widget";
/// Text shown in an empty control. **Not a default**: it is never stored,
/// and a field with a real default says so in the schema.
pub const PLACEHOLDER: &str = "placeholder";
/// The one condition under which a field is shown.
///
/// For what a variant cannot say. "These fields exist only for this arm"
/// is already a variant; "show `ca` only when `verify` is true" is this.
pub const VISIBLE_WHEN: &str = "visibleWhen";

// --- written onto the SCHEMA, beside a field ---------------------------
//
// Not part of the form document: these three sit on the schema itself,
// where a field can carry them. They are named here rather than in
// `guatiao` because they are opinions about how to organise controls on a
// screen, and that crate holds none. It carries them as annotations, like
// any key it does not know.
//
// `x-sensitive` is NOT one of them. It stays `guatiao`'s, on
// `FieldBuilder::sensitive` and `FieldRef::is_sensitive`, because "never
// print this value" is obeyed by a log and a crash dump as much as by a
// form.

/// Which section a field belongs to, by whatever id draws the form.
///
/// Naming a section nothing declared is not an error: a consumer that does
/// not know it puts the field wherever it puts the ungrouped ones.
pub const X_SECTION: &str = "x-section";
/// Declaration position, as a number.
///
/// Not a preference. If every field omits it, a consumer has no ordering
/// information and falls back to something arbitrary — alphabetical,
/// usually — which silently rearranges a carefully grouped form.
pub const X_ORDER: &str = "x-order";
/// True when the field is advanced: hidden behind a disclosure by default.
pub const X_ADVANCED: &str = "x-advanced";

// --- a condition --------------------------------------------------------

/// The path of the field a condition reads.
pub const FIELD: &str = "field";
/// The value that field must hold for the condition to be met.
///
/// Compared as a value, structurally. For a variant named by its own key
/// it is compared with the **discriminant** — the arm a person picked —
/// because that is the one thing about a variant a single value can say.
pub const EQUALS: &str = "equals";

/// Widget names worth agreeing on. **An open set**: a renderer that does
/// not know a name falls back to its default for the field's kind.
pub mod widget {
    /// A single line of text.
    pub const TEXT: &str = "text";
    /// Several lines of text.
    pub const TEXTAREA: &str = "textarea";
    /// Text that is masked while typed.
    pub const PASSWORD: &str = "password";
    /// A number typed in.
    pub const NUMBER: &str = "number";
    /// A number chosen along its declared range.
    pub const SLIDER: &str = "slider";
    /// A boolean as a box to tick.
    pub const CHECKBOX: &str = "checkbox";
    /// A boolean as a switch.
    pub const TOGGLE: &str = "toggle";
    /// One choice from a dropdown.
    pub const SELECT: &str = "select";
    /// One choice, every alternative visible at once.
    pub const RADIO: &str = "radio";
}

/// The keys this vocabulary gives a meaning at the top of a form.
pub(crate) const FORM_KEYS: &[&str] = &[SECTIONS, FIELDS];
/// The keys it gives a meaning inside a section.
pub(crate) const SECTION_KEYS: &[&str] = &[ID, TITLE, DESCRIPTION];
/// The keys it gives a meaning inside a field's hints.
pub(crate) const HINT_KEYS: &[&str] = &[WIDGET, PLACEHOLDER, VISIBLE_WHEN];

#[cfg(test)]
mod tests {
    use super::*;

    /// A key means one thing at each level of the document.
    #[test]
    fn every_level_names_distinct_keys() {
        for level in [FORM_KEYS, SECTION_KEYS, HINT_KEYS] {
            let mut keys = level.to_vec();
            keys.sort_unstable();
            let before = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), before, "{level:?}");
        }
    }

    /// A section's title is spelled the way the schema spells one, so a
    /// reader that knows the one knows the other.
    #[test]
    fn a_title_is_the_schema_s_title() {
        assert_eq!(TITLE, "title");
        assert_eq!(DESCRIPTION, "description");
    }
}
