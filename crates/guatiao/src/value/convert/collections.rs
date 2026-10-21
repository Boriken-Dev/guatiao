// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sequences beside `Vec`: a deque, the two sets, a fixed-size array and
//! a tuple, all lists; and the unit, a null.

#![forbid(unsafe_code)]

use alloc::collections::{BTreeSet, VecDeque};
use core::hash::{BuildHasher, Hash};
use std::collections::HashSet;

use super::{FromValue, MapError, ToValue};
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::types::{List, Tag, Value};

/// A list of every item, in the order they arrive.
fn list_of<'a, T: ToValue + 'a>(
    items: impl Iterator<Item = &'a T>,
    alloc: Alloc,
) -> Result<Value, ValueError> {
    let mut list = List::new_in(alloc);
    for item in items {
        list.push_in(item.to_value_in(alloc)?, alloc)?;
    }
    Ok(list.into())
}

/// Each element read under `[i]`, handed to `keep`, which answers `false`
/// for one equal to an element already kept.
fn read_each<T: FromValue>(value: &Value, mut keep: impl FnMut(T) -> bool) -> Result<(), MapError> {
    for (i, item) in <&List>::try_from(value)?.iter().enumerate() {
        let read = T::from_value(item).map_err(|e| e.at(i))?;
        if !keep(read) {
            return Err(MapError::bad_value("an element equal to no earlier one").at(i));
        }
    }
    Ok(())
}

impl<T: ToValue> ToValue for VecDeque<T> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        list_of(self.iter(), alloc)
    }
}

impl<T: FromValue> FromValue for VecDeque<T> {
    fn from_value(value: &Value) -> Result<VecDeque<T>, MapError> {
        let mut out = VecDeque::new();
        read_each(value, |v| {
            out.push_back(v);
            true
        })?;
        Ok(out)
    }
}

/// A list in the set's order: sorted.
impl<T: ToValue> ToValue for BTreeSet<T> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        list_of(self.iter(), alloc)
    }
}

/// A repeated element is refused rather than dropped, as the schema's
/// `uniqueItems` refuses it.
impl<T: FromValue + Ord> FromValue for BTreeSet<T> {
    fn from_value(value: &Value) -> Result<BTreeSet<T>, MapError> {
        let mut out = BTreeSet::new();
        read_each(value, |v| out.insert(v))?;
        Ok(out)
    }
}

/// A list in whatever order the set iterates: not reproducible from one
/// run to the next.
impl<T: ToValue, S> ToValue for HashSet<T, S> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        list_of(self.iter(), alloc)
    }
}

/// A repeated element is refused, as for a `BTreeSet`.
impl<T: FromValue + Eq + Hash, S: BuildHasher + Default> FromValue for HashSet<T, S> {
    fn from_value(value: &Value) -> Result<HashSet<T, S>, MapError> {
        let mut out = HashSet::with_hasher(S::default());
        read_each(value, |v| out.insert(v))?;
        Ok(out)
    }
}

impl<T: ToValue, const N: usize> ToValue for [T; N] {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        list_of(self.iter(), alloc)
    }
}

/// A list of exactly `N` elements.
impl<T: FromValue, const N: usize> FromValue for [T; N] {
    fn from_value(value: &Value) -> Result<[T; N], MapError> {
        let len = <&List>::try_from(value)?.len();
        if len != N {
            return Err(MapError::bad_value(format!("a list of exactly {N} items")));
        }
        let mut out = Vec::with_capacity(N);
        read_each(value, |v| {
            out.push(v);
            true
        })?;
        Ok(out
            .try_into()
            .ok()
            .expect("the length was checked against N"))
    }
}

macro_rules! tuple {
    ($n:literal; $($t:ident $i:tt),+) => {
        /// A list, one element per position.
        impl<$($t: ToValue),+> ToValue for ($($t,)+) {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                let mut list = List::new_in(alloc);
                $( list.push_in(self.$i.to_value_in(alloc)?, alloc)?; )+
                Ok(list.into())
            }
        }

        /// A list of exactly as many elements as positions, each read
        /// under `[i]`.
        impl<$($t: FromValue),+> FromValue for ($($t,)+) {
            fn from_value(value: &Value) -> Result<($($t,)+), MapError> {
                let items = &<&List>::try_from(value)?[..];
                if items.len() != $n {
                    return Err(MapError::bad_value(concat!(
                        "a list of exactly ", stringify!($n), " items"
                    )));
                }
                Ok(($( $t::from_value(&items[$i]).map_err(|e| e.at($i))?, )+))
            }
        }
    };
}

tuple!(1; A 0);
tuple!(2; A 0, B 1);
tuple!(3; A 0, B 1, C 2);
tuple!(4; A 0, B 1, C 2, D 3);
tuple!(5; A 0, B 1, C 2, D 3, E 4);
tuple!(6; A 0, B 1, C 2, D 3, E 4, F 5);
tuple!(7; A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple!(8; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
tuple!(9; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
tuple!(10; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
tuple!(11; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
tuple!(12; A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);

/// The unit carries nothing, and writes null.
impl ToValue for () {
    fn to_value_in(&self, _alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Value::null())
    }
}

/// Null, and only null.
impl FromValue for () {
    fn from_value(value: &Value) -> Result<(), MapError> {
        if value.tag() == Ok(Tag::GUATIAO_NULL) {
            Ok(())
        } else {
            Err(MapError::wrong_type(Tag::GUATIAO_NULL, value))
        }
    }
}
