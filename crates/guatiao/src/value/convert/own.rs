// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! This crate's own containers as field types: written as the kind they
//! are, copied into the target allocator; read as a copy in this binary's
//! allocator, as [`Value`]'s own impl reads.

#![forbid(unsafe_code)]

use super::Kinds;
use super::{FromValue, MapError, ToValue, TryAsRef, copied};
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::types::{Buffer, List, Map, Number, Tag, Text, Value};

macro_rules! own_container {
    ($($ty:ident => $tag:ident),* $(,)?) => {$(
        impl ToValue for $ty {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                Ok(self.clone_in(alloc)?.into())
            }
        }

        /// The wrong kind is [`MapError::WrongType`]; content this build
        /// refuses to copy is [`MapError::BadValue`].
        impl FromValue for $ty {
            const KINDS: Kinds = Kinds::of(Tag::$tag);
            fn from_value(value: &Value) -> Result<$ty, MapError> {
                let found = TryAsRef::<$ty>::try_as_ref(value)
                    .ok_or_else(|| MapError::refused(Tag::$tag, value))?;
                copied(found.clone_in(Alloc::rust()))
            }
        }
    )*};
}

own_container!(
    Map => GUATIAO_MAP,
    List => GUATIAO_LIST,
    Text => GUATIAO_STRING,
    Number => GUATIAO_NUMBER,
    Buffer => GUATIAO_BYTES,
);
