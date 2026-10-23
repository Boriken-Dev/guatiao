// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`FromWalk`] for what [`FromValue`] reads, refusing what it refuses
//! with the same errors.

#![forbid(unsafe_code)]

use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use core::hash::{BuildHasher, Hash};
use core::marker::PhantomData;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use super::replay::{NoWalker, Replay};
use super::{Build, FromWalk, ListWalker, MapWalker, Walker};
use crate::value::convert::{Bytes, FromValue, Kinds, MapError, MapKey};
use crate::value::secret::Secret;
use crate::value::span::Span;
use crate::value::time::{Date, DateTime, Time, Timestamp};
use crate::value::types::Value;

/// How many elements a length hint may reserve room for: a hint comes
/// from the input, and a count is a bound to check, not a size to trust.
const RESERVE: usize = 1024;

// --- scalars -----------------------------------------------------------------

struct BoolBuild;

impl Build for BoolBuild {
    type Out = bool;

    fn kinds(&self) -> Kinds {
        Kinds::BOOL
    }

    fn bool(self, value: bool) -> Result<bool, MapError> {
        Ok(value)
    }
}

impl FromWalk for bool {
    fn from_walk<W: Walker>(walker: W) -> Result<bool, MapError> {
        walker.walk(BoolBuild)
    }
}

/// An integer read never truncates: a fractional or exponent spelling
/// is refused rather than rounded, as `FromValue` refuses it.
struct IntBuild<T>(PhantomData<T>);

macro_rules! integer {
    ($($t:ty),*) => {$(
        impl Build for IntBuild<$t> {
            type Out = $t;

            fn kinds(&self) -> Kinds {
                Kinds::NUMBER
            }

            fn number(self, text: &str) -> Result<$t, MapError> {
                if text.contains(['.', 'e', 'E']) {
                    return Err(MapError::bad_value(concat!(
                        "an integer written plainly -- a fractional or exponent \
                         spelling is refused rather than rounded into a ",
                        stringify!($t),
                    )));
                }
                text.parse::<$t>().map_err(|_| IntBuild::<$t>::range())
            }

            fn i64(self, value: i64) -> Result<$t, MapError> {
                <$t>::try_from(value).map_err(|_| IntBuild::<$t>::range())
            }

            fn u64(self, value: u64) -> Result<$t, MapError> {
                <$t>::try_from(value).map_err(|_| IntBuild::<$t>::range())
            }
        }

        impl IntBuild<$t> {
            fn range() -> MapError {
                MapError::bad_value(concat!("an integer a ", stringify!($t), " can hold"))
            }
        }

        impl FromWalk for $t {
            fn from_walk<W: Walker>(walker: W) -> Result<$t, MapError> {
                walker.walk(IntBuild::<$t>(PhantomData))
            }
        }
    )*};
}

integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

struct F64Build;

impl Build for F64Build {
    type Out = f64;

    fn kinds(&self) -> Kinds {
        Kinds::NUMBER
    }

    fn number(self, text: &str) -> Result<f64, MapError> {
        text.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| MapError::bad_value("a number with a finite f64 value"))
    }

    // The float itself, which is what its shortest text reads back as.
    fn f64(self, value: f64) -> Result<f64, MapError> {
        if value.is_finite() {
            Ok(value)
        } else {
            Err(MapError::bad_value("a number with a finite f64 value"))
        }
    }
}

impl FromWalk for f64 {
    fn from_walk<W: Walker>(walker: W) -> Result<f64, MapError> {
        walker.walk(F64Build)
    }
}

struct F32Build;

impl Build for F32Build {
    type Out = f32;

    fn kinds(&self) -> Kinds {
        Kinds::NUMBER
    }

    fn number(self, text: &str) -> Result<f32, MapError> {
        text.parse::<f32>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| MapError::bad_value("a number with a finite f32 value"))
    }
}

impl FromWalk for f32 {
    fn from_walk<W: Walker>(walker: W) -> Result<f32, MapError> {
        walker.walk(F32Build)
    }
}

struct StringBuild;

impl Build for StringBuild {
    type Out = String;

    fn kinds(&self) -> Kinds {
        Kinds::STRING
    }

    fn string(self, text: &str) -> Result<String, MapError> {
        Ok(String::from(text))
    }
}

impl FromWalk for String {
    fn from_walk<W: Walker>(walker: W) -> Result<String, MapError> {
        walker.walk(StringBuild)
    }
}

