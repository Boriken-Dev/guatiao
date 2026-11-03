// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[sensitive]` on a kind's slot: the description says it, the floor
//! hash does not, and the proxy names the wiping allocator for the answer
//! unless the caller names another.

#![cfg(feature = "provider")]

use core::ffi::c_void;

use guatiao::exports::kind::guatiao_kind_floor_hash;
use guatiao::library::{Kind, ProviderError, Remote};
use guatiao::value::types::Str;
use guatiao::{Alloc, Map, Status, Text, Value};

#[guatiao::kind(name = "vault")]
pub trait Vault: Send + Sync {
    /// A secret, as a tree.
    #[sensitive]
    fn read(&self, key: &str) -> Result<Value, ProviderError>;
    /// Secrets, by name.
    #[sensitive]
    fn all(&self) -> Result<Map, ProviderError>;
    /// A secret, as text.
    #[sensitive]
    fn pin(&self) -> String;
    /// Not a secret.
    fn name(&self) -> String;
}

/// [`Vault`] with no slot marked.
#[guatiao::kind(name = "vault_unmarked")]
pub trait VaultUnmarked: Send + Sync {
    /// A secret, as a tree.
    fn read(&self, key: &str) -> Result<Value, ProviderError>;
    /// Secrets, by name.
    fn all(&self) -> Result<Map, ProviderError>;
    /// A secret, as text.
    fn pin(&self) -> String;
    /// Not a secret.
    fn name(&self) -> String;
}

/// The floor hash both kinds have, pinned: the mark is not hashed.
const FLOOR_HASH: u32 = 249_464_095;

struct Keeper;

impl Vault for Keeper {
    fn read(&self, key: &str) -> Result<Value, ProviderError> {
        Ok(Text::new(key).into())
    }
    fn read_in(&self, key: &str, alloc: Alloc) -> Result<Value, ProviderError> {
        Ok(Text::new_in(alloc, key)?.into())
    }
    fn all(&self) -> Result<Map, ProviderError> {
        let mut m = Map::new();
        m.set("k", Text::new("v"));
        Ok(m)
    }
    fn pin(&self) -> String {
        String::from("1234")
    }
    fn name(&self) -> String {
        String::from("keeper")
    }
}

static KEEPER: Keeper = Keeper;
static KEEPER_TABLE: VaultVtable = VaultVtable::of::<Keeper>();

fn keeper() -> Remote<dyn Vault> {
    // SAFETY: a table `of::<Keeper>()` with the `&Keeper` it was built
    // for, both `'static`.
    unsafe {
        Remote::<dyn Vault>::from_raw(
            &KEEPER_TABLE as *const VaultVtable as *const c_void,
            size_of::<VaultVtable>(),
            &KEEPER as *const Keeper as *mut c_void,
        )
    }
    .expect("a full table validates")
}

fn hash_of(description: &str) -> u32 {
    let (mut out, mut why) = (0u32, Value::absent());
    // SAFETY: the text is readable, the allocator ours, the slots writable.
    let status = unsafe {
        guatiao_kind_floor_hash(
            Str::new(description),
            Alloc::rust().as_raw(),
            &mut out,
            &mut why,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK, "{why:?}");
    out
}

#[test]
fn the_mark_is_not_in_the_floor_hash() {
    assert_eq!(
        <dyn Vault as Kind>::FLOOR_HASH,
        <dyn VaultUnmarked as Kind>::FLOOR_HASH
    );
    assert_eq!(<dyn Vault as Kind>::FLOOR_HASH, FLOOR_HASH);
    assert_eq!(hash_of(&VaultVtable::description()), FLOOR_HASH);
    assert_eq!(hash_of(&VaultUnmarkedVtable::description()), FLOOR_HASH);
}

#[test]
fn the_description_marks_each_sensitive_slot_and_no_other() {
    let marked = VaultVtable::description();
    assert_eq!(marked.matches("\"sensitive\":true").count(), 3, "{marked}");
    let unmarked = VaultUnmarkedVtable::description();
    assert!(!unmarked.contains("sensitive"), "{unmarked}");
    let read = guatiao::library::kind::KindDescription::document_json(&format!(
        "{{\"kinds\":[{marked}]}}"
    ))
    .expect("the reader takes it");
    let flags: Vec<(String, bool)> = read[0]
        .slots()
        .iter()
        .map(|s| (s.name().to_string(), s.sensitive()))
        .collect();
    assert_eq!(
        flags,
        [
            ("read", true),
            ("all", true),
            ("pin", true),
            ("name", false)
        ]
        .map(|(n, f)| (n.to_string(), f))
    );
}

#[test]
fn a_sensitive_answer_is_in_the_wiping_allocator_unless_the_caller_names_another() {
    let vault = keeper();
    let read = vault.read("hunter2").unwrap();
    assert!(read.alloc().unwrap().is_wiping());
    assert!(read.is_sensitive());
    let all = Value::from(vault.all().unwrap());
    assert!(all.alloc().unwrap().is_wiping());
    let named = vault.read_in("hunter2", Alloc::rust()).unwrap();
    assert!(!named.alloc().unwrap().is_wiping());
    assert_eq!(vault.pin(), "1234");
    assert_eq!(vault.name(), "keeper");
}
