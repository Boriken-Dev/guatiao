// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the sequences, the sets, a fixed-size array, a tuple, the unit
//! and a map keyed by data accept.

#![forbid(unsafe_code)]

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use std::collections::{HashMap, HashSet};

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::value::alloc::Alloc;
use crate::value::convert::MapKey;

impl<T: Schema> Schema for VecDeque<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc))
    }
}

/// A list with no element repeated: `uniqueItems`.
impl<T: Schema> Schema for BTreeSet<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc)).unique_items()
    }
}

/// The same as a `BTreeSet`.
impl<T: Schema, S> Schema for HashSet<T, S> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc)).unique_items()
    }
}

/// A list of exactly `N`: `minItems` and `maxItems` both `N`.
impl<T: Schema, const N: usize> Schema for [T; N] {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, T::kind(alloc))
            .min_items(N as u64)
            .max_items(N as u64)
    }
}

macro_rules! tuple {
    ($($t:ident),+) => {
        /// A tuple: `prefixItems`, one kind per position, and nothing past
        /// them.
        impl<$($t: Schema),+> Schema for ($($t,)+) {
            fn kind(alloc: Alloc) -> KindBuilder {
                KindBuilder::tuple_in(alloc, alloc::vec![$($t::kind(alloc)),+])
            }
        }
    };
}

tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);
tuple!(A, B, C, D, E, F);
tuple!(A, B, C, D, E, F, G);
tuple!(A, B, C, D, E, F, G, H);
tuple!(A, B, C, D, E, F, G, H, I);
tuple!(A, B, C, D, E, F, G, H, I, J);
tuple!(A, B, C, D, E, F, G, H, I, J, K);
tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

/// Null.
impl Schema for () {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::null_in(alloc)
    }
}

/// An object whose keys are data, every value of one kind, and every key
/// what [`MapKey::key_kind`] says (`propertyNames`) when it says anything.
fn keyed<K: MapKey, V: Schema>(alloc: Alloc) -> KindBuilder {
    let kind = KindBuilder::map_of_in(alloc, V::kind(alloc));
    match K::key_kind(alloc) {
        Some(names) => kind.property_names(names),
        None => kind,
    }
}

impl<K: MapKey, V: Schema> Schema for BTreeMap<K, V> {
    fn kind(alloc: Alloc) -> KindBuilder {
        keyed::<K, V>(alloc)
    }
}

/// The same. A `HashMap`'s iteration order is its own, which shows in the
/// value it writes and not in what it declares.
impl<K: MapKey, V: Schema, S> Schema for HashMap<K, V, S> {
    fn kind(alloc: Alloc) -> KindBuilder {
        keyed::<K, V>(alloc)
    }
}
