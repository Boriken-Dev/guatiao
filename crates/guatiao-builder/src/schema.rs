// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao schema made from a description, one to one.
//!
//! | description | schema |
//! | --- | --- |
//! | a leaf of a builtin type | what guatiao's `Schema` gives that Rust type (`u16` bounds, an address's `format`) |
//! | a `Duration` | `anyOf` a number from 0 (seconds) and a string matching [`DURATION_PATTERN`] |
//! | any other leaf | a string, an `enum` with `x-enum-labels` when it has choices, or the names offered as `examples` when it also accepts a name outside them |
//! | a list, an open map | `array`, an object whose values are the item's schema |
//! | a group | a sealed object: `properties`, `required`, `additionalProperties: false` |
//! | a variant | an object with `x-variant-tag` and a `oneOf` arm per name |
//! | an untagged enum | `anyOf` its text side (a string) and its structured arm |
//! | a field with a written `None` | `anyOf` the one text and the field's schema |
//!
//! A field is `required` as the description says, its `default` is the default's text through
//! [`crate::text::to_value_in`], and its notes become `title`, `description`, `x-sensitive`
//! and `writeOnly`, `readOnly`, `deprecated`, `examples`, the bounds and
//! `x-section`, `x-order` and `x-advanced`.

use alloc::string::ToString;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::PathBuf;

use coqui_builder::Description;
use coqui_builder::describe::{Field, Kind, LeafInfo, Notes};
use guatiao::schema::{ArmBuilder, Extras, FieldBuilder, KindBuilder, vocab};
use guatiao::{Alloc, Map, Number, Schema, Text, Value, ValueError};

use crate::text;

/// The text of a duration, as coqui-builder reads it: a decimal number and an optional unit
/// (`ns`, `us`, `ms`, `s`, `m`, `h`; no unit is seconds), with no sign and no spaces.
pub const DURATION_PATTERN: &str = r"^[0-9]+(\.[0-9]+)?(ns|us|ms|s|m|h)?$";

/// A text no choice can be: a leaf that accepts it also accepts names outside its choices.
const PROBE: &str = "\u{1}guatiao-builder-probe";

/// The kind a description describes.
pub fn kind_in(description: &Description, alloc: Alloc) -> KindBuilder {
    let kind = plain_kind(description, alloc);
    // A secret by itself (a `Secret`, an item of a list of them) marks its own kind; a field
    // marks the field instead.
    let kind = if description.notes().sensitive {
        kind.sensitive()
    } else {
        kind
    };
    bounded(kind, description.notes())
}

/// The kind of a description before its notes' shape keywords are chained on.
fn plain_kind(description: &Description, alloc: Alloc) -> KindBuilder {
    match description.kind() {
        Kind::Leaf(info) => leaf_kind(info, alloc),
        Kind::List(item) => KindBuilder::list_in(alloc, kind_in(item, alloc)),
        Kind::Map(value) => KindBuilder::map_of_in(alloc, kind_in(value, alloc)),
        Kind::Group(group) => KindBuilder::map_in(
            alloc,
            group.fields.iter().map(|f| field_in(f, alloc)).collect(),
        ),
        Kind::Variant(variant) => KindBuilder::variant_in(
            alloc,
            variant.tag(),
            variant
                .arms()
                .iter()
                .map(|arm| {
                    let mut built =
                        ArmBuilder::new_in(alloc, &arm.name, arm.title.as_deref().unwrap_or(""));
                    if let Some(description) = &arm.description {
                        built = built.description(description);
                    }
                    for field in &arm.fields {
                        built = built.field(field_in(field, alloc));
                    }
                    built
                })
                .collect(),
        ),
        Kind::Union(union) => KindBuilder::union_in(
            alloc,
            alloc::vec![
                text_kind(union.text(), alloc),
                kind_in(union.structured(), alloc)
            ],
        ),
        // A kind added later is a value of any shape until it is described here.
        _ => KindBuilder::any_in(alloc),
    }
}

/// The kind of a leaf: a builtin type's own, else a string.
fn leaf_kind(info: &LeafInfo, alloc: Alloc) -> KindBuilder {
    match info.ty() {
        "bool" => bool::kind(alloc),
        "u8" => u8::kind(alloc),
        "u16" => u16::kind(alloc),
        "u32" => u32::kind(alloc),
        "u64" => u64::kind(alloc),
        "u128" => u128::kind(alloc),
        "usize" => usize::kind(alloc),
        "i8" => i8::kind(alloc),
        "i16" => i16::kind(alloc),
        "i32" => i32::kind(alloc),
        "i64" => i64::kind(alloc),
        "i128" => i128::kind(alloc),
        "isize" => isize::kind(alloc),
        "f32" => f32::kind(alloc),
        "f64" => f64::kind(alloc),
        "String" => String::kind(alloc),
        "char" => char::kind(alloc),
        "Path" => PathBuf::kind(alloc),
        "IpAddr" => IpAddr::kind(alloc),
        "Ipv4Addr" => Ipv4Addr::kind(alloc),
        "Ipv6Addr" => Ipv6Addr::kind(alloc),
        "SocketAddr" => SocketAddr::kind(alloc),
        "SocketAddrV4" => SocketAddrV4::kind(alloc),
        "SocketAddrV6" => SocketAddrV6::kind(alloc),
        "Duration" => duration_kind(alloc),
        _ => text_kind(info, alloc),
    }
}

/// A duration: seconds as a number, or its text.
fn duration_kind(alloc: Alloc) -> KindBuilder {
    KindBuilder::union_in(
        alloc,
        alloc::vec![
            KindBuilder::float_bounds_in(alloc, Some(0.0), None),
            KindBuilder::string_in(alloc).pattern(DURATION_PATTERN),
        ],
    )
}

