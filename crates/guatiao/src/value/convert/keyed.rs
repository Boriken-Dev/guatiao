// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Maps whose **keys are data**: a `BTreeMap` or `HashMap` keyed by
//! anything with a text form, written as an object keyed by that text.

#![forbid(unsafe_code)]

use super::Kinds;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::sync::Arc;
use core::hash::{BuildHasher, Hash};
use std::collections::HashMap;

use super::{FromValue, MapError, ToValue};
use crate::schema::build::KindBuilder;
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::types::map::Filling;
use crate::value::types::{Map, Value};

/// A type that can key a map: written as an object's key, as text, and
/// read back from it.
///
/// Implemented for `String`, `Box<str>`, `Arc<str>` and `Rc<str>`, every
/// integer width (decimal, no sign on an unsigned one, no leading zero),
/// `char`, `bool` (`true`/`false`), the IP and socket addresses and
/// `PathBuf`, so `BTreeMap<u32, T>` and `HashMap<IpAddr, T>` are field
/// types. A type of your own implements it to be one too.
pub trait MapKey: Sized {
    /// The key's text.
    ///
    /// # Errors
    ///
    /// [`ValueError::NotUtf8`] for a key with no text form, such as a
    /// path that is not UTF-8.
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError>;

    /// Reads a key back from its text.
    ///
    /// # Errors
    ///
    /// [`MapError::BadValue`] naming what a key of this type looks like.
    /// The reader adds the key it was found under.
    fn from_key(key: &str) -> Result<Self, MapError>;

    /// What every key looks like, as the schema of a string: written as
    /// the map's `propertyNames`. `None`, the default, for any text.
    fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
        let _ = alloc;
        None
    }
}

macro_rules! text_key {
    ($($t:ty),*) => {$(
        impl MapKey for $t {
            fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
                Ok(Cow::Borrowed(self))
            }

            fn from_key(key: &str) -> Result<$t, MapError> {
                Ok(<$t>::from(key))
            }
        }
    )*};
}

text_key!(String, Box<str>, Arc<str>, Rc<str>);

/// The digits an integer key is written with: an optional minus, then no
/// leading zero.
const SIGNED_KEY: &str = "^-?(0|[1-9][0-9]*)$";
const UNSIGNED_KEY: &str = "^(0|[1-9][0-9]*)$";

/// Whether `key` is an integer written plainly: what [`SIGNED_KEY`] says.
fn is_integer_key(key: &str) -> bool {
    let digits = key.strip_prefix('-').unwrap_or(key);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

macro_rules! integer_key {
    ($pattern:expr; $($t:ty),*) => {$(
        impl MapKey for $t {
            fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
                Ok(Cow::Owned(self.to_string()))
            }

            fn from_key(key: &str) -> Result<$t, MapError> {
                is_integer_key(key)
                    .then(|| key.parse::<$t>().ok())
                    .flatten()
                    .ok_or_else(|| {
                        MapError::bad_value(concat!("a key that is an integer a ", stringify!($t), " can hold"))
                    })
            }

            fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
                Some(KindBuilder::string_in(alloc).pattern($pattern))
            }
        }
    )*};
}

integer_key!(SIGNED_KEY; i8, i16, i32, i64, i128, isize);
integer_key!(UNSIGNED_KEY; u8, u16, u32, u64, u128, usize);

impl MapKey for char {
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
        Ok(Cow::Owned(self.to_string()))
    }

    fn from_key(key: &str) -> Result<char, MapError> {
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(MapError::bad_value("a key of exactly one character")),
        }
    }

    fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
        Some(KindBuilder::string_in(alloc).min_length(1).max_length(1))
    }
}

impl MapKey for bool {
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
        Ok(Cow::Borrowed(if *self { "true" } else { "false" }))
    }

    fn from_key(key: &str) -> Result<bool, MapError> {
        match key {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(MapError::bad_value("a key that is true or false")),
        }
    }

    fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
        Some(KindBuilder::enumeration_in(
            alloc,
            &[("false", ""), ("true", "")],
        ))
    }
}

/// Builds an object from key/value pairs, in the order they arrive.
fn entries_to_value<'a, K: MapKey + 'a, V: ToValue + 'a>(
    entries: impl Iterator<Item = (&'a K, &'a V)>,
    alloc: Alloc,
) -> Result<Value, ValueError> {
    // Two keys may write one text, so a key is looked for, not appended.
    let mut out = Filling::new(Map::new_in(alloc));
    for (key, value) in entries {
        out.set_in(&key.to_key()?, value.to_value_in(alloc)?, alloc)?;
    }
    Ok(out.into_map().into())
}

/// Reads every entry of an object, calling `insert` with each key and
/// value read; `insert` answers `false` for a key already read.
fn entries_from_value<K: MapKey, V: FromValue>(
    value: &Value,
    mut insert: impl FnMut(K, V) -> bool,
) -> Result<(), MapError> {
    for entry in <&Map>::try_from(value)? {
        let at = |e: MapError| e.at_key(entry.key());
        let key = K::from_key(entry.key()).map_err(at)?;
        let read = V::from_value(entry.value()).map_err(at)?;
        if !insert(key, read) {
            return Err(at(MapError::bad_value(
                "a key naming no entry an earlier key named",
            )));
        }
    }
    Ok(())
}

/// An object keyed by each key's text, in the map's order: sorted, so the
/// value it writes is the same every time.
impl<K: MapKey, V: ToValue> ToValue for BTreeMap<K, V> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        entries_to_value(self.iter(), alloc)
    }
}

/// The same, in whatever order the `HashMap` iterates, which the value
/// keeps because a map here is insertion-ordered: not reproducible from
/// one run to the next.
impl<K: MapKey, V: ToValue, S> ToValue for HashMap<K, V, S> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        entries_to_value(self.iter(), alloc)
    }
}

/// Every entry, each key read back from its text. An error inside one
/// reports `env[PATH]`; two keys reading as one key are refused rather
/// than one replacing the other.
impl<K: MapKey + Ord, V: FromValue> FromValue for BTreeMap<K, V> {
    const KINDS: Kinds = Kinds::MAP;
    fn from_value(value: &Value) -> Result<BTreeMap<K, V>, MapError> {
        let mut out = BTreeMap::new();
        entries_from_value(value, |k, v| out.insert(k, v).is_none())?;
        Ok(out)
    }
}

/// The same, into a `HashMap`, which keeps no order.
impl<K: MapKey + Eq + Hash, V: FromValue, S: BuildHasher + Default> FromValue for HashMap<K, V, S> {
    const KINDS: Kinds = Kinds::MAP;
    fn from_value(value: &Value) -> Result<HashMap<K, V, S>, MapError> {
        let mut out = HashMap::with_hasher(S::default());
        entries_from_value(value, |k, v| out.insert(k, v).is_none())?;
        Ok(out)
    }
}
