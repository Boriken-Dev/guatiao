// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Rust type describing the values it accepts.
//!
//! [`Schema`] is the third derive, beside `ToValue` and `FromValue`. The
//! first two move a struct across a boundary; this one describes it, so a
//! consumer that has never seen the type can build a value the type will
//! accept — which is the whole job a schema exists for.
//!
//! ```
//! use guatiao::schema::read::{Kind, SchemaRef};
//! use guatiao::{Alloc, Schema};
//!
//! #[derive(Schema)]
//! struct Connection {
//!     /// Where to connect.
//!     host: String,
//!     port: u16,
//!     motd: Option<String>,
//! }
//!
//! let alloc = Alloc::rust();
//!
//! let declared = Connection::schema_in(alloc)?;
//! let s = SchemaRef::new(&declared).unwrap();
//!
//! let host = s.find("host").unwrap();
//! assert!(host.is_required());
//! assert_eq!(host.description(), "Where to connect.");
//!
//! // The bounds come from the Rust width, which already stated them.
//! assert!(matches!(
//!     s.find("port").unwrap().kind(),
//!     Kind::Int { min: Some(0), max: Some(65535), .. }
//! ));
//!
//! // `Option<T>` is how a field says it may be left out.
//! assert!(!s.find("motd").unwrap().is_required());
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # The kind comes from the type, the field comes from the field
//!
//! A **kind** is a property of a type: `u16` is an integer between 0 and
//! 65535 wherever it appears, so [`Schema::kind`] is an associated
//! function with no `self` and no field to ask.
//!
//! Everything else — the key, whether it is required, its title, any
//! hint hung beside it — belongs to the *field*, not to the type, and is
//! read off the declaration by the derive. `Option<T>` is the one wrapper
//! with something to add: its kind is `T`'s, accepting null as well,
//! because `FromValue` reads a stored null as `None` -- in a field, in a
//! list, as a map's value -- and a schema must not refuse what the type
//! reads. Whether the field may be left out is still the field's
//! business: the derive omits `required`.

#![forbid(unsafe_code)]

mod own;

use super::build::KindBuilder;
use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::{Bytes, ToValue};
use crate::value::error::ValueError;
use crate::value::types::{Map, Text, Value};

/// A type that can describe itself to a consumer that has never seen it.
///
/// Derive it with `#[derive(Schema)]`; the impls below cover the types a
/// derived field is likely to hold.
pub trait Schema {
    /// The kind describing values of this type.
    fn kind(alloc: Alloc) -> KindBuilder;

    /// The whole schema: the fields a consumer fills in, as a root
    /// document with its dialect declared.
    ///
    /// Built from [`Schema::kind`], so the two cannot disagree: **every
    /// key the kind finished with is carried onto the document**, not
    /// only `properties` and `required`. A tagged enum's kind is its
    /// `x-variant-tag` and its `oneOf`, and a document that dropped those
    /// would describe an object with no fields — which is what
    /// `export_schema!` would then hand to C.
    ///
    /// The dialect and `type: "object"` are written first so they read
    /// first; a kind that names its own `type` replaces that one in
    /// place. `properties` is written even when the kind declares none,
    /// so a reader can tell an object that declares nothing from a
    /// document that forgot to say.
    ///
    /// [`schema`](Schema::schema) is the same through the crate's own
    /// allocator.
    fn schema_in(alloc: Alloc) -> Result<Value, ValueError> {
        let kind = Self::kind(alloc).finish()?;
        let mut out = Map::new_in(alloc);
        out.try_set(
            vocab::SCHEMA,
            Text::new_in(alloc, vocab::DIALECT).map(Value::from)?,
        )?;
        out.try_set(
            vocab::TYPE,
            Text::new_in(alloc, vocab::TYPE_OBJECT).map(Value::from)?,
        )?;
        for entry in <&Map>::try_from(&kind).map(Map::entries).unwrap_or(&[]) {
            out.try_set(entry.key(), entry.value().to_value_in(alloc)?)?;
        }
        if out.get(vocab::PROPERTIES).is_none() {
            out.try_set(vocab::PROPERTIES, Map::new_in(alloc))?;
        }
        Ok(out.into())
    }

    /// The whole schema, built through the crate's own allocator. See
    /// [`schema_in`](Schema::schema_in).
    fn schema() -> Result<Value, ValueError> {
        Self::schema_in(Alloc::rust())
    }
}

impl Schema for bool {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::bool_in(alloc)
    }
}

/// Every integer width, with the bounds the width already implies,
/// written as their exact digits: `u64::MAX` and `i128::MIN` too, which
/// [`Kind::Int`](super::read::Kind::Int) cannot hold and validation still
/// enforces.
macro_rules! integer_schema {
    ($($t:ty),* $(,)?) => {$(
        impl Schema for $t {
            fn kind(alloc: Alloc) -> KindBuilder {
                KindBuilder::int_in(alloc)
                    .number(vocab::MINIMUM, &<$t>::MIN.to_string())
                    .number(vocab::MAXIMUM, &<$t>::MAX.to_string())
            }
        }
    )*};
}

integer_schema!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl Schema for f64 {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::float_in(alloc)
    }
}

/// The magnitude from which a number has no finite `f32`: `f32::MAX`
/// plus half its last place, `2^128 - 2^103`, which rounds to infinity.
const F32_OVERFLOW: &str = "340282356779733661637539395458142568448";

/// A real number strictly between the two magnitudes past which `FromValue`
/// finds no finite `f32` to read.
impl Schema for f32 {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::float_in(alloc)
            .number(vocab::EXCLUSIVE_MINIMUM, &format!("-{F32_OVERFLOW}"))
            .number(vocab::EXCLUSIVE_MAXIMUM, F32_OVERFLOW)
    }
}

impl Schema for String {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc)
    }
}

impl Schema for Bytes {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::bytes_in(alloc)
    }
}

impl<T: Schema> Schema for Vec<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc))
    }
}

/// A map keyed by text describes an object whose **keys are data**: any
/// number of them, every value of one kind.
///
/// The schema says what the values are and nothing about the keys,
/// because there is nothing to say — a struct holding one of these is
/// exactly that, and a schema describes the struct as it is.
impl<T: Schema> Schema for alloc::collections::BTreeMap<String, T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, T::kind(alloc))
    }
}

/// The same. A `HashMap`'s iteration order is its own, which shows in the
/// value it writes and not in what it declares.
impl<T: Schema> Schema for std::collections::HashMap<String, T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, T::kind(alloc))
    }
}

/// An `Option<T>` describes what `T` describes, and null.
///
/// `FromValue` reads a stored null as `None` wherever the option sits, so
/// the kind says so: `type: [T, "null"]` for a kind its `type` describes,
/// `anyOf: [T, {"type": "null"}]` for an enumeration, a union or a
/// variant. Whether a field may be left out is still the field's
/// property, which the derive says by leaving it out of `required`.
impl<T: Schema> Schema for Option<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        T::kind(alloc).nullable()
    }
}
