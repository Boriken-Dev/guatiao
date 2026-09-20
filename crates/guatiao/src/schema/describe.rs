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
//! let declared = Connection::schema(alloc)?;
//! let s = SchemaRef::new(&declared).unwrap();
//!
//! let host = s.find("host").unwrap();
//! assert!(host.is_required());
//! assert_eq!(host.help(), "Where to connect.");
//!
//! // The bounds come from the Rust width, which already stated them.
//! assert!(matches!(
//!     s.find("port").unwrap().kind(),
//!     Kind::Int { min: Some(0), max: Some(65535) }
//! ));
//!
//! // `Option<T>` is how a field says it may be left out.
//! assert!(!s.find("motd").unwrap().is_required());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # The kind comes from the type, the field comes from the field
//!
//! A **kind** is a property of a type: `u16` is an integer between 0 and
//! 65535 wherever it appears, so [`Schema::kind`] is an associated
//! function with no `self` and no field to ask.
//!
//! Everything else — the key, whether it is required, its label, its
//! section — belongs to the *field*, not to the type, and is read off the
//! declaration by the derive. That split is why `Option<T>` contributes no
//! kind of its own: optionality is the field's business, so
//! `Option<T>::kind` is `T::kind` and the derive omits `required` instead.

#![forbid(unsafe_code)]

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
    fn schema(alloc: Alloc) -> Result<Value, ValueError> {
        let kind = Self::kind(alloc).finish()?;
        let mut out = Map::new_in(alloc);
        out.set(
            vocab::SCHEMA,
            Text::new_in(alloc, vocab::DIALECT).map(Value::from)?,
        )?;
        out.set(
            vocab::TYPE,
            Text::new_in(alloc, vocab::TYPE_OBJECT).map(Value::from)?,
        )?;
        for entry in <&Map>::try_from(&kind).map(Map::entries).unwrap_or(&[]) {
            let key = entry.key_str().ok_or(ValueError::NotUtf8)?;
            out.set(key, entry.value().to_value(alloc)?)?;
        }
        if out.get(vocab::PROPERTIES).is_none() {
            out.set(vocab::PROPERTIES, Map::new_in(alloc))?;
        }
        Ok(out.into())
    }
}

impl Schema for bool {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::bool_in(alloc)
    }
}

/// Every integer width, with the bounds the width already implies.
///
/// A bound that does not fit an `i64` is **left off** rather than clamped:
/// a clamped bound enforces a limit nobody declared, and `u64` genuinely
/// has no upper bound this vocabulary can write down.
macro_rules! integer_schema {
    ($($t:ty),* $(,)?) => {$(
        impl Schema for $t {
            fn kind(alloc: Alloc) -> KindBuilder {
                KindBuilder::int_bounds_in(alloc,
                    i64::try_from(<$t>::MIN).ok(),
                    i64::try_from(<$t>::MAX).ok(),
                )
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

/// The same kind as [`f64`]: the vocabulary has one real-number type, and
/// a narrower Rust width is not a bound this schema can write down.
impl Schema for f32 {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::float_in(alloc)
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

/// An `Option<T>` describes exactly what `T` describes.
///
/// Optionality is the **field's** property, not the type's: a schema says
/// so with `required`, which the derive leaves off, and a kind that tried
/// to say it as well would let the two disagree.
impl<T: Schema> Schema for Option<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        T::kind(alloc)
    }
}
