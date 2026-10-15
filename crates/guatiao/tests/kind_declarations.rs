// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Declarations the kind attribute accepts whose names come close to the
//! glue's own: each compiles and is called once through its table.

#![cfg(feature = "provider")]

use guatiao::library::{ProviderError, Remote};
use guatiao::{FromValue, ToValue};

/// `x` and `mut_x`: the lent tables' shims for the two must not share a
/// name.
#[guatiao::kind(object)]
pub trait Pair: Send {
    fn x(&self) -> i64;
    fn mut_x(&mut self) -> i64;
}

/// A provider kind's table has no `destroy` slot of its own, so a method
/// may take the name.
#[guatiao::kind]
pub trait Pool: Send + Sync {
    fn destroy(&self) -> i64;
    /// Lends a pair both ways.
    fn both(&self, mutable: &mut dyn Pair, shared: &dyn Pair) -> Result<i64, ProviderError>;
    /// A type of the caller's that ends in `String`, by value.
    fn take(&self, s: proto::String) -> Result<i64, ProviderError>;
}

pub mod proto {
    /// The caller's own `String`, crossing through `ToValue`/`FromValue`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct String(pub i64);
}

impl ToValue for proto::String {
    fn to_value_in(&self, alloc: guatiao::Alloc) -> Result<guatiao::Value, guatiao::ValueError> {
        self.0.to_value_in(alloc)
    }
}

impl FromValue for proto::String {
    fn from_value(v: &guatiao::Value) -> Result<Self, guatiao::value::convert::MapError> {
        Ok(proto::String(i64::from_value(v)?))
    }
}

struct Counter(i64);

impl Pair for Counter {
    fn x(&self) -> i64 {
        self.0
    }
    fn mut_x(&mut self) -> i64 {
        self.0 += 1;
        self.0
    }
}

struct Plain;

impl Pool for Plain {
    fn destroy(&self) -> i64 {
        7
    }
    fn both(&self, mutable: &mut dyn Pair, shared: &dyn Pair) -> Result<i64, ProviderError> {
        Ok(mutable.mut_x() * 100 + shared.x())
    }
    fn take(&self, s: proto::String) -> Result<i64, ProviderError> {
        Ok(s.0 * 2)
    }
}

static PLAIN: PoolVtable = PoolVtable::of::<Plain>();

fn pool() -> Remote<dyn Pool> {
    // SAFETY: the table `of::<Plain>()` built; `Plain` is a ZST read only
    // through `&Plain`, so any aligned non-null address serves as `ctx`.
    unsafe {
        Remote::<dyn Pool>::from_raw(
            (&PLAIN as *const PoolVtable).cast(),
            size_of::<PoolVtable>(),
            core::ptr::NonNull::<Plain>::dangling().as_ptr().cast(),
        )
    }
    .expect("the table validates")
}

#[test]
fn a_provider_method_named_destroy_is_an_ordinary_slot() {
    assert_eq!(pool().destroy(), 7);
}

#[test]
fn methods_named_x_and_mut_x_are_lent_through_their_own_slots() {
    let mut mutable = Counter(1);
    let shared = Counter(5);
    assert_eq!(pool().both(&mut mutable, &shared).expect("the call"), 205);
    assert_eq!(mutable.0, 2);
    let mut owned = Counter(9).into_object();
    assert_eq!((owned.x(), owned.mut_x()), (9, 10));
}

#[test]
fn a_type_of_the_callers_named_string_crosses_by_value() {
    assert_eq!(pool().take(proto::String(21)).expect("the call"), 42);
}
