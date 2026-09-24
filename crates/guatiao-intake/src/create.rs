// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The form a schema implies, for when nobody wrote one.
//!
//! **An object is a form.** A renderer handed a schema and no form
//! should not have to invent the grouping itself — the schema already
//! says which section each field belongs to, and a form made of exactly
//! that is one every renderer agrees on.
//!
//! # What a created form says, and what it does not
//!
//! It says **one section per distinct `x-section` the fields name**, in
//! first-appearance order, and **ids only**. What a section is *called*
//! is a form's business and a schema has no opinion, so a created form
//! declares the id and leaves the title empty; a renderer shows the id,
//! or a person writes a form that titles it.
//!
//! It also carries, for each member that has members of its own, **that
//! member's created form** under its `form` hint — but only when there
//! is something in it. A nested object whose fields name no section
//! contributes nothing, and a form full of empty forms is noise.
//!
//! It says nothing about widgets or placeholders. A schema does not know
//! them, and inventing one would be this crate guessing.
//!
//! # Empty is complete
//!
//! For a schema whose fields name no section at all, the created form is
//! `{}` — valid, and exactly as informative as the schema was. That is
//! the answer, not a failure.

#![forbid(unsafe_code)]

use guatiao::schema::read::{FieldRef, SchemaRef};
use guatiao::value::alloc::Alloc;
use guatiao::value::error::{MAX_DEPTH, ValueError};
use guatiao::value::types::Value;

use crate::build::{Form, Hints, Section};
use crate::judge::member_schema;
use crate::read::FormRef;

/// The form a schema implies.
///
/// ```
/// use guatiao::schema::read::SchemaRef;
/// use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
/// use guatiao::value::alloc::Alloc;
/// use guatiao_intake::{FormBuilder, FormRef, check, for_schema};
///
/// let schema = SchemaBuilder::new()
///     .field(FieldBuilder::new("host", KindBuilder::string()).section("net"))
///     .field(FieldBuilder::new("port", KindBuilder::int()).section("net"))
///     .field(FieldBuilder::new("name", KindBuilder::string()))
///     .finish()?;
///
/// let s = SchemaRef::new(&schema).expect("a schema is a map");
/// let form = for_schema(s, Alloc::rust())?;
/// let f = FormRef::new(&form).expect("what it made is a form");
///
/// // One section, because the fields name one; `name` names none and
/// // joins the default group.
/// let ids: Vec<&str> = f.sections().map(|s| s.id()).collect();
/// assert_eq!(ids, ["net"]);
/// check(s, f).expect("what a schema implies fits that schema");
/// # Ok::<(), guatiao::ValueError>(())
/// ```
pub fn for_schema(schema: SchemaRef<'_>, alloc: Alloc) -> Result<Value, ValueError> {
    build(schema, alloc, 0)
}

/// The form a **field** implies, from the members it has.
///
/// `None` when the field has none — a text is shown by a control, not by
/// a form, and there is nothing to make one out of. An object, a list of
/// objects and a map of them each answer with the form of their member.
pub fn for_field(field: FieldRef<'_>, alloc: Alloc) -> Option<Result<Value, ValueError>> {
    let members = SchemaRef::new(member_schema(field)?)?;
    Some(build(members, alloc, 0))
}

/// The form to show the field at `path` with: **the one assigned, or the
/// one its schema implies**.
///
/// This is the question a renderer actually asks. A form may assign a
/// sub-form to a field ([`Hints::form`](crate::Hints::form)), and
/// assignment wins — somebody wrote it down. Where nobody did, the
/// schema still says enough to group the member's fields, so the answer
/// is [`for_field`] rather than nothing.
///
/// `None` when the path names no field, or names one with no members and
/// no form assigned to it.
///
/// **What comes back is owned**, in `alloc`, whichever way it was
/// reached: an assigned form is copied out of the document rather than
/// borrowed from it, so a caller holds one kind of thing.
pub fn form_for(
    form: FormRef<'_>,
    schema: SchemaRef<'_>,
    path: &str,
    alloc: Alloc,
) -> Option<Result<Value, ValueError>> {
    if let Some(assigned) = form.hints(path).form() {
        return Some(assigned.as_value().clone_in(alloc));
    }
    for_field(crate::flat::resolve(schema, path)?, alloc)
}

/// One level, bounded: a schema whose members nest each other past the
/// bound is refused rather than followed.
fn build(schema: SchemaRef<'_>, alloc: Alloc, depth: u32) -> Result<Value, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    let mut form = Form::new_in(alloc);

    // The sections, in the order the fields first name them. Ids only:
    // see the module note.
    let mut seen: Vec<&str> = Vec::new();
    for field in schema.fields() {
        let id = crate::FormField::section(&field);
        if !id.is_empty() && !seen.contains(&id) {
            seen.push(id);
            form = form.section(Section::new_in(alloc, id));
        }
    }

    // Then each member that has a form of its own worth carrying.
    for field in schema.fields() {
        let Some(members) = member_schema(field).and_then(SchemaRef::new) else {
            continue;
        };
        let nested = build(members, alloc, depth + 1)?;
        if says_nothing(&nested) {
            continue;
        }
        form = form.field(field.key(), Hints::new_in(alloc).form_value(Ok(nested)));
    }

    form.finish()
}

/// Whether a form would tell a renderer anything it did not know.
fn says_nothing(form: &Value) -> bool {
    FormRef::new(form).is_none_or(|f| f.sections().next().is_none() && f.fields().next().is_none())
}
