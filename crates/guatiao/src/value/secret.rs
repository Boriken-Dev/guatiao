// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Secret`]: a value nobody prints.

#![forbid(unsafe_code)]

use core::fmt;

/// A value nobody prints: a passphrase, a key, a token.
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
/// What it does not do: compare in constant time (`==` is `T`'s own), or
/// wipe memory when dropped. A value moves, a `String` reallocates as it
/// grows, and a value tree holds its own copy; none of those copies is
/// this type's to wipe, so wiping its own would promise what it cannot
/// keep.
///
/// ```
/// use guatiao::Secret;
///
/// let key = Secret::new(String::from("hunter2"));
/// assert_eq!(format!("{key:?}"), "[redacted]");
/// assert_eq!(key.expose(), "hunter2");
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Holds `value` as a secret.
    pub fn new(value: T) -> Secret<T> {
        Secret(value)
    }

    /// The value, for the code that needs it.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// The value, no longer held as a secret.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Secret<T> {
        Secret(value)
    }
}

/// `[redacted]`, whatever it holds.
impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// `[redacted]`, whatever it holds.
impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
