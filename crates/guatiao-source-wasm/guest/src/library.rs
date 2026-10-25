// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library: its providers, what each serves, and the declaration the
//! host reads.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use guatiao::{List, Map, Value};

use crate::call::Handler;

/// One provider: an id unique in its library, a name for a person, the
/// kinds it serves with the slots it answers, and its handler.
pub struct Provider {
    pub(crate) id: String,
    name: String,
    serves: Vec<(String, Vec<String>)>,
    pub(crate) handler: Box<dyn Handler>,
}

impl Provider {
    /// A provider answering through `handler`. The host serves it as
    /// `<module>_<id>`.
    pub fn new(id: &str, handler: impl Handler) -> Provider {
        Provider {
            id: id.to_string(),
            name: String::new(),
            serves: Vec::new(),
            handler: Box::new(handler),
        }
    }

    /// A name to show a person (default: its id).
    #[must_use]
    pub fn name(mut self, name: &str) -> Provider {
        self.name = name.to_string();
        self
    }

    /// Serves `kind`, answering the slots named: a slot left out runs the
    /// kind's default at the caller; a required one left out refuses the
    /// library.
    #[must_use]
    pub fn serves<'a>(mut self, kind: &str, slots: impl IntoIterator<Item = &'a str>) -> Provider {
        self.serves.push((
            kind.to_string(),
            slots.into_iter().map(str::to_string).collect(),
        ));
        self
    }

    fn declaration(&self) -> Map {
        let mut serves = Map::new();
        for (kind, slots) in &self.serves {
            let mut names = List::new();
            for slot in slots {
                names.push(slot.as_str());
            }
            serves.set(kind, names);
        }
        let mut out = Map::new();
        out.set("id", self.id.as_str());
        let name = if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        };
        out.set("name", name.as_str());
        out.set("serves", serves);
        out
    }
}

/// The providers a module offers, in the order the host addresses them.
#[derive(Default)]
pub struct Library {
    pub(crate) providers: Vec<Provider>,
}

impl Library {
    /// A library with no providers yet.
    pub fn new() -> Library {
        Library::default()
    }

    /// Adds a provider.
    #[must_use]
    pub fn provider(mut self, provider: Provider) -> Library {
        self.providers.push(provider);
        self
    }

    /// The declaration the host reads: `{"providers": [{"id", "name",
    /// "serves": {kind: [slot, ..]}}, ..]}`.
    pub(crate) fn declaration(&self) -> Value {
        let mut providers = List::new();
        for p in &self.providers {
            providers.push(p.declaration());
        }
        let mut out = Map::new();
        out.set("providers", providers);
        Value::from(out)
    }
}
