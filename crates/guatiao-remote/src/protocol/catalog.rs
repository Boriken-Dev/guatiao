// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a LIST is answered with, as a value:
//!
//! ```text
//! {"kinds": [<kind description>, ...],
//!  "libraries": [{"id", "version", "stamp",
//!                 "providers": [{"id", "name", "meta"?,
//!                                "tables": [{"kind", "table", "slots": [..]}]}]}]}
//! ```
//!
//! `table` is the number a CALL names; `slots` the slots the table fills,
//! by name (a caller runs its own default for the others).

use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{List, Map, Value};

/// A server's catalog.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Every kind its tables and objects are of, described (`docs/encodings.md`, "A kind, described").
    pub kinds: Vec<Value>,
    /// Every library it serves.
    pub libraries: Vec<LibraryEntry>,
}

/// One library a server serves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryEntry {
    /// Its id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Changes whenever the library does.
    pub stamp: String,
    /// Its providers.
    pub providers: Vec<ProviderEntry>,
}

/// One provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Its id.
    pub id: String,
    /// Its display name.
    pub name: String,
    /// What else it declares.
    pub meta: Option<Map>,
    /// One per kind it serves.
    pub tables: Vec<TableEntry>,
}

/// One provider's table of one kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableEntry {
    /// The kind.
    pub kind: String,
    /// The number a call names it by.
    pub table: u64,
    /// The slots it fills.
    pub slots: Vec<String>,
}

fn text(map: &Map, key: &str, at: &str) -> Result<String, String> {
    map.get(key)
        .and_then(TryAsRef::<str>::try_as_ref)
        .map(str::to_string)
        .ok_or_else(|| format!("{at}: `{key}` is not text"))
}

fn list<'a>(map: &'a Map, key: &str, at: &str) -> Result<&'a List, String> {
    map.get(key)
        .and_then(TryAsRef::<List>::try_as_ref)
        .ok_or_else(|| format!("{at}: `{key}` is not a list"))
}

fn maps<'a>(list: &'a List, at: &str) -> Result<Vec<&'a Map>, String> {
    list.iter()
        .map(|v| {
            TryAsRef::<Map>::try_as_ref(v).ok_or_else(|| format!("{at}: an entry is not a map"))
        })
        .collect()
}

impl Catalog {
    /// As the value a REPLY carries.
    pub fn to_value(&self) -> Value {
        let mut kinds = List::new();
        for k in &self.kinds {
            kinds.push(k.clone());
        }
        let mut libraries = List::new();
        for l in &self.libraries {
            let mut providers = List::new();
            for p in &l.providers {
                let mut tables = List::new();
                for t in &p.tables {
                    let mut slots = List::new();
                    for s in &t.slots {
                        slots.push(s.as_str());
                    }
                    let mut table = Map::new();
                    table.set("kind", t.kind.as_str());
                    table.set("table", t.table);
                    table.set("slots", slots);
                    tables.push(table);
                }
                let mut provider = Map::new();
                provider.set("id", p.id.as_str());
                provider.set("name", p.name.as_str());
                if let Some(meta) = &p.meta {
                    provider.set("meta", meta.clone());
                }
                provider.set("tables", tables);
                providers.push(provider);
            }
            let mut library = Map::new();
            library.set("id", l.id.as_str());
            library.set("version", l.version.as_str());
            library.set("stamp", l.stamp.as_str());
            library.set("providers", providers);
            libraries.push(library);
        }
        let mut map = Map::new();
        map.set("kinds", kinds);
        map.set("libraries", libraries);
        Value::from(map)
    }

    /// Reads a catalog, saying where it is wrong.
    pub fn from_value(value: &Value) -> Result<Catalog, String> {
        let map = TryAsRef::<Map>::try_as_ref(value).ok_or("the catalog is not a map")?;
        let kinds = list(map, "kinds", "the catalog")?.iter().cloned().collect();
        let mut libraries = Vec::new();
        for l in maps(list(map, "libraries", "the catalog")?, "libraries")? {
            let id = text(l, "id", "a library")?;
            let at = format!("library `{id}`");
            let mut providers = Vec::new();
            for p in maps(list(l, "providers", &at)?, &at)? {
                let pid = text(p, "id", &at)?;
                let pat = format!("provider `{pid}`");
                let mut tables = Vec::new();
                for t in maps(list(p, "tables", &pat)?, &pat)? {
                    let table = t
                        .get("table")
                        .and_then(|v| u64::from_value(v).ok())
                        .ok_or_else(|| format!("{pat}: a table's `table` is not a number"))?;
                    let slots = list(t, "slots", &pat)?
                        .iter()
                        .map(|s| {
                            TryAsRef::<str>::try_as_ref(s)
                                .map(str::to_string)
                                .ok_or_else(|| format!("{pat}: a slot is not text"))
                        })
                        .collect::<Result<_, _>>()?;
                    tables.push(TableEntry {
                        kind: text(t, "kind", &pat)?,
                        table,
                        slots,
                    });
                }
                providers.push(ProviderEntry {
                    name: p
                        .get("name")
                        .and_then(TryAsRef::<str>::try_as_ref)
                        .unwrap_or_default()
                        .to_string(),
                    meta: p.get("meta").and_then(TryAsRef::<Map>::try_as_ref).cloned(),
                    id: pid,
                    tables,
                });
            }
            libraries.push(LibraryEntry {
                version: text(l, "version", &at)?,
                stamp: text(l, "stamp", &at)?,
                id,
                providers,
            });
        }
        Ok(Catalog { kinds, libraries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_round_trips() {
        let catalog = Catalog {
            kinds: vec![Value::from("not checked here")],
            libraries: vec![LibraryEntry {
                id: "lib".into(),
                version: "1.0.0".into(),
                stamp: "s".into(),
                providers: vec![ProviderEntry {
                    id: "p".into(),
                    name: "P".into(),
                    meta: None,
                    tables: vec![TableEntry {
                        kind: "greeter".into(),
                        table: 3,
                        slots: vec!["greet".into()],
                    }],
                }],
            }],
        };
        let back = Catalog::from_value(&catalog.to_value()).expect("reads");
        assert_eq!(back.libraries, catalog.libraries);
        assert_eq!(back.kinds.len(), 1);
        assert!(Catalog::from_value(&Value::null()).is_err());
    }
}
