// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library: its providers, what each serves, and the declaration the
//! host reads.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use guatiao::{List, Map, Value};

use crate::call::{Answer, Call, Handler, Refusal};

/// What builds an instance from its configuration.
pub(crate) type Build = Box<dyn FnMut(Value) -> Result<Box<dyn Handler>, Refusal>>;

/// What answers whether a provider can run now.
pub(crate) type Available = Box<dyn FnMut() -> Result<(), String>>;

/// One provider: an id unique in its library, a name for a person, the
/// kinds it serves with the slots it answers, and its handler; and, when
/// it has them, what builds its instances and what says whether it runs.
pub struct Provider {
    pub(crate) id: String,
    name: String,
    serves: Vec<(String, Vec<String>)>,
    pub(crate) handler: Box<dyn Handler>,
    config: Option<Value>,
    instances_only: bool,
    pub(crate) build: Option<Build>,
    pub(crate) available: Option<Available>,
}

/// The handler of a provider served only through its instances: the host
/// never calls it.
struct NotItself;

impl Handler for NotItself {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        Err(Refusal::unsupported(alloc::format!(
            "`{}` is called on an instance, not on the provider",
            call.method()
        )))
    }
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
            config: None,
            instances_only: false,
            build: None,
            available: None,
        }
    }

    /// A provider served only through instances `build` makes from a
    /// configuration fitting `schema`: [`config`](Provider::config) with
    /// no provider to call as itself.
    pub fn instances<H: Handler>(
        id: &str,
        schema: impl Into<Value>,
        build: impl FnMut(Value) -> Result<H, Refusal> + 'static,
    ) -> Provider {
        let mut provider = Provider::new(id, NotItself).config(schema, build);
        provider.instances_only = true;
        provider
    }

    /// Builds instances from a configuration fitting `schema` (a JSON
    /// Schema as a value, checked by the host first): `build` makes each
    /// one's handler, which its calls reach until the host drops it. The
    /// provider stays callable as itself.
    #[must_use]
    pub fn config<H: Handler>(
        mut self,
        schema: impl Into<Value>,
        mut build: impl FnMut(Value) -> Result<H, Refusal> + 'static,
    ) -> Provider {
        self.config = Some(schema.into());
        self.build = Some(Box::new(move |config| {
            build(config).map(|h| Box::new(h) as Box<dyn Handler>)
        }));
        self
    }

    /// Asked whether it can run each time a host asks: `Err(words)` is the
    /// reason it cannot. Without this it is always available.
    #[must_use]
    pub fn available(
        mut self,
        available: impl FnMut() -> Result<(), String> + 'static,
    ) -> Provider {
        self.available = Some(Box::new(available));
        self
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
        if let Some(schema) = &self.config {
            out.set("config", schema.clone());
        }
        if self.instances_only {
            out.set("instances_only", true);
        }
        if self.available.is_some() {
            out.set("may_be_unavailable", true);
        }
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
    /// "serves": {kind: [slot, ..]}, "config"?, "instances_only"?,
    /// "may_be_unavailable"?}, ..]}`.
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
