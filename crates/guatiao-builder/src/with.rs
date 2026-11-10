// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `with_module!`: a guatiao `with` module for a described type.

/// Writes a module `$name` for `#[map(with = $name)]`, serving a field of the described type
/// `$ty` inside a struct that derives guatiao's `ToValue`, `FromValue` and `Schema`:
///
/// ```
/// use coqui_builder::Describe;
/// use guatiao::{FromValue, Schema, ToValue};
///
/// #[derive(Describe)]
/// struct Listener {
///     host: String,
/// }
///
/// guatiao_builder::with_module!(pub listener, Listener);
///
/// #[derive(ToValue, FromValue, Schema)]
/// struct Host {
///     #[map(with = listener)]
///     listener: Listener,
/// }
/// # fn main() {}
/// ```
///
/// Call it at module level: the generated module names `$ty` through `use super::*`.
///
/// A `with` module's `kind` takes no type parameter, so one generic module cannot serve every
/// type; this writes one per type. `Described<T>` serves a type that is a whole value or a
/// kind's argument.
#[macro_export]
macro_rules! with_module {
    ($vis:vis $name:ident, $ty:ty) => {
        $vis mod $name {
            #[allow(unused_imports)]
            use super::*;

            /// The field as a guatiao value.
            pub fn to_value_in(
                value: &$ty,
                alloc: $crate::__private::Alloc,
            ) -> ::core::result::Result<$crate::__private::Value, $crate::__private::ValueError> {
                $crate::to_value_in(value, alloc)
            }

            /// The field read from a guatiao value.
            pub fn from_value(
                value: &$crate::__private::Value,
            ) -> ::core::result::Result<$ty, $crate::__private::MapError> {
                $crate::__private::from_value::<$ty>(value)
            }

            /// The field's schema, made from its description.
            pub fn kind(alloc: $crate::__private::Alloc) -> $crate::__private::KindBuilder {
                $crate::kind_in(&<$ty as $crate::__private::Describe>::description(), alloc)
            }
        }
    };
}