/// A string kind: with the leaf's choices when it has them.
fn text_kind(info: &LeafInfo, alloc: Alloc) -> KindBuilder {
    let Some(names) = info.choices() else {
        return KindBuilder::string_in(alloc);
    };
    let rows: Vec<(&str, &str)> = names
        .iter()
        .map(|name| (*name, info.label(name).unwrap_or("")))
        .collect();
    if info.check(PROBE).is_ok() {
        KindBuilder::open_enumeration_in(alloc, &rows)
    } else {
        KindBuilder::enumeration_in(alloc, &rows)
    }
}

/// The shape keywords of a description's notes, in the order guatiao's derive writes them.
fn bounded(mut kind: KindBuilder, notes: &Notes) -> KindBuilder {
    if let Some(n) = notes.min_length {
        kind = kind.min_length(n);
    }
    if let Some(n) = notes.max_length {
        kind = kind.max_length(n);
    }
    if let Some(n) = notes.min_items {
        kind = kind.min_items(n);
    }
    if let Some(n) = notes.max_items {
        kind = kind.max_items(n);
    }
    if let Some(pattern) = &notes.pattern {
        kind = kind.pattern(pattern);
    }
    if let Some(format) = &notes.format {
        kind = kind.format(format);
    }
    if let Some(step) = notes.multiple_of {
        kind = kind.multiple_of(step);
    }
    kind
}

/// The type name of the text a description holds: a leaf's, an untagged enum's text side's.
fn text_type(description: &Description) -> &'static str {
    match description.kind() {
        Kind::Leaf(info) => info.ty(),
        Kind::Union(union) => union.text().ty(),
        Kind::List(item) => text_type(item),
        _ => "String",
    }
}

/// A field, with its notes, requiredness and default.
fn field_in(field: &Field, alloc: Alloc) -> FieldBuilder {
    let description = &field.description;
    let notes = description.notes();
    // As guatiao's derive orders it: the written `None` or the null an `Option` accepts
    // first, then the shape keywords on the result.
    let mut kind = plain_kind(description, alloc);
    if let Some(none) = &field.none {
        kind = KindBuilder::union_in(
            alloc,
            alloc::vec![KindBuilder::enumeration_in(alloc, &[(none, "")]), kind],
        );
    } else if field.optional {
        kind = kind.nullable();
    }
    kind = bounded(kind, notes);
    let mut built = FieldBuilder::new_in(alloc, &field.key, kind);
    if let Some(title) = &notes.title {
        built = built.title(title);
    }
    if let Some(text) = &notes.description {
        built = built.description(text);
    }
    if let Some(section) = &notes.section {
        built = built.extra("x-section", Text::new_in(alloc, section).map(Value::from));
    }
    if let Some(order) = notes.order {
        built = built.extra(
            "x-order",
            Number::new_in(alloc, &order.to_string()).map(Value::from),
        );
    }
    if notes.advanced {
        built = built.extra("x-advanced", Ok(Value::from(true)));
    }
    if notes.sensitive {
        built = built.sensitive();
    }
    if notes.read_only {
        built = built.read_only();
    }
    if notes.deprecated {
        built = built.deprecated();
    }
    if !notes.examples.is_empty() {
        let ty = text_type(description);
        let examples: Result<Vec<Value>, ValueError> = notes
            .examples
            .iter()
            .map(|example| text::to_value_in(ty, example, alloc))
            .collect();
        built = match examples {
            Ok(examples) => built.examples(examples),
            Err(error) => built.extra(vocab::EXAMPLES, Err(error)),
        };
    }
    if field.required {
        built = built.required();
    }
    if let Some(items) = &field.default_items {
        let ty = text_type(description);
        let list: Result<Value, ValueError> = (|| {
            let mut list = guatiao::List::new_in(alloc);
            for item in items {
                list.push_in(text::to_value_in(ty, item, alloc)?, alloc)?;
            }
            Ok(list.into())
        })();
        built = built.default_checked(list);
    } else if let Some(default) = &field.default {
        // The text that writes a `None` is text whatever the type.
        let ty = if field.none.as_ref() == Some(default) {
            "String"
        } else {
            text_type(description)
        };
        built = built.default_checked(text::to_value_in(ty, default, alloc));
    }
    built
}

/// The whole schema of a description, built in `alloc`: the dialect, `type: "object"`, then
/// the kind's own keys, as `Schema::schema_in` writes a derived type's.
///
/// # Errors
///
/// [`ValueError`] when an allocation fails or the description cannot be written as a schema.
pub fn schema_in(description: &Description, alloc: Alloc) -> Result<Value, ValueError> {
    if !matches!(
        description.kind(),
        Kind::Group(_) | Kind::Variant(_) | Kind::Map(_)
    ) {
        return Err(ValueError::WrongKind);
    }
    let kind = kind_in(description, alloc).finish()?;
    let mut out = Map::new_in(alloc);
    out.try_set(
        vocab::SCHEMA,
        Text::new_in(alloc, vocab::DIALECT).map(Value::from)?,
    )?;
    out.try_set(
        vocab::TYPE,
        Text::new_in(alloc, vocab::TYPE_OBJECT).map(Value::from)?,
    )?;
    if let Ok(entries) = <&Map>::try_from(&kind) {
        for entry in entries.iter() {
            out.try_set(entry.key(), entry.value().clone_in(alloc)?)?;
        }
    }
    if out.get(vocab::PROPERTIES).is_none() {
        out.try_set(vocab::PROPERTIES, Map::new_in(alloc))?;
    }
    Ok(out.into())
}

/// The whole schema of a description, through guatiao's own allocator.
///
/// # Errors
///
/// As [`schema_in`].
pub fn schema(description: &Description) -> Result<Value, ValueError> {
    schema_in(description, Alloc::rust())
}