struct CharBuild;

impl Build for CharBuild {
    type Out = char;

    fn kinds(&self) -> Kinds {
        Kinds::STRING
    }

    fn string(self, text: &str) -> Result<char, MapError> {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(MapError::bad_value("text of exactly one character")),
        }
    }
}

impl FromWalk for char {
    fn from_walk<W: Walker>(walker: W) -> Result<char, MapError> {
        walker.walk(CharBuild)
    }
}

struct BytesBuild;

impl Build for BytesBuild {
    type Out = Bytes;

    fn kinds(&self) -> Kinds {
        Kinds::BYTES
    }

    fn bytes(self, bytes: &[u8]) -> Result<Bytes, MapError> {
        Ok(Bytes(bytes.to_vec()))
    }
}

impl FromWalk for Bytes {
    fn from_walk<W: Walker>(walker: W) -> Result<Bytes, MapError> {
        walker.walk(BytesBuild)
    }
}

struct UnitBuild;

impl Build for UnitBuild {
    type Out = ();

    fn kinds(&self) -> Kinds {
        Kinds::NULL
    }

    fn null(self) -> Result<(), MapError> {
        Ok(())
    }
}

impl FromWalk for () {
    fn from_walk<W: Walker>(walker: W) -> Result<(), MapError> {
        walker.walk(UnitBuild)
    }
}

// --- wrappers ----------------------------------------------------------------

/// A stored null reads as `None`; anything else is offered to `T`.
struct OptionBuild<T>(PhantomData<T>);

/// The scalar events of a builder that hands each one, replayed, to
/// `$read` and maps the result with `$wrap`.
macro_rules! replaying {
    ($read:path, $wrap:expr) => {
        fn absent(self) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::Absent).map($wrap)
        }

        fn bool(self, value: bool) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::Bool(value)).map($wrap)
        }

        fn number(self, text: &str) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::Number(text)).map($wrap)
        }

        fn i64(self, value: i64) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::I64(value)).map($wrap)
        }

        fn u64(self, value: u64) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::U64(value)).map($wrap)
        }

        fn f64(self, value: f64) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::F64(value)).map($wrap)
        }

        fn string(self, text: &str) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::String(text)).map($wrap)
        }

        fn bytes(self, bytes: &[u8]) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, NoWalker>::Bytes(bytes)).map($wrap)
        }

        fn list<L: ListWalker>(self, items: L) -> Result<Self::Out, MapError> {
            $read(Replay::<L, NoWalker>::List(items)).map($wrap)
        }

        fn map<M: MapWalker>(self, entries: M) -> Result<Self::Out, MapError> {
            $read(Replay::<NoWalker, M>::Map(entries)).map($wrap)
        }
    };
}

impl<T: FromWalk> Build for OptionBuild<T> {
    type Out = Option<T>;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn null(self) -> Result<Option<T>, MapError> {
        Ok(None)
    }

    replaying!(T::from_walk, Some);
}

impl<T: FromWalk> FromWalk for Option<T> {
    fn from_walk<W: Walker>(walker: W) -> Result<Option<T>, MapError> {
        walker.walk(OptionBuild(PhantomData))
    }
}

macro_rules! pointer {
    ($($p:ident),*) => {$(
        impl<T: FromWalk> FromWalk for $p<T> {
            fn from_walk<W: Walker>(walker: W) -> Result<$p<T>, MapError> {
                T::from_walk(walker).map($p::new)
            }
        }

        impl FromWalk for $p<str> {
            fn from_walk<W: Walker>(walker: W) -> Result<$p<str>, MapError> {
                String::from_walk(walker).map($p::from)
            }
        }

        impl<T: FromWalk> FromWalk for $p<[T]> {
            fn from_walk<W: Walker>(walker: W) -> Result<$p<[T]>, MapError> {
                Vec::<T>::from_walk(walker).map($p::from)
            }
        }
    )*};
}

pointer!(Box, Arc, Rc);

/// Always [`Cow::Owned`], as `FromValue` reads it.
impl<T: ToOwned + ?Sized> FromWalk for Cow<'_, T>
where
    T::Owned: FromWalk,
{
    fn from_walk<W: Walker>(walker: W) -> Result<Self, MapError> {
        T::Owned::from_walk(walker).map(Cow::Owned)
    }
}

impl<T: FromWalk> FromWalk for Secret<T> {
    fn from_walk<W: Walker>(walker: W) -> Result<Secret<T>, MapError> {
        T::from_walk(walker).map(Secret::new)
    }
}

