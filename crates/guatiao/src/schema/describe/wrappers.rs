// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A pointer, a borrow or a `Cow` describes what it holds; `str` and a
//! slice describe what `String` and `Vec` do, so a pointer to either
//! describes the same; a `char` is text one character long; a
//! [`Secret`] is what it holds, marked.

#![forbid(unsafe_code)]

use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::schema::vocab;
use crate::value::alloc::Alloc;
use crate::value::secret::Secret;
use crate::value::types::Value;

impl Schema for str {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc)
    }
}

impl<T: Schema> Schema for [T] {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc))
    }
}

macro_rules! pointer {
    ($($p:ident),*) => {$(
        impl<T: Schema + ?Sized> Schema for $p<T> {
            fn kind(alloc: Alloc) -> KindBuilder {
                T::kind(alloc)
            }
        }
    )*};
}

pointer!(Box, Arc, Rc);

impl<T: Schema + ?Sized> Schema for &T {
    fn kind(alloc: Alloc) -> KindBuilder {
        T::kind(alloc)
    }
}

impl<T: Schema + ToOwned + ?Sized> Schema for Cow<'_, T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        T::kind(alloc)
    }
}

/// `T`'s kind with `writeOnly: true` and `x-sensitive: true`, the two
/// marks `FieldBuilder::sensitive` writes, which redaction honours.
impl<T: Schema> Schema for Secret<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        T::kind(alloc)
            .keyword(vocab::WRITE_ONLY, Ok(Value::from(true)))
            .keyword(vocab::X_SENSITIVE, Ok(Value::from(true)))
    }
}

/// One Unicode code point: `minLength` and `maxLength` 1.
impl Schema for char {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).min_length(1).max_length(1)
    }
}
