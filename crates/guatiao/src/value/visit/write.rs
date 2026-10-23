// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Walk`] for what [`ToValue`] writes, each writing the events of the
//! tree `ToValue` builds.

#![forbid(unsafe_code)]

use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::number::Text;
use super::{ListVisitor, MapVisitor, Visitor, Walk};
use crate::value::convert::{Bytes, MapKey, ToValue};
use crate::value::error::ValueError;
use crate::value::secret::Secret;
use crate::value::span::Span;
use crate::value::time::{Date, DateTime, Time, Timestamp};

impl Walk for bool {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.bool(*self)
    }
}

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl Walk for $t {
            fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
                visitor.i64(i64::from(*self))
            }
        }
    )*};
}

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl Walk for $t {
            fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
                visitor.u64(u64::from(*self))
            }
        }
    )*};
}

signed!(i8, i16, i32, i64);
unsigned!(u8, u16, u32, u64);

impl Walk for isize {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        // `isize` is at most 64 bits on every target Rust supports.
        visitor.i64(*self as i64)
    }
}

impl Walk for usize {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.u64(*self as u64)
    }
}

impl Walk for i128 {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        match i64::try_from(*self) {
            Ok(n) => visitor.i64(n),
            Err(_) => visitor.number(Text::wide(*self).as_str()),
        }
    }
}

impl Walk for u128 {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        match u64::try_from(*self) {
            Ok(n) => visitor.u64(n),
            Err(_) => visitor.number(Text::wide_uint(*self).as_str()),
        }
    }
}

impl Walk for f64 {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.f64(*self)
    }
}

/// Its own shortest text, as [`ToValue`] writes it: `1.1f32` is `1.1`.
impl Walk for f32 {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        let text = Text::float32(*self).ok_or(ValueError::NotANumber)?;
        visitor.number(text.as_str())
    }
}

impl Walk for str {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.string(self)
    }
}

impl Walk for String {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.string(self)
    }
}

impl Walk for char {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.string(self.encode_utf8(&mut [0; 4]))
    }
}

impl Walk for Bytes {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.bytes(&self.0)
    }
}

impl Walk for () {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.null()
    }
}

/// `None` is null **here**; a derived struct omits the key instead.
impl<T: Walk> Walk for Option<T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        match self {
            Some(value) => value.walk(visitor),
            None => visitor.null(),
        }
    }
}

impl<T: Walk + ?Sized> Walk for &T {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        (**self).walk(visitor)
    }
}

macro_rules! pointer {
    ($($p:ident),*) => {$(
        impl<T: Walk + ?Sized> Walk for $p<T> {
            fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
                (**self).walk(visitor)
            }
        }
    )*};
}

pointer!(Box, Rc, Arc);

impl<T: Walk + ToOwned + ?Sized> Walk for Cow<'_, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        (**self).walk(visitor)
    }
}

impl<T: Walk> Walk for Secret<T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        self.expose().walk(visitor)
    }
}

/// A list of every item, in the order they arrive.
fn list_of<'a, T: Walk + 'a, V: Visitor>(
    items: impl ExactSizeIterator<Item = &'a T>,
    visitor: V,
) -> Result<V::Ok, V::Error> {
    let mut list = visitor.list(Some(items.len()))?;
    for item in items {
        list.item(item)?;
    }
    list.end()
}

impl<T: Walk> Walk for [T] {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

impl<T: Walk> Walk for Vec<T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

impl<T: Walk, const N: usize> Walk for [T; N] {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

impl<T: Walk> Walk for VecDeque<T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

impl<T: Walk> Walk for BTreeSet<T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

impl<T: Walk, S> Walk for HashSet<T, S> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        list_of(self.iter(), visitor)
    }
}

macro_rules! tuple {
    ($n:literal; $($t:ident $i:tt),+) => {
        impl<$($t: Walk),+> Walk for ($($t,)+) {
            fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
                let mut list = visitor.list(Some($n))?;
                $( list.item(&self.$i)?; )+
                list.end()
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

/// An object keyed by each key's text, in the order they arrive.
fn entries_of<'a, K: MapKey + 'a, T: Walk + 'a, V: Visitor>(
    entries: impl ExactSizeIterator<Item = (&'a K, &'a T)>,
    visitor: V,
) -> Result<V::Ok, V::Error> {
    let mut map = visitor.map(Some(entries.len()))?;
    for (key, value) in entries {
        map.entry(&key.to_key()?, value)?;
    }
    map.end()
}

impl<K: MapKey, T: Walk> Walk for BTreeMap<K, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        entries_of(self.iter(), visitor)
    }
}

impl<K: MapKey, T: Walk, S> Walk for HashMap<K, T, S> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        entries_of(self.iter(), visitor)
    }
}

/// Types whose tree is a leaf or two, walked as the tree [`ToValue`]
/// builds, so the two can never disagree about the text they write.
macro_rules! through_the_tree {
    ($($t:ty),* $(,)?) => {$(
        impl Walk for $t {
            fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
                self.to_value()?.walk(visitor)
            }
        }
    )*};
}

through_the_tree!(
    Date,
    Time,
    DateTime,
    Timestamp,
    SystemTime,
    Duration,
    Span,
    Path,
    PathBuf,
    IpAddr,
    Ipv4Addr,
    Ipv6Addr,
    SocketAddr,
    SocketAddrV4,
    SocketAddrV6,
    NonZeroI8,
    NonZeroI16,
    NonZeroI32,
    NonZeroI64,
    NonZeroI128,
    NonZeroIsize,
    NonZeroU8,
    NonZeroU16,
    NonZeroU32,
    NonZeroU64,
    NonZeroU128,
    NonZeroUsize,
);
