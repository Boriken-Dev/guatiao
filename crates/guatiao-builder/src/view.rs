// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao value read as a coqui-builder [`Inspect`].

use coqui_builder::describe::Kind;
use coqui_builder::path::Segment;
use coqui_builder::{Datum, Description, Entries, Inspect};
use guatiao::Value;

use crate::source::{entries_of, lookup};
use crate::text;

/// A guatiao [`Value`] read through the description of the type it holds, so a report, a
/// profile or `differences` can judge it as they judge a built value.
///
/// A leaf is a [`Datum`] of the leaf's type name and the value's text (see [`crate::text`]),
/// marked sensitive where the description says. A list answers its length and a map its keys.
#[derive(Debug, Clone, Copy)]
pub struct ValueView<'a> {
    description: &'a Description,
    value: &'a Value,
}

impl<'a> ValueView<'a> {
    /// `value` read through `description`.
    #[must_use]
    pub fn new(description: &'a Description, value: &'a Value) -> ValueView<'a> {
        ValueView { description, value }
    }
}

impl Inspect for ValueView<'_> {
    fn at(&self, at: &[Segment]) -> Option<Datum> {
        let node = lookup(self.value, at)?;
        // A key several arms share is read from the arm the value names.
        let chosen = |tag: &[Segment]| {
            text::from_value(lookup(self.value, tag)?, "String")
                .ok()
                .flatten()
        };
        let place = self.description.resolve_chosen(at, &chosen)?.description();
        let ty = match place.kind() {
            Kind::Leaf(info) => info.ty(),
            Kind::Union(union) => union.text().ty(),
            _ => return None,
        };
        let text = text::from_value(node, ty).ok()??;
        let datum = Datum::new(ty, text);
        Some(if place.notes().sensitive {
            datum.sensitive()
        } else {
            datum
        })
    }

    fn entries(&self, at: &[Segment]) -> Option<Entries> {
        entries_of(lookup(self.value, at)?)
    }
}