// --- sequences ---------------------------------------------------------------

/// A list's elements, each read under `[i]` and handed to `keep`, which
/// answers `false` for one equal to an element already kept.
struct EachBuild<T, K>(K, PhantomData<T>);

impl<T: FromWalk, K: FnMut(T, Option<usize>) -> bool> Build for EachBuild<T, K> {
    type Out = ();

    fn kinds(&self) -> Kinds {
        Kinds::LIST
    }

    fn list<L: ListWalker>(mut self, mut items: L) -> Result<(), MapError> {
        let mut i = 0;
        while let Some(item) = items.next::<T>().map_err(|e| e.at(i))? {
            if !(self.0)(item, items.len_hint()) {
                return Err(MapError::bad_value("an element equal to no earlier one").at(i));
            }
            i += 1;
        }
        Ok(())
    }
}

/// Reads every element of a list into `keep`.
fn each<T: FromWalk, W: Walker>(
    walker: W,
    keep: impl FnMut(T, Option<usize>) -> bool,
) -> Result<(), MapError> {
    walker.walk(EachBuild(keep, PhantomData))
}

/// Room for the elements a hint promises, bounded.
fn room(hint: Option<usize>) -> usize {
    hint.unwrap_or(0).min(RESERVE)
}

/// Each element is read under `[i]`, so a failure names which one.
impl<T: FromWalk> FromWalk for Vec<T> {
    fn from_walk<W: Walker>(walker: W) -> Result<Vec<T>, MapError> {
        let mut out = Vec::new();
        each(walker, |item, left| {
            if out.is_empty() {
                out.reserve(room(left).saturating_add(1));
            }
            out.push(item);
            true
        })?;
        Ok(out)
    }
}

impl<T: FromWalk> FromWalk for VecDeque<T> {
    fn from_walk<W: Walker>(walker: W) -> Result<VecDeque<T>, MapError> {
        let mut out = VecDeque::new();
        each(walker, |item, _| {
            out.push_back(item);
            true
        })?;
        Ok(out)
    }
}

/// A repeated element is refused rather than dropped.
impl<T: FromWalk + Ord> FromWalk for BTreeSet<T> {
    fn from_walk<W: Walker>(walker: W) -> Result<BTreeSet<T>, MapError> {
        let mut out = BTreeSet::new();
        each(walker, |item, _| out.insert(item))?;
        Ok(out)
    }
}

impl<T: FromWalk + Eq + Hash, S: BuildHasher + Default> FromWalk for HashSet<T, S> {
    fn from_walk<W: Walker>(walker: W) -> Result<HashSet<T, S>, MapError> {
        let mut out = HashSet::with_hasher(S::default());
        each(walker, |item, _| out.insert(item))?;
        Ok(out)
    }
}

/// A list of exactly `N` elements.
impl<T: FromWalk, const N: usize> FromWalk for [T; N] {
    fn from_walk<W: Walker>(walker: W) -> Result<[T; N], MapError> {
        let exactly = || MapError::bad_value(alloc::format!("a list of exactly {N} items"));
        let mut out = Vec::with_capacity(N.min(RESERVE));
        let mut over = false;
        each(walker, |item, _| {
            if out.len() == N {
                over = true;
            } else {
                out.push(item);
            }
            true
        })?;
        if over {
            return Err(exactly());
        }
        out.try_into().map_err(|_| exactly())
    }
}

/// A list of exactly as many elements as positions, each read under `[i]`.
struct TupleBuild<T>(PhantomData<T>);

