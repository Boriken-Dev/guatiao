// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What paths, addresses, non-zero integers and `SystemTime` accept: text
//! with the `format` JSON Schema names for it, or a pattern where it names
//! none, and an integer that is not zero.

#![forbid(unsafe_code)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::schema::vocab;
use crate::value::alloc::Alloc;
use crate::value::types::{List, Map, Number, Text, Value};

/// A path is text; nothing narrower holds on every platform.
impl Schema for Path {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc)
    }
}

impl Schema for PathBuf {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc)
    }
}

/// `format: ipv4`, JSON Schema's name for RFC 2673's dotted quad.
impl Schema for Ipv4Addr {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("ipv4")
    }
}

/// `format: ipv6`, RFC 4291's text form.
impl Schema for Ipv6Addr {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("ipv6")
    }
}

/// Either: `anyOf` the two formats, beside `type: string`.
impl Schema for IpAddr {
    fn kind(alloc: Alloc) -> KindBuilder {
        let arm = |format: &str| {
            let mut m = Map::new_in(alloc);
            m.try_set(vocab::FORMAT, Text::new_in(alloc, format)?)?;
            Ok(Value::from(m))
        };
        let mut arms = List::new_in(alloc);
        let built = arm("ipv4")
            .and_then(|a| arms.push_in(a, alloc))
            .and_then(|()| arm("ipv6").and_then(|a| arms.push_in(a, alloc)));
        KindBuilder::string_in(alloc).keyword(vocab::ANY_OF, built.map(|()| arms.into()))
    }
}

/// `address:port`, as a pattern: JSON Schema names no format for it.
const SOCKET_V4: &str = r"^([0-9]{1,3}\.){3}[0-9]{1,3}:[0-9]{1,5}$";
/// `[address]:port`, with an optional numeric `%scope`.
const SOCKET_V6: &str = r"^\[[0-9A-Fa-f:.]+(%[0-9]+)?\]:[0-9]{1,5}$";
/// Either.
const SOCKET: &str = r"^(([0-9]{1,3}\.){3}[0-9]{1,3}|\[[0-9A-Fa-f:.]+(%[0-9]+)?\]):[0-9]{1,5}$";

impl Schema for SocketAddrV4 {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).pattern(SOCKET_V4)
    }
}

impl Schema for SocketAddrV6 {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).pattern(SOCKET_V6)
    }
}

impl Schema for SocketAddr {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).pattern(SOCKET)
    }
}

macro_rules! non_zero_unsigned {
    ($($nz:ty => $t:ty),* $(,)?) => {$(
        /// The width's integers from 1.
        impl Schema for $nz {
            fn kind(alloc: Alloc) -> KindBuilder {
                KindBuilder::int_in(alloc)
                    .number(vocab::MINIMUM, "1")
                    .number(vocab::MAXIMUM, &<$t>::MAX.to_string())
            }
        }
    )*};
}

non_zero_unsigned!(
    NonZeroU8 => u8, NonZeroU16 => u16, NonZeroU32 => u32, NonZeroU64 => u64,
    NonZeroU128 => u128, NonZeroUsize => usize,
);

macro_rules! non_zero_signed {
    ($($nz:ty => $t:ty),* $(,)?) => {$(
        /// The width's integers, `not` the one that is zero:
        /// `{"not": {"const": 0}}` beside the bounds.
        impl Schema for $nz {
            fn kind(alloc: Alloc) -> KindBuilder {
                let mut zero = Map::new_in(alloc);
                let excluded = Number::new_in(alloc, "0")
                    .and_then(|n| Ok(zero.try_set(vocab::CONST, n)?))
                    .map(|()| Value::from(zero));
                <$t as Schema>::kind(alloc).keyword(vocab::NOT, excluded)
            }
        }
    )*};
}

non_zero_signed!(
    NonZeroI8 => i8, NonZeroI16 => i16, NonZeroI32 => i32, NonZeroI64 => i64,
    NonZeroI128 => i128, NonZeroIsize => isize,
);

/// `format: date-time`, RFC 3339's `date-time`.
impl Schema for SystemTime {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("date-time")
    }
}
