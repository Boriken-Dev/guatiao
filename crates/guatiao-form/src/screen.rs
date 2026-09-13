// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A type's default screen: the form `#[derive(Form)]` writes for it.

#![forbid(unsafe_code)]

use guatiao::value::alloc::Alloc;
use guatiao::value::mutate::ValueError;
use guatiao::value::types::Value;

use crate::build::Form;

/// A type that declares its default screen.
///
/// Implemented by `#[derive(Form)]` (behind the `derive` feature) from
/// `#[form(..)]` on the type and its fields, and written by hand for
/// anything else. It is one screen — the one the type is shown with when
/// nobody asks for another — and a second form beside it is still a
/// value a consumer builds or reads.
///
/// The derive is called `Form` after what it builds; this trait is not,
/// because [`Form`] is the builder it builds with.
pub trait Screen {
    /// The form, built through `alloc`: the sections the type declares,
    /// then every hint [`Screen::hints`] adds at the root.
    fn form(alloc: Alloc) -> Result<Value, ValueError>;

    /// Adds this type's field hints to `into`, each path under `prefix`
    /// — `""` at the root, `"auth."` for the fields a member named `auth`
    /// contributes — so an owner composes a member's hints by calling this
    /// with the member's key. Sections are the root's business and are
    /// not added here.
    fn hints(prefix: &str, into: Form) -> Form;
}