macro_rules! tuple {
    ($n:literal; $($t:ident $i:tt),+) => {
        impl<$($t: FromWalk),+> Build for TupleBuild<($($t,)+)> {
            type Out = ($($t,)+);

            fn kinds(&self) -> Kinds {
                Kinds::LIST
            }

            fn list<__L: ListWalker>(self, mut items: __L) -> Result<Self::Out, MapError> {
                let exactly = || MapError::bad_value(concat!(
                    "a list of exactly ", stringify!($n), " items"
                ));
                if items.len_hint().is_some_and(|n| n != $n) {
                    return Err(exactly());
                }
                let out = ($(
                    items.next::<$t>().map_err(|e| e.at($i))?.ok_or_else(exactly)?,
                )+);
                if items.next::<Ignored>()?.is_some() {
                    return Err(exactly());
                }
                Ok(out)
            }
        }

        impl<$($t: FromWalk),+> FromWalk for ($($t,)+) {
            fn from_walk<W: Walker>(walker: W) -> Result<Self, MapError> {
                walker.walk(TupleBuild::<($($t,)+)>(PhantomData))
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

// --- maps whose keys are data --------------------------------------------------

/// A key's own text, to name it in an error.
fn named<K: MapKey>(e: MapError, key: &K) -> MapError {
    match key.to_key() {
        Ok(text) => e.at_key(&text),
        Err(_) => e,
    }
}

/// What two keys reading as one key are refused with.
fn repeated<K: MapKey>(key: &K) -> MapError {
    named(
        MapError::bad_value("a key naming no entry an earlier key named"),
        key,
    )
}

/// Every entry, each key read back from its text, handed to `insert`,
/// which refuses a key already read. An error inside one names it
/// `[key]`.
struct EntriesBuild<K, T, I>(I, PhantomData<(K, T)>);

impl<K: MapKey, T: FromWalk, I: FnMut(K, T) -> Result<(), MapError>> Build
    for EntriesBuild<K, T, I>
{
    type Out = ();

    fn kinds(&self) -> Kinds {
        Kinds::MAP
    }

    fn map<M: MapWalker>(mut self, mut entries: M) -> Result<(), MapError> {
        while let Some(key) =
            entries.next_key(|text| K::from_key(text).map_err(|e| e.at_key(text)))?
        {
            let key = key?;
            let value = entries.value::<T>().map_err(|e| named(e, &key))?;
            (self.0)(key, value)?;
        }
        Ok(())
    }
}

impl<K: MapKey + Ord, T: FromWalk> FromWalk for BTreeMap<K, T> {
    fn from_walk<W: Walker>(walker: W) -> Result<BTreeMap<K, T>, MapError> {
        use alloc::collections::btree_map::Entry;
        let mut out = BTreeMap::new();
        walker.walk(EntriesBuild(
            |key: K, value| match out.entry(key) {
                Entry::Occupied(taken) => Err(repeated(taken.key())),
                Entry::Vacant(room) => {
                    room.insert(value);
                    Ok(())
                }
            },
            PhantomData,
        ))?;
        Ok(out)
    }
}

impl<K: MapKey + Eq + Hash, T: FromWalk, S: BuildHasher + Default> FromWalk for HashMap<K, T, S> {
    fn from_walk<W: Walker>(walker: W) -> Result<HashMap<K, T, S>, MapError> {
        use std::collections::hash_map::Entry;
        let mut out = HashMap::with_hasher(S::default());
        walker.walk(EntriesBuild(
            |key: K, value| match out.entry(key) {
                Entry::Occupied(taken) => Err(repeated(taken.key())),
                Entry::Vacant(room) => {
                    room.insert(value);
                    Ok(())
                }
            },
            PhantomData,
        ))?;
        Ok(out)
    }
}

// --- through the tree -----------------------------------------------------------

/// Types read as `FromValue` reads the small tree they are stored as, so
/// the two readers can never disagree.
macro_rules! through_the_tree {
    ($($t:ty),* $(,)?) => {$(
        impl FromWalk for $t {
            fn from_walk<W: Walker>(walker: W) -> Result<$t, MapError> {
                <$t as FromValue>::from_value(&Value::from_walk(walker)?)
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

// --- anything, read and dropped ---------------------------------------------------

/// Takes any value and keeps nothing: what passing over a value reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ignored;

struct IgnoredBuild;

impl Build for IgnoredBuild {
    type Out = Ignored;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn absent(self) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn null(self) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn bool(self, _: bool) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn number(self, _: &str) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn i64(self, _: i64) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn u64(self, _: u64) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn f64(self, _: f64) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn string(self, _: &str) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn bytes(self, _: &[u8]) -> Result<Ignored, MapError> {
        Ok(Ignored)
    }

    fn list<L: ListWalker>(self, mut items: L) -> Result<Ignored, MapError> {
        while items.next::<Ignored>()?.is_some() {}
        Ok(Ignored)
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<Ignored, MapError> {
        while entries.next_key(|_| ())?.is_some() {
            entries.skip_value()?;
        }
        Ok(Ignored)
    }
}

impl FromWalk for Ignored {
    fn from_walk<W: Walker>(walker: W) -> Result<Ignored, MapError> {
        walker.walk(IgnoredBuild)
    }
}
