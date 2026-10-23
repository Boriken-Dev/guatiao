// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Smart pointers, `Cow` and [`Secret`] cross as what they hold,
//! transparently; a `char` crosses as text one character long.

#![forbid(unsafe_code)]

use super::Kinds;
use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;

use super::{FromValue, MapError, ToValue, TryAsRef};
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::secret::Secret;
use crate::value::types::{Tag, Text, Value};

macro_rules! pointer {
    ($($p:ident),*) => {$(
        impl<T: ToValue + ?Sized> ToValue for $p<T> {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                (**self).to_value_in(alloc)
            }
        }

        impl<T: FromValue> FromValue for $p<T> {
            const KINDS: Kinds = T::KINDS;
            fn from_value(value: &Value) -> Result<$p<T>, MapError> {
                T::from_value(value).map($p::new)
            }
        }

        impl FromValue for $p<str> {
            const KINDS: Kinds = Kinds::STRING;
            fn from_value(value: &Value) -> Result<$p<str>, MapError> {
                Ok($p::from(<&str>::try_from(value)?))
            }
        }

        impl<T: FromValue> FromValue for $p<[T]> {
            const KINDS: Kinds = Kinds::LIST;
            fn from_value(value: &Value) -> Result<$p<[T]>, MapError> {
                Vec::<T>::from_value(value).map($p::from)
            }
        }
    )*};
}

pointer!(Box, Arc, Rc);

impl<T: ToValue + ToOwned + ?Sized> ToValue for Cow<'_, T> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        (**self).to_value_in(alloc)
    }
}

/// Always [`Cow::Owned`]: a reader copies out of the value, and nothing it
/// returns can borrow from it.
impl<T: ToOwned + ?Sized> FromValue for Cow<'_, T>
where
    T::Owned: FromValue,
{
    const KINDS: Kinds = <T::Owned as FromValue>::KINDS;
    fn from_value(value: &Value) -> Result<Self, MapError> {
        T::Owned::from_value(value).map(Cow::Owned)
    }
}

/// The value it holds, as `T` writes it.
impl<T: ToValue> ToValue for Secret<T> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.expose().to_value_in(alloc)
    }
}

impl<T: FromValue> FromValue for Secret<T> {
    const KINDS: Kinds = T::KINDS;
    fn from_value(value: &Value) -> Result<Secret<T>, MapError> {
        T::from_value(value).map(Secret::new)
    }
}

impl ToValue for char {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut buf = [0u8; 4];
        Ok(Text::new_in(alloc, self.encode_utf8(&mut buf))?.into())
    }
}

/// Text of exactly one character, counted in Unicode code points as JSON
/// Schema counts a length.
impl FromValue for char {
    const KINDS: Kinds = Kinds::STRING;
    fn from_value(value: &Value) -> Result<char, MapError> {
        let text = TryAsRef::<str>::try_as_ref(value)
            .ok_or_else(|| MapError::refused(Tag::GUATIAO_STRING, value))?;
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(MapError::bad_value("text of exactly one character")),
        }
    }
}
