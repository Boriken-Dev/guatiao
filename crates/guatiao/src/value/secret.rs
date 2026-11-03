// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Secret`]: a value nobody prints, wiped when dropped.

#![forbid(unsafe_code)]

use core::fmt;

use super::wipe::Wipe;

/// A value nobody prints, wiped when dropped: a passphrase, a key, a
/// token.
///
/// **It crosses as `T` does**, unchanged, so another language reads and
/// writes it as the value it holds. What makes it a secret is its schema:
/// `T`'s own, with `writeOnly: true` and `x-sensitive: true`, which
/// [`redact`](crate::schema::redact) honours at any depth and the form a
/// schema implies shows as a masked field.
///
/// In Rust, [`Debug`](fmt::Debug) and [`Display`](fmt::Display) print
/// `[redacted]` whatever `T` holds, so a derived `Debug` on the struct
/// around it, a log line or a panic message does not carry it.
/// [`expose`](Secret::expose) is the one way to the value, and greps.
///
/// **Dropped, it [wipes](Wipe) what it holds**: every byte `T` holds at
/// that moment is overwritten with zeros before it is freed. What it
/// cannot reach is said by [`Wipe`]: a `String` that grew before it was
/// wrapped left earlier blocks unwiped, and a copy made elsewhere is that
/// copy's. Hold a secret in a [`Value`](crate::Value) or
/// [`Text`](crate::Text) built in [`Alloc::wiping`](crate::Alloc::wiping)
/// for one whose every block is wiped. It does not compare in constant
/// time (`==` is `T`'s own).
///
/// ```
/// use guatiao::Secret;
///
/// let key = Secret::new(String::from("hunter2"));
/// assert_eq!(format!("{key:?}"), "[redacted]");
/// assert_eq!(key.expose(), "hunter2");
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Secret<T: Wipe>(T);

impl<T: Wipe> Secret<T> {
    /// Holds `value` as a secret.
    pub fn new(value: T) -> Secret<T> {
        Secret(value)
    }

    /// The value, for the code that needs it.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// The value, no longer held as a secret: wiping it is the caller's.
    pub fn into_inner(mut self) -> T
    where
        T: Default,
    {
        core::mem::take(&mut self.0)
    }
}

impl<T: Wipe> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.wipe();
    }
}

impl<T: Wipe> From<T> for Secret<T> {
    fn from(value: T) -> Secret<T> {
        Secret(value)
    }
}

/// `[redacted]`, whatever it holds.
impl<T: Wipe> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// `[redacted]`, whatever it holds.
impl<T: Wipe> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
