// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The kinds a bridge knows: descriptions read from a kind crate's
//! `<crate>.kinds.json`, from `<Trait>Vtable::description()` text, or
//! from a value.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use std::path::Path;

use guatiao::library::kind::{AnswerType, ArgType, KindDescription};
use guatiao::value::convert::{ToValue, TryAsRef};
use guatiao::{Alloc, List, Map, Value};

use crate::error::BridgeError;

/// Descriptions by kind name.
#[derive(Debug, Clone, Default)]
pub struct Kinds {
    by_name: BTreeMap<String, Arc<KindDescription>>,
}

impl Kinds {
    /// None yet.
    pub fn new() -> Kinds {
        Kinds::default()
    }

    /// Adds one kind. The same description again is a no-op; another one
    /// under the same name is refused, since a table built from either
    /// would be called as the other.
    pub fn add(&mut self, kind: KindDescription) -> Result<(), BridgeError> {
        if let Some(known) = self.by_name.get(kind.name()) {
            if **known == kind {
                return Ok(());
            }
            return Err(BridgeError::Table(format!(
                "two descriptions of the kind `{}` differ (floor hashes {} and {})",
                kind.name(),
                known.floor_hash(),
                kind.floor_hash()
            )));
        }
        self.by_name.insert(kind.name().to_string(), Arc::new(kind));
        Ok(())
    }

    /// Adds every kind a value describes: a document
    /// (`{"format": 1, "kinds": [..]}`) or one kind. Answers how many.
    pub fn add_value(&mut self, value: &Value) -> Result<usize, BridgeError> {
        let is_document =
            TryAsRef::<Map>::try_as_ref(value).is_some_and(|m| m.get("format").is_some());
        let kinds = if is_document {
            KindDescription::document(value)
        } else {
            KindDescription::from_value(value).map(|k| vec![k])
        }
        .map_err(|e| BridgeError::Table(e.to_string()))?;
        let n = kinds.len();
        for kind in kinds {
            self.add(kind)?;
        }
        Ok(n)
    }

    /// Adds every kind JSON text describes, as [`add_value`](Self::add_value).
    pub fn add_json(&mut self, json: &str) -> Result<usize, BridgeError> {
        let value = guatiao_serde::text::json::from_str(
            json,
            Alloc::rust(),
            guatiao_serde::Presentation::new(),
        )
        .map_err(|e| BridgeError::Table(format!("a kind description is not JSON: {e}")))?;
        self.add_value(&value)
    }

    /// Adds every kind a description document on disk holds.
    pub fn load(&mut self, path: &Path) -> Result<usize, BridgeError> {
        let text = std::fs::read_to_string(path).map_err(|e| BridgeError::Manifest {
            path: path.to_path_buf(),
            why: format!("{}: {e}", path.display()),
        })?;
        self.add_json(&text).map_err(|e| BridgeError::Manifest {
            path: path.to_path_buf(),
            why: format!("{}: {e}", path.display()),
        })
    }

    /// The kind named `name`.
    pub fn get(&self, name: &str) -> Option<&Arc<KindDescription>> {
        self.by_name.get(name)
    }

    /// Every kind's name, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// The document a provider serving `served` carries
    /// (`ProviderPartsBuilder::kind_descriptions`): `{"format":1,"kinds":[..]}`
    /// as compact JSON, each kind served followed by the object kinds its
    /// slots take or answer, in slot order and transitively, every kind
    /// once, as `#[derive(Provider)]` writes it. A kind not known here is
    /// left out; empty when none is known.
    pub fn document<'a>(
        &self,
        served: impl IntoIterator<Item = &'a str>,
    ) -> Result<String, BridgeError> {
        let mut order = Vec::new();
        for name in served {
            self.reach(name, &mut order);
        }
        if order.is_empty() {
            return Ok(String::new());
        }
        let written = |e: guatiao::ValueError| {
            BridgeError::Table(format!("a kind's description cannot be written: {e}"))
        };
        let mut kinds = List::new();
        for kind in order {
            kinds.push(kind.to_value().map_err(written)?);
        }
        let mut document = Map::new();
        document.set("format", 1u32);
        document.set("kinds", kinds);
        guatiao_serde::text::json::to_string(
            &Value::from(document),
            guatiao_serde::Presentation::new(),
        )
        .map_err(|e| BridgeError::Table(format!("a kind document cannot be written: {e}")))
    }

    /// Pushes `name`, then every object kind its slots reach, each once.
    fn reach<'s>(&'s self, name: &str, order: &mut Vec<&'s KindDescription>) {
        let Some(kind) = self.by_name.get(name) else {
            return;
        };
        if order.iter().any(|k| k.name() == name) {
            return;
        }
        order.push(kind);
        for slot in kind.slots() {
            for arg in slot.args() {
                if let ArgType::Object { kind: object, .. } = arg.ty() {
                    self.reach(object, order);
                }
            }
            if let Some(AnswerType::Object { kind: object }) = slot.answer() {
                self.reach(object, order);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greeter() -> String {
        greeter_kind::GreeterVtable::description()
    }

    #[test]
    fn a_kind_is_added_once_and_a_different_one_of_its_name_refused() {
        let mut kinds = Kinds::new();
        assert_eq!(kinds.add_json(&greeter()).expect("JSON"), 1);
        assert_eq!(kinds.add_json(&greeter()).expect("the same again"), 1);
        let other = greeter().replace("\"floor_hash\":", "\"floor_hash\":1");
        assert!(kinds.add_json(&other).is_err());
        assert_eq!(kinds.names().collect::<Vec<_>>(), ["greeter"]);
    }

    /// The document a provider of greeter and counter carries is the one
    /// the derive writes for the same kinds: the same kinds, in the same
    /// order.
    #[test]
    fn a_document_names_what_is_served_and_every_object_kind_it_reaches() {
        use greeter_kind::{Counter, Greeter};
        use guatiao::library::Kind;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
        let mut kinds = Kinds::new();
        kinds.load(&path).expect("read");

        let document = kinds
            .document(["greeter", "counter", "nobody"])
            .expect("written");
        let derived = guatiao::library::kind::kind_document(&[
            <dyn Greeter as Kind>::describe,
            <dyn Counter as Kind>::describe,
        ]);
        assert_eq!(
            KindDescription::document_json(&document).expect("reads"),
            KindDescription::document_json(&derived).expect("reads")
        );
        assert_eq!(kinds.document(["nobody"]).expect("written"), "");
    }

    #[test]
    fn a_document_on_disk_is_loaded() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
        let mut kinds = Kinds::new();
        assert_eq!(kinds.load(&path).expect("read"), 5);
        assert!(kinds.get("listener").expect("described").object());
    }
}
