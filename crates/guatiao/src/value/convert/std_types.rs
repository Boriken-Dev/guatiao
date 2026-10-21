// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Paths, network addresses and non-zero integers: each crosses as the
//! text or number a person writes for it.

#![forbid(unsafe_code)]

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::sync::Arc;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
};
use std::path::{Path, PathBuf};

use super::{FromValue, MapError, MapKey, ToValue};
use crate::schema::build::KindBuilder;
use crate::schema::describe::Schema;
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::types::{Text, Value};

// --- paths ---------------------------------------------------------------

/// The path's text. One that is not UTF-8 is refused with
/// [`ValueError::NotUtf8`] rather than written with replacement
/// characters, which would name another path.
impl ToValue for Path {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let text = self.to_str().ok_or(ValueError::NotUtf8)?;
        Ok(Text::new_in(alloc, text)?.into())
    }
}

impl ToValue for PathBuf {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.as_path().to_value_in(alloc)
    }
}

impl FromValue for PathBuf {
    fn from_value(value: &Value) -> Result<PathBuf, MapError> {
        Ok(PathBuf::from(<&str>::try_from(value)?))
    }
}

macro_rules! shared_path {
    ($($p:ident),*) => {$(
        impl FromValue for $p<Path> {
            fn from_value(value: &Value) -> Result<$p<Path>, MapError> {
                PathBuf::from_value(value).map($p::from)
            }
        }
    )*};
}

shared_path!(Box, Arc, Rc);

impl MapKey for PathBuf {
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
        self.to_str().map(Cow::Borrowed).ok_or(ValueError::NotUtf8)
    }

    fn from_key(key: &str) -> Result<PathBuf, MapError> {
        Ok(PathBuf::from(key))
    }
}

// --- addresses -------------------------------------------------------------

macro_rules! address {
    ($($t:ty => $what:literal),* $(,)?) => {$(
        /// Its `Display` text.
        impl ToValue for $t {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                Ok(Text::new_in(alloc, &self.to_string())?.into())
            }
        }

        /// Text its `FromStr` reads.
        impl FromValue for $t {
            fn from_value(value: &Value) -> Result<$t, MapError> {
                <&str>::try_from(value)?
                    .parse()
                    .map_err(|_| MapError::bad_value($what))
            }
        }

        impl MapKey for $t {
            fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
                Ok(Cow::Owned(self.to_string()))
            }

            fn from_key(key: &str) -> Result<$t, MapError> {
                key.parse()
                    .map_err(|_| MapError::bad_value(concat!("a key that is ", $what)))
            }

            fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
                Some(<$t as Schema>::kind(alloc))
            }
        }
    )*};
}

address!(
    IpAddr => "an IPv4 or IPv6 address",
    Ipv4Addr => "an IPv4 address",
    Ipv6Addr => "an IPv6 address",
    SocketAddr => "an IPv4 or IPv6 socket address",
    SocketAddrV4 => "an IPv4 socket address, address:port",
    SocketAddrV6 => "an IPv6 socket address, [address]:port",
);

// --- non-zero integers -------------------------------------------------------

macro_rules! non_zero {
    ($($nz:ty => $t:ty),* $(,)?) => {$(
        impl ToValue for $nz {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                self.get().to_value_in(alloc)
            }
        }

        /// An integer of the width, other than zero.
        impl FromValue for $nz {
            fn from_value(value: &Value) -> Result<$nz, MapError> {
                <$nz>::new(<$t>::from_value(value)?)
                    .ok_or_else(|| MapError::bad_value("an integer other than 0"))
            }
        }
    )*};
}

non_zero!(
    NonZeroI8 => i8, NonZeroI16 => i16, NonZeroI32 => i32, NonZeroI64 => i64,
    NonZeroI128 => i128, NonZeroIsize => isize, NonZeroU8 => u8, NonZeroU16 => u16,
    NonZeroU32 => u32, NonZeroU64 => u64, NonZeroU128 => u128, NonZeroUsize => usize,
);
