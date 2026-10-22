// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library that is nothing but its trait impls. The descriptor, the
//! tables, the entry symbol, the id (`derived_greeter_hello`) and the
//! version all come from the derive and the one macro line at the end.
extern crate alloc;

use core::sync::atomic::{AtomicI64, Ordering};

use greeter_kind::{Conversation, Counter, Greeter, Listener, Wide};
use guatiao::library::{Object, ProviderError};
use guatiao::{Alloc, Map, Provider, Status, Text, TryAsRef};
use guatiao_intake::Form;

/// One provider serving three kinds.
#[derive(Default, Provider)]
#[provider(Greeter, Counter, Wide)]
pub struct Hello {
    asked: AtomicI64,
}

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        self.greet_in(name, Alloc::rust())
    }

    /// What a call through the table runs: the map is built in the
    /// allocator the caller named, so it crosses without a copy.
    fn greet_in(&self, name: &str, alloc: Alloc) -> Result<Map, ProviderError> {
        if name == "panic" {
            panic!("a provider bug, on request");
        }
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        let mut map = Map::new_in(alloc);
        let greeting = Text::new_in(alloc, &format!("hello, {name}"))?;
        map.set_in("greeting", greeting, alloc)?;
        Ok(map)
    }

    fn greet_from(&self, config: Map) -> Result<Map, ProviderError> {
        self.greet_from_in(config, Alloc::rust())
    }

    /// A configuration handed over: read, then dropped here, through the
    /// caller's allocator it records. The answer is built where asked.
    fn greet_from_in(&self, config: Map, alloc: Alloc) -> Result<Map, ProviderError> {
        let name = config
            .get("name")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or_default();
        self.greet_in(name, alloc)
    }

    /// Completes the caller's configuration where it stands: `name` when
    /// it is missing, then the `greeting` it leads to.
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        // Built in the allocator the caller's map records, so nothing has
        // to be moved into it after the call.
        let alloc = config.alloc()?;
        if !config.contains_key("name") {
            config.set_in("name", Text::new_in(alloc, "world")?, alloc)?;
        }
        let Some(name) = config.get("name").and_then(TryAsRef::<str>::try_as_ref) else {
            return Ok(false);
        };
        let greeting = Text::new_in(alloc, &format!("hello, {name}"))?;
        config.set_in("greeting", greeting, alloc)?;
        Ok(true)
    }

    /// A conversation: an object this library builds and hands back,
    /// holding the listener the host handed in.
    fn start(
        &self,
        listener: Object<dyn Listener>,
    ) -> Result<Object<dyn Conversation>, ProviderError> {
        Ok(Chat {
            listener,
            transcript: String::new(),
            turns: 0,
        }
        .into_object())
    }
}

/// One conversation. Plain Rust; `into_object()` makes it a handle.
struct Chat {
    listener: Object<dyn Listener>,
    transcript: String,
    turns: i64,
}

impl Conversation for Chat {
    fn say(&mut self, what: &str) -> Result<(), ProviderError> {
        if what.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nothing to say",
            ));
        }
        if !self.transcript.is_empty() {
            self.transcript.push('\n');
        }
        self.transcript.push_str(what);
        self.turns += 1;
        // Across the boundary and back: the listener is the host's.
        self.listener.heard(what);
        Ok(())
    }

    fn read(&mut self, dst: &mut [u8]) -> i64 {
        let n = self.transcript.len().min(dst.len());
        dst[..n].copy_from_slice(&self.transcript.as_bytes()[..n]);
        n as i64
    }

    fn turns(&self) -> i64 {
        self.turns
    }
}

impl Counter for Hello {
    fn count(&self) -> i64 {
        self.asked.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl Wide for Hello {
    fn echo_i64(&self, x: i64) -> Result<i64, ProviderError> {
        Ok(x)
    }

    fn echo_u64(&self, x: u64) -> Result<u64, ProviderError> {
        Ok(x)
    }
}

/// A provider built from a configuration: no default instance, one
/// instance per configuration, each its own `self`. It is its own
/// configuration: a host reads this type's schema, fills it in, and the
/// value it hands to `instantiate` decodes straight into a `Shouter`.
///
/// It carries its form too: `form = Shouter` builds the screen
/// `#[derive(Form)]` writes into the descriptor's `meta`, so a host shows
/// the settings in two sections, with a placeholder, a unit and a field
/// that appears only when it applies.
// FORM-PROVIDER-BEGIN
#[derive(guatiao::Schema, guatiao::FromValue, Form, Provider)]
#[form(
    section(id = "words", label = "Words"),
    section(id = "volume", label = "Volume", help = "How loud a shout is")
)]
#[provider(Greeter, config, form = Shouter)]
pub struct Shouter {
    /// What every shout starts with.
    #[schema(section = "words")]
    #[form(placeholder = "hey")]
    prefix: String,
    /// Whether a shout ends in exclamation marks.
    #[schema(section = "volume")]
    #[map(default)]
    exclaim: bool,
    /// How many exclamation marks end a shout.
    #[schema(section = "volume")]
    #[map(default = 1)]
    #[form(unit = "marks", visible_when(field = "exclaim", equals = true))]
    marks: i64,
}
// FORM-PROVIDER-END

/// Builds its map with the short forms, in this library's own allocator:
/// the caller copies it on arrival.
impl Greeter for Shouter {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        let marks = if self.exclaim {
            "!".repeat(usize::try_from(self.marks).unwrap_or(0))
        } else {
            String::new()
        };
        let mut map = Map::new();
        map.set(
            "greeting",
            format!("{} {}{marks}", self.prefix, name.to_uppercase()),
        );
        Ok(map)
    }

    /// Adds its prefix with the short forms too: the value is built in
    /// this library's allocator, and the caller moves it into the map's
    /// once the call returns.
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        config.set("prefix", self.prefix.as_str());
        Ok(true)
    }
}

guatiao::providers!(Hello, Shouter);
