// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Loading libraries and reading a registry, from C — or from Python
//! through `ctypes`, or from anything else that can call a C function.
//!
//! **Rust is one user of this crate, not its audience.** A host written in
//! any language can create a registry, load libraries into it, ask what it
//! has, and call through a provider's table.
//!
//! # The registry is the one opaque handle in this crate
//!
//! Everything else crosses as a plain struct, on purpose. A registry
//! cannot: it owns growable collections and its whole job is to change
//! over time, which is what a handle is for and what a `repr(C)` struct is
//! not. The line this draws is the one [`crate::schema::vocab`] states —
//! **data crosses as a value, and only code crosses as a typed struct** —
//! with a handle for the third thing, a live object with behaviour.
//!
//! # Answers come back as VALUES
//!
//! [`guatiao_registry_providers`] does not hand back an array of structs
//! with an accessor per field; it builds an ordinary list of maps and
//! writes it to an out parameter. A caller already has everything needed
//! to walk one — the header's `static inline` readers need nothing linked
//! — so this adds no API to learn, and a language with a binding for a
//! value already has a binding for the answer.
//!
//! The exception is the half that is **not data**: a vtable pointer, its
//! size, the `ctx` to pass back, and a borrowed config schema. Those have
//! their own accessors, because a pointer inside a map would be a number a
//! caller has to cast back, and because reading one is the moment a caller
//! takes on the kind's contract.
//!
//! # The shape of every function here
//!
//! **The work is a safe method on [`HostRegistry`]; the `extern "C"`
//! function converts pointers and calls it.** So `unsafe` in this file is
//! a handful of conversions with the same contract each time, rather than
//! a body per export that a reader has to audit separately.
//!
//! # Why these are behind `load`
//!
//! Unlike the rest of the `extern "C"` surface, which is always compiled.
//! Loading needs `libloading`, and the `load` feature exists so a library
//! AUTHOR — who writes a descriptor and never loads anything — builds with
//! no dependency at all.

#![allow(non_camel_case_types)]

use alloc::collections::BTreeMap;
use core::ffi::c_void;
use std::path::Path;

use super::{guard, guard_with, out};
use crate::library::ProviderError;
use crate::library::{
    EntryFn, HostInfo, LibraryInfo, LoadReport, Loading, Order, Provider, Refreshed, Registry,
    ScanRules, SearchPath, Served, Skipped, SourceError, SourceInfo, UnloadError, WhyNot,
    scan_dir_rules, scan_path,
};
use crate::schema::read::SchemaRef;
use crate::schema::validate::validate_map;
use crate::value::ValueError;
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{List, Map, Str, Text, Value};

/// A host's registry of loaded libraries.
///
/// Opaque: create one with [`guatiao_registry_new`] and release it with
/// [`guatiao_registry_free`]. Every other function here takes the pointer
/// that gave you.
///
/// **Not thread-safe.** One registry is one host's table; a host sharing
/// one across threads guards it itself, as it would any other mutable
/// object it owns. What a library reaches through its host's `services`
/// is a snapshot the registry publishes after every change, guarded on
/// its own, so a library asking from any thread never contends with the
/// host's own calls — but a library's entry point must not call these
/// functions on the handle that is loading it, which is held exclusively
/// for the whole call.
#[derive(Debug)]
pub struct HostRegistry {
    inner: Registry,
    /// Every instance `guatiao_registry_provider_create` built and
    /// `guatiao_registry_provider_destroy` has not released, by address,
    /// with the view of the provider that built it: its `destroy` slot,
    /// and the lease that keeps its library open while the instance lives,
    /// whatever has been retired or loaded under its key since.
    instances: std::sync::Mutex<BTreeMap<usize, crate::library::ProviderView>>,
}

// --- the work, in safe Rust --------------------------------------------

impl HostRegistry {
    /// Borrows a handle, or `None` for null.
    ///
    /// # Safety
    ///
    /// `reg` is null, or a live handle from [`guatiao_registry_new`] that
    /// has not been freed and that nothing else is mutating.
    unsafe fn get<'a>(reg: *const HostRegistry) -> Option<&'a HostRegistry> {
        // SAFETY: the caller's contract, stated above.
        unsafe { reg.as_ref() }
    }

    /// Borrows a handle mutably, or `None` for null.
    ///
    /// # Safety
    ///
    /// As [`HostRegistry::get`], and nothing else is using it at all.
    unsafe fn get_mut<'a>(reg: *mut HostRegistry) -> Option<&'a mut HostRegistry> {
        // SAFETY: the caller's contract, stated above.
        unsafe { reg.as_mut() }
    }

    /// Re-keys, leaving the registry unchanged on a refusal.
    fn rekey(&mut self, template: &str, libraries: bool) -> Status {
        let done = if libraries {
            self.inner.libraries_keyed_by(template)
        } else {
            self.inner.keyed_by(template)
        };
        match done {
            Ok(()) => Status::GUATIAO_OK,
            Err(_) => Status::GUATIAO_ERR_BAD_VALUE,
        }
    }

    /// Loads one file and describes what happened.
    ///
    /// **All three outcomes are an answer**, refusal included: the map
    /// says `loaded`, `skipped` or `failed`, and a failure carries the
    /// loader's own message, which names the real cause far better than a
    /// status code could. The `Err` arm is reserved for not being able to
    /// answer at all.
    fn load(&mut self, path: &str, alloc: Alloc) -> Result<Value, Status> {
        let mut map = Map::new_in(alloc);
        match self.inner.load_file(Path::new(path)) {
            Ok(Loading::Loaded(one)) => {
                let described = library_value(alloc, one)?;
                map.try_set("loaded", described)?;
            }
            Ok(Loading::Skipped(why)) => return Ok(skip_value(alloc, &why)?),
            Err(e) => map.try_set("failed", e.to_string().as_str())?,
        }
        Ok(map.into())
    }

    /// [`load`](HostRegistry::load) for a library the host links: its
    /// entry point, called with this registry's host block.
    fn register(&mut self, name: &str, entry: EntryFn, alloc: Alloc) -> Result<Value, Status> {
        let mut map = Map::new_in(alloc);
        match self.inner.register_entry(name, entry) {
            Ok(Loading::Loaded(one)) => {
                let described = library_value(alloc, one)?;
                map.try_set("loaded", described)?;
            }
            Ok(Loading::Skipped(why)) => return Ok(skip_value(alloc, &why)?),
            Err(e) => map.try_set("failed", e.to_string().as_str())?,
        }
        Ok(map.into())
    }

    /// Scans a directory and reports the three outcomes.
    fn scan(
        &mut self,
        dir: &str,
        order: Order,
        rules: &ScanRules,
        alloc: Alloc,
    ) -> Result<Value, Status> {
        // The one thing a scan cannot answer: the directory itself.
        let report = scan_dir_rules(&mut self.inner, Path::new(dir), order, rules)
            .map_err(|_| Status::GUATIAO_ERR_NOT_FOUND)?;
        Ok(report_value(alloc, &report)?)
    }

    /// Walks a search path and reports everything it found, including
    /// the entries it could not read.
    fn scan_path(
        &mut self,
        spec: &str,
        order: Order,
        rules: &ScanRules,
        alloc: Alloc,
    ) -> Result<Value, ValueError> {
        let path = SearchPath::parse(spec);
        let report = scan_path(&mut self.inner, &path, order, rules);
        report_value(alloc, &report)
    }

    /// Every library loaded, as a list of maps.
    fn libraries(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        for one in self.inner.loaded() {
            list.try_push(library_value(alloc, one)?)?;
        }
        Ok(list.into())
    }

    /// Every provider serving `kind`, or all of them when it is empty,
    /// best first.
    fn providers(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        let ranked: Vec<&Provider> = if kind.is_empty() {
            self.inner.all_ranked().collect()
        } else {
            self.inner.providers(kind).collect()
        };
        for provider in ranked {
            list.try_push(provider_value(alloc, provider)?)?;
        }
        Ok(list.into())
    }

    /// One provider by key, or `None` when nothing answers to it.
    fn provider(&self, key: &str, alloc: Alloc) -> Option<Result<Value, ValueError>> {
        self.inner.provider(key).map(|p| provider_value(alloc, p))
    }

    /// Every provider serving `kind` that can actually run here.
    fn available(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        for provider in self.inner.available(kind) {
            list.try_push(provider_value(alloc, provider)?)?;
        }
        Ok(list.into())
    }

    /// Why nothing can serve `kind`, or that something can.
    fn why_not(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut map = Map::new_in(alloc);
        let Some(why) = self.inner.why_not(kind) else {
            map.try_set("available", Value::from(true))?;
            return Ok(map.into());
        };
        map.try_set("available", Value::from(false))?;
        match why {
            WhyNot::NothingClaimsIt => {
                map.try_set(
                    "why",
                    Text::new_in(alloc, "nothing-claims-it").map(Value::from)?,
                )?;
            }
            WhyNot::NoneAvailable(refused) => {
                map.try_set(
                    "why",
                    Text::new_in(alloc, "none-available").map(Value::from)?,
                )?;
                let mut list = List::new_in(alloc);
                for (provider, reason) in refused {
                    let mut one = Map::new_in(alloc);
                    one.try_set("id", Text::new_in(alloc, provider.id()).map(Value::from)?)?;
                    one.try_set("reason", Text::new_in(alloc, reason).map(Value::from)?)?;
                    list.try_push(one)?;
                }
                map.try_set("providers", list)?;
            }
        }
        Ok(map.into())
    }

    /// Whether one provider can run here, and why not when it cannot.
    fn provider_available(&self, key: &str) -> Option<Result<(), &str>> {
        self.inner.provider(key).map(Provider::available)
    }

    /// Ranks every provider with this id.
    fn set_priority(&mut self, id: &str, priority: i32) {
        self.inner.set_priority(id, priority);
    }

    /// What this host ranked that id.
    fn priority(&self, id: &str) -> i32 {
        self.inner.priority(id)
    }

    /// The best provider serving a kind that can run here, as a map.
    fn best(&self, kind: &str, alloc: Alloc) -> Option<Result<Value, ValueError>> {
        self.inner.best(kind).map(|p| provider_value(alloc, p))
    }

    /// One provider's table and the size it was compiled at.
    fn vtable(&self, key: &str) -> Option<(*const c_void, usize)> {
        self.inner.provider(key).map(Provider::vtable)
    }

    /// The table one provider speaks `kind` through.
    fn table_for(&self, key: &str, kind: &str) -> Option<(*const c_void, usize)> {
        self.inner.provider(key)?.table_for(kind)
    }

    /// One provider's context pointer.
    fn ctx(&self, key: &str) -> Option<*mut c_void> {
        self.inner.provider(key).map(Provider::ctx)
    }

    /// Closes the retired libraries nothing uses, as a list of their keys.
    fn collect(&mut self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        for key in self.inner.collect() {
            list.try_push(Text::new_in(alloc, &key).map(Value::from)?)?;
        }
        Ok(list.into())
    }

    /// One provider's configuration schema, the copy this registry owns.
    fn config(&self, key: &str) -> Option<&Value> {
        self.inner.provider(key).and_then(Provider::config_schema)
    }

    /// One provider's `meta`: `None` for an unknown key, `Some(None)` for a
    /// provider that declares none.
    fn meta(&self, key: &str) -> Option<Option<&Map>> {
        self.inner.provider(key).map(Provider::meta)
    }

    /// Refreshes every source and describes what changed.
    fn refresh(&mut self, alloc: Alloc) -> Result<Value, ValueError> {
        refreshed_value(alloc, &self.inner.refresh())
    }
}

/// A refusal to add a source, as its status, its words written to `err`.
fn source_refusal(err: *mut ProviderError, why: &SourceError) -> Status {
    let status = why.status();
    if !err.is_null() {
        // SAFETY: non-null and writable by the caller's contract, written
        // empty on entry.
        unsafe { err.write(ProviderError::new(status, &why.to_string())) };
    }
    status
}

/// What a refusal to retire or unload crosses as.
///
/// No wildcard arm: [`UnloadError`] is `#[non_exhaustive]` for a
/// CONSUMER, so appending a variant is a compile error here rather than a
/// reason that crosses the boundary unnamed.
fn unload_status(why: &UnloadError) -> Status {
    match why {
        UnloadError::NotFound { .. } => Status::GUATIAO_ERR_NOT_FOUND,
        UnloadError::Linked { .. } => Status::GUATIAO_ERR_WRONG_KIND,
        UnloadError::NotSupported { .. } => Status::GUATIAO_ERR_UNSUPPORTED,
        UnloadError::Busy { .. } => Status::GUATIAO_ERR_BUSY,
        // The library's own answer, as it gave it.
        UnloadError::Refused { status, .. } => *status,
        UnloadError::Close { .. } => Status::GUATIAO_ERR_INTERNAL,
    }
}

// --- the boundary: convert pointers, call the impl ----------------------

/// A new registry introducing its host as `id`/`version`.
///
/// `alloc` may be null, and is the allocator offered to every library, so
/// a tree a library builds for this host is built in the host's own arena.
/// A library may ignore it and use its own; either way the tree carries
/// the allocator that made it, so this host frees it correctly without
/// knowing which.
///
/// The two strings are **copied**, so they need not outlive this call.
/// Returns null when one of them is not UTF-8, or when `alloc` is
/// non-null and incomplete.
///
/// # Safety
///
/// `id` and `version` are views whose bytes are readable for this call,
/// and `alloc` is null or a complete allocator that outlives every tree
/// built through it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_new(
    id: Str,
    version: Str,
    alloc: *const Allocator,
) -> *mut HostRegistry {
    guard_with(core::ptr::null_mut(), || {
        // SAFETY: the caller's contract.
        let (Ok(id), Ok(version)) = (
            core::str::from_utf8(id.into()),
            core::str::from_utf8(version.into()),
        ) else {
            return core::ptr::null_mut();
        };
        // Null is "no allocator", which is an answer. A malformed one is
        // not, and is refused rather than ignored.
        let alloc = if alloc.is_null() {
            None
        } else {
            // SAFETY: the caller's contract.
            match unsafe { Alloc::from_raw(alloc) } {
                Ok(a) => Some(a),
                Err(_) => return core::ptr::null_mut(),
            }
        };
        Box::into_raw(Box::new(HostRegistry {
            inner: Registry::with_alloc(id, version, alloc),
            instances: std::sync::Mutex::new(BTreeMap::new()),
        }))
    })
}

/// Builds an instance of the provider filed under `key` from `config`.
///
/// On `GUATIAO_OK` the instance is written through `out`, and it is the
/// `ctx` to pass to every slot of that provider's tables for calls on it;
/// release it with [`guatiao_registry_provider_destroy`]. `err` may be
/// null; when it is not, a failure writes the provider's own words there
/// (free the message like any text). `GUATIAO_ERR_NOT_FOUND` for an
/// unknown key; `GUATIAO_ERR_NULL` for a provider that builds no
/// instances, which is its one instance.
///
/// Both are written first: `out` null and `err` an empty error
/// (`GUATIAO_OK`, empty message), so whatever the call answers, `err`'s
/// message may be freed and `out` read.
///
/// `config` is checked against the provider's configuration schema first,
/// as the Rust `instantiate` does: a misspelled or ill-typed setting is
/// `GUATIAO_ERR_BAD_VALUE`, with the field and what it expected written
/// to `err`, and the provider is never asked. A schema this build cannot
/// read is not checked.
///
/// # Safety
///
/// `reg` is a live handle, `key` is readable, `config` addresses a
/// well-formed value, `out` is writable, `err` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_create(
    reg: *const HostRegistry,
    key: Str,
    config: *const Value,
    out: *mut *mut c_void,
    err: *mut crate::library::ProviderError,
) -> Status {
    // Both out-parameters first, so a caller that frees `err.message` or
    // reads `out` after any failure finds nothing it must not touch.
    if !out.is_null() {
        // SAFETY: non-null, and writable by the caller's contract.
        unsafe { out.write(core::ptr::null_mut()) };
    }
    if !err.is_null() {
        // SAFETY: non-null, and writable by the caller's contract; the
        // placeholder owns nothing.
        unsafe { err.write(ProviderError::none()) };
    }
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        let Some(config) = (unsafe { config.as_ref() }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        if out.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        let Some(provider) = registry.inner.provider(key) else {
            return Status::GUATIAO_ERR_NOT_FOUND;
        };
        let view = provider.view();
        let Some(create) = view.create else {
            return Status::GUATIAO_ERR_NULL;
        };
        if let Some(schema) = view.config.as_ref().and_then(SchemaRef::new)
            && let Err(why) = validate_map(schema, config)
        {
            let refusal = ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why.to_string());
            if !err.is_null() {
                // SAFETY: non-null, and writable by the caller's contract.
                unsafe { err.write(refusal) };
            }
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: the slot is the descriptor's own; the pointers are the
        // caller's, checked above; the library is mapped for the call.
        let status = unsafe { create(view.ctx, config, out, err) };
        // SAFETY: non-null, checked above, and written by the slot.
        let built = unsafe { out.read() };
        if status == Status::GUATIAO_OK && !built.is_null() {
            registry
                .instances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(built as usize, view.clone());
        }
        status
    })
}

/// Releases an instance [`guatiao_registry_provider_create`] built. Null
/// is a no-op; an unknown key or a provider that builds no instances does
/// nothing.
///
/// **The provider that built it releases it**, found by the instance
/// rather than by `key`: after its library was retired, or a new build of
/// it loaded under the same key by a refresh, the old build's `destroy`
/// is the one called. An instance keeps its library open until it is
/// released: `guatiao_registry_unload` answers `GUATIAO_ERR_BUSY`, and a
/// retired library is not closed. `key` is consulted only for an instance
/// this registry did not build.
///
/// # Safety
///
/// `reg` is a live handle, `key` is readable, and `instance` is null or
/// came from `guatiao_registry_provider_create` under the same key and is
/// not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_destroy(
    reg: *const HostRegistry,
    key: Str,
    instance: *mut c_void,
) {
    guard_with((), || {
        if instance.is_null() {
            return;
        }
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return;
        };
        let built = registry
            .instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(instance as usize));
        if let Some(view) = built {
            if let Some(destroy) = view.destroy {
                // SAFETY: the slot of the provider that built the instance,
                // whose library the view's lease keeps open.
                unsafe { destroy(view.ctx, instance) };
            }
            // The lease goes with the view, after the call.
            drop(view);
            return;
        }
        let Ok(key) = core::str::from_utf8(key.into()) else {
            return;
        };
        if let Some(provider) = registry.inner.provider(key)
            && let Some(destroy) = provider.view().destroy
        {
            // SAFETY: the slot is the descriptor's own and the instance is
            // the caller's, from `create`.
            unsafe { destroy(provider.view().ctx, instance) };
        }
    })
}

/// How this registry introduces itself to a library: a pointer to a block
/// that outlives the registry, carrying the host's id and version, its
/// allocator, and a `services` table a library calls to ask what is
/// loaded.
///
/// For a host that drives a library itself — calling
/// `guatiao_library_entry` by hand — this is the pointer to pass. The
/// loader passes it on every load. Leaked on first use, never freed, so a
/// library may keep it; after `guatiao_registry_free` its services answer
/// `GUATIAO_ERR_GONE`. Null for a null handle.
///
/// # Safety
///
/// `reg` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_host(reg: *mut HostRegistry) -> *const HostInfo<'static> {
    guard_with(core::ptr::null(), || {
        // SAFETY: the caller's contract.
        match unsafe { HostRegistry::get_mut(reg) } {
            Some(registry) => registry.inner.host().as_raw(),
            None => core::ptr::null(),
        }
    })
}

/// Takes one library out of this registry and **leaves it mapped**.
///
/// `key` is the library key — the template `guatiao_registry_libraries_keyed_by`
/// sets, `%id` by default. Its providers leave the registry and the
/// snapshot other libraries read, and the key may be loaded again.
/// Its configuration schema, which this registry owned, goes with it:
/// `guatiao_registry_provider_config` answers null for a retired provider.
///
/// **A retired library is closed once nothing uses it**: by
/// [`guatiao_registry_collect`], or by this registry's next load,
/// registration, retire or unload, as soon as no Rust handle and no
/// answer another library pinned through the host's services holds it
/// and its `unload` slot agrees. A table, vtable or `ctx` pointer this
/// caller took from it stays valid until then and not after: let it go
/// before any of those calls. A library the host links is let go here.
///
/// `GUATIAO_ERR_NOT_FOUND` when no library answers to `key`.
/// `guatiao_registry_unload` takes it out and closes it at once.
///
/// # Safety
///
/// `reg` is a live handle and `key` is readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_retire(reg: *mut HostRegistry, key: Str) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        match registry.inner.retire(key) {
            Ok(_) => Status::GUATIAO_OK,
            Err(why) => unload_status(&why),
        }
    })
}

/// Takes one library out of this registry **and unmaps it**, when
/// nothing uses it and the library agrees.
///
/// `GUATIAO_ERR_BUSY` while anything holding a lease on it is alive: a
/// Rust handle taken from this registry, or an answer another library
/// pinned through the host's services (`get_pinned`, `list_pinned`).
/// Then the library's `unload` slot is asked: its promise that what it can
/// account for is released. `GUATIAO_ERR_UNSUPPORTED` when it has no slot
/// ([`guatiao_registry_unload_unchecked`] is the caller insisting); the
/// library's own status when it refuses, `GUATIAO_ERR_BUSY` from one with
/// an instance or object still alive; `GUATIAO_ERR_WRONG_KIND` for a
/// library the host LINKS; `GUATIAO_ERR_NOT_FOUND` for an unknown key.
/// Every one of those leaves it registered and mapped.
/// `GUATIAO_ERR_INTERNAL` when the loader could not close the mapping, in
/// which case it is retired.
///
/// # Safety
///
/// `reg` is a live handle and `key` is readable for this call. What no
/// lease counts is the caller's word: no vtable, table or `ctx` pointer
/// taken from it, no value it built through its own allocator that its
/// slot does not track, and no answer another library holds unpinned, is
/// used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_unload(reg: *mut HostRegistry, key: Str) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        match registry.inner.unload(key) {
            Ok(()) => Status::GUATIAO_OK,
            Err(why) => unload_status(&why),
        }
    })
}

/// [`guatiao_registry_unload`] past the two refusals only the caller can
/// overrule: a lease still held, and a library with no `unload` slot. A
/// library that has a slot is still asked, and its refusal still stands.
///
/// # Safety
///
/// As [`guatiao_registry_unload`], and nothing the library handed out is
/// alive at all: no handle or pinned answer holding it, no instance, no
/// object, no value from its allocator, and no thread of its own still
/// running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_unload_unchecked(
    reg: *mut HostRegistry,
    key: Str,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: forwarded -- the caller stated the contract above.
        match unsafe { registry.inner.unload_unchecked(key) } {
            Ok(()) => Status::GUATIAO_OK,
            Err(why) => unload_status(&why),
        }
    })
}

/// Closes every retired library nothing uses any more, writing their
/// library keys to `out` as a list of texts, oldest first.
///
/// A retired library qualifies once nothing holding a lease on it is
/// alive (see [`guatiao_registry_unload`]); it is then asked through its
/// `unload` slot and closed when it agrees. One that refuses stays retired
/// and mapped and is asked again next time; one with no slot stays
/// mapped. `GUATIAO_ERR_ALLOC` for a null or incomplete `alloc`, as the
/// other answers.
///
/// # Safety
///
/// `reg` is a live handle, `alloc` is valid, and `out` addresses writable
/// storage for one value. As for [`guatiao_registry_unload`], no vtable,
/// table or `ctx` pointer taken from a retired library is used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_collect(
    reg: *mut HostRegistry,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.collect(alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// Releases a registry. Null is a no-op.
///
/// **The libraries it loaded stay mapped.** This frees the host's own
/// table and nothing else; `guatiao_registry_unload` and
/// `guatiao_registry_collect` are how a library is unmapped. The block
/// `guatiao_registry_host` handed out stays too, and answers
/// `GUATIAO_ERR_GONE` from then on.
///
/// # Safety
///
/// `reg` is null or a handle from [`guatiao_registry_new`] that has not
/// already been freed, and nothing else is using it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_free(reg: *mut HostRegistry) {
    if reg.is_null() {
        return;
    }
    guard_with((), || {
        // SAFETY: the caller's contract.
        drop(unsafe { Box::from_raw(reg) });
    });
}

/// Names providers with `template` instead of `%id`.
///
/// A template is text with fields: `%id`, `%version`, `%name` (its
/// display name), `%library` (its library's id) and `%source` (the source
/// that served its library, as a refresh reports it; empty for a library
/// loaded or registered by the host); `%%` is a literal `%`.
/// `%source/%id`, here and for libraries, holds the same library from
/// several sources, such as identical devices each reached through its own.
///
/// Anything already loaded is re-keyed. **A refusal changes nothing**: the
/// keys are all rendered and checked before any is written.
///
/// # Safety
///
/// `reg` is a live handle and `template` is readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_keyed_by(
    reg: *mut HostRegistry,
    template: Str,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let template = match core::str::from_utf8(template.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        registry.rekey(template, false)
    })
}

/// Names libraries with `template` instead of `%id` — the "how many builds
/// of one library may I hold" knob. A library's fields are `%id`,
/// `%version` and `%source`.
///
/// # Safety
///
/// As [`guatiao_registry_keyed_by`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_libraries_keyed_by(
    reg: *mut HostRegistry,
    template: Str,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let template = match core::str::from_utf8(template.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        registry.rekey(template, true)
    })
}

/// Loads one file, writing what happened to `out` as a map.
///
/// `{"loaded": <library>}`, `{"skipped": "<why>", "from": "<path>",
/// "abi": <n>}` with `from` and `abi` present only when the reason has
/// one, or `{"failed": "<message>"}`. **A skip is an answer, not a
/// failure**: the file is not a library (`no-entry-symbol`), the library
/// declined this host (`declined-this-host`), speaks another envelope
/// version (`unsupported-abi`), or this registry already has it
/// (`already-loaded`). A failure is a file the loader could not map or a
/// descriptor this build cannot read.
///
/// `GUATIAO_ERR_BAD_VALUE` when `path` is not UTF-8, and
/// `GUATIAO_ERR_ALLOC` when `alloc` is null or incomplete; `out` is
/// written ABSENT first, so either leaves it ABSENT.
///
/// **Mapping a library runs its static initialisers**, which may do
/// anything, including abort the process. Name files you are willing to
/// run.
///
/// # Safety
///
/// `reg` is a live handle, `path` and `alloc` are valid for this call, and
/// `out` addresses writable storage for one value, whose previous contents
/// are the caller's to have freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_load_file(
    reg: *mut HostRegistry,
    path: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let path = match core::str::from_utf8(path.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.load(path, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e,
        }
    })
}

/// Registers a library the host LINKS rather than loads: `entry` is its
/// `guatiao_library_entry`, called with this registry's host block, and
/// `name` stands in for the path (`<name>`) in every report. The report
/// is [`guatiao_registry_load_file`]'s: `loaded`, `skipped` or `failed`,
/// and so are its statuses: `GUATIAO_ERR_BAD_VALUE` for a `name` that is
/// not UTF-8, `GUATIAO_ERR_ALLOC` for a null or incomplete `alloc`.
///
/// # Safety
///
/// `reg` is a live handle, `name` and `alloc` are valid for this call,
/// `entry` is null or behaves as `guatiao_library_entry` -- answering
/// null or a descriptor well-formed for its own `struct_size` that lives
/// for the process -- and `out` addresses writable storage for one value,
/// whose previous contents are the caller's to have freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_register_entry(
    reg: *mut HostRegistry,
    name: Str,
    entry: Option<unsafe extern "C" fn(*const HostInfo<'static>) -> *const LibraryInfo<'static>>,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let name = match core::str::from_utf8(name.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        let Some(entry) = entry else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.register(name, entry, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e,
        }
    })
}

/// Adds a source the host LINKS, under `name`: a table that speaks
/// `guatiao_source_info`, read and checked now. Its slots and `ctx` must
/// stay valid for the registry's life; the struct itself is copied and
/// need not. Nothing is opened until [`guatiao_registry_refresh`].
///
/// `err` is written empty first and, on a refusal, carries words to free
/// like any text: `GUATIAO_ERR_BAD_VALUE` for a table the checks refuse or
/// a name already added, `GUATIAO_ERR_NULL` for a null table.
///
/// # Safety
///
/// `reg` is a live handle, `name` is readable for this call, `source` is
/// null or a well-formed table, `err` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_add_source(
    reg: *mut HostRegistry,
    name: Str,
    source: *const SourceInfo,
    err: *mut ProviderError,
) -> Status {
    if !err.is_null() {
        // SAFETY: non-null, and writable by the caller's contract; the
        // placeholder owns nothing.
        unsafe { err.write(ProviderError::none()) };
    }
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let name = match core::str::from_utf8(name.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or a well-formed table by the caller's contract.
        let Some(source) = (unsafe { source.as_ref() }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        match registry.inner.add_source_table(name, source) {
            Ok(()) => Status::GUATIAO_OK,
            Err(why) => source_refusal(err, &why),
        }
    })
}

/// Maps a source file and adds the source its `guatiao_source` answers
/// for `config`, under the file's path. The file is never unmapped, and
/// it is not a library: no key retires it. Nothing is opened until
/// [`guatiao_registry_refresh`].
///
/// `config` is a map the source reads (null is an empty one). `err` is
/// written empty first and, on a refusal, carries words to free like any
/// text: `GUATIAO_ERR_NOT_FOUND` when the file cannot be mapped,
/// `GUATIAO_ERR_WRONG_KIND` when it exports no `guatiao_source` or
/// `config` is not a map, `GUATIAO_ERR_BAD_VALUE` for a table the checks
/// refuse or a source already added, and the source's own status when it
/// refuses the configuration.
///
/// **Mapping a file runs its static initialisers**, which may do
/// anything. Name files you are willing to run.
///
/// # Safety
///
/// `reg` is a live handle, `path` is readable for this call, `config` is
/// null or a well-formed value, `err` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_add_source_file(
    reg: *mut HostRegistry,
    path: Str,
    config: *const Value,
    err: *mut ProviderError,
) -> Status {
    if !err.is_null() {
        // SAFETY: non-null, and writable by the caller's contract; the
        // placeholder owns nothing.
        unsafe { err.write(ProviderError::none()) };
    }
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let path = match core::str::from_utf8(path.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or a well-formed value by the caller's contract.
        let config = match unsafe { config.as_ref() } {
            None => Map::new(),
            Some(value) => match crate::value::convert::TryAsRef::<Map>::try_as_ref(value) {
                Some(map) => Map::clone(map),
                None => return Status::GUATIAO_ERR_WRONG_KIND,
            },
        };
        match registry.inner.add_source_file(Path::new(path), &config) {
            Ok(_) => Status::GUATIAO_OK,
            Err(why) => source_refusal(err, &why),
        }
    })
}

/// Watches a directory: each [`guatiao_registry_refresh`] loads a library
/// file that appeared there (by `rules`, as
/// [`guatiao_registry_scan_dir_rules`] takes them: one `[!]KEY=VALUE` per
/// line), retires one that vanished, and retires and loads again one whose
/// length or modification time changed. Nothing is read until then.
///
/// `GUATIAO_ERR_BAD_VALUE` for text that is not UTF-8, a rule that is not
/// one, or a directory already watched.
///
/// # Safety
///
/// `reg` is a live handle; `dir` and `rules` are readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_watch_dir(
    reg: *mut HostRegistry,
    dir: Str,
    rules: Str,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let (Ok(dir), Ok(rules)) = (
            core::str::from_utf8(dir.into()),
            core::str::from_utf8(rules.into()),
        ) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let lines: Vec<&str> = rules.lines().collect();
        let Ok(rules) = ScanRules::parse(&lines) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        match registry.inner.watch_dir(Path::new(dir), rules) {
            Ok(()) => Status::GUATIAO_OK,
            Err(why) => why.status(),
        }
    })
}

/// Asks every source and watched directory what it serves now, makes the
/// registry agree, and writes what changed to `out` as a map:
///
/// `{"loaded": [{"source", "id", "stamp", "key"}…], "retired": [the
/// same…], "skipped": [{"source", "id", "skipped", …}…], "refused":
/// [{"source", "id"?, "why"}…]}`.
///
/// A new candidate loads; a changed one loads and, once it has, its old
/// build is retired; a vanished one is retired; a library the host retired
/// stays out until its stamp changes. A retired library closes once
/// nothing holds it, and its source is then told. **Refusals are part of
/// the answer, never a failed call.** `GUATIAO_ERR_ALLOC` for a null or
/// incomplete `alloc`.
///
/// # Safety
///
/// As [`guatiao_registry_collect`]: no table or `ctx` taken from a library
/// this retires is used once it is closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_refresh(
    reg: *mut HostRegistry,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.refresh(alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// Scans a directory, writing a report to `out` as a map.
///
/// `{"loaded": [path…], "skipped": [{"skipped", "path", "from"?}…],
/// "failed": [{"path", "error"}…]}`.
///
/// Nothing is opened that has not first been shown, **by reading its
/// export table as data**, to declare the entry symbol — so a directory
/// full of ordinary libraries costs no static initialisers.
///
/// `descending` visits names highest-first, and under a `%id` library key
/// the first to claim a key keeps it. That is byte order, not version
/// order: `libfoo-1.2.0` sorts above `libfoo-1.10.0`. Nothing here parses
/// a version; a host wanting the newest keys libraries by
/// `%id@%version` and picks.
///
/// Statuses are [`guatiao_registry_load_file`]'s: `GUATIAO_ERR_BAD_VALUE`
/// for a `dir` that is not UTF-8, `GUATIAO_ERR_ALLOC` for a null or
/// incomplete `alloc`.
///
/// # Safety
///
/// As [`guatiao_registry_load_file`], with `dir` naming a directory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_scan_dir(
    reg: *mut HostRegistry,
    dir: Str,
    descending: bool,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let dir = match core::str::from_utf8(dir.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let order = if descending {
            Order::Descending
        } else {
            Order::Ascending
        };
        match registry.scan(dir, order, &ScanRules::default(), alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e,
        }
    })
}

/// [`guatiao_registry_scan_dir`] with rules applied to what each library
/// declares, **before it is mapped**.
///
/// `rules` is newline-separated: each line is `KEY=VALUE` to require a
/// declaration or `!KEY=VALUE` to skip a library that declares it; blank
/// lines are ignored. Every kind a library was built with is declared as
/// `kind=<name>`, so `kind=session-backend` scans for that kind alone.
/// A library skipped this way is reported as `{"skipped": "filtered",
/// "by": "<rule>"}`. A line that is not a rule is `GUATIAO_ERR_BAD_VALUE`.
///
/// # Safety
///
/// As [`guatiao_registry_scan_dir`], with `rules` a valid [`Str`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_scan_dir_rules(
    reg: *mut HostRegistry,
    dir: Str,
    descending: bool,
    rules: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let dir = match core::str::from_utf8(dir.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        let rules = match core::str::from_utf8(rules.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let Ok(rules) = ScanRules::parse(&rules.lines().collect::<Vec<_>>()) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let order = if descending {
            Order::Descending
        } else {
            Order::Ascending
        };
        match registry.scan(dir, order, &rules, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e,
        }
    })
}

/// Walks a search path — directories or files, separated by `;` on
/// Windows and `:` elsewhere, each visited once — under `rules`, and
/// writes one report for the whole path to `out`.
///
/// The report is [`guatiao_registry_scan_dir_rules`]'s, with one more
/// list when it applies: `"unreadable": [{"path", "error"}…]`, the
/// entries that do not exist or could not be listed. Those are reported
/// and the rest of the path is still walked; a search path routinely
/// names a place that is not on this machine. A file entry is probed and
/// loaded on its own, its extension unchecked; a `.framework` bundle is
/// its binary.
///
/// # Safety
///
/// As [`guatiao_registry_scan_dir_rules`], with `spec` a valid [`Str`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_scan_path(
    reg: *mut HostRegistry,
    spec: Str,
    descending: bool,
    rules: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let spec = match core::str::from_utf8(spec.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        let rules = match core::str::from_utf8(rules.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let Ok(rules) = ScanRules::parse(&rules.lines().collect::<Vec<_>>()) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let order = if descending {
            Order::Descending
        } else {
            Order::Ascending
        };
        match registry
            .scan_path(spec, order, &rules, alloc)
            .map_err(Status::from)
        {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e,
        }
    })
}

/// Every library loaded, as a list of maps.
///
/// `GUATIAO_ERR_ALLOC` when `alloc` is null or incomplete; `out` is
/// written ABSENT first, so it is left ABSENT. The providers, the
/// availability and the best-provider questions below answer the same
/// way, and add `GUATIAO_ERR_BAD_VALUE` for a `kind` or `key` that is
/// not UTF-8.
///
/// # Safety
///
/// `reg` is a live handle, `alloc` is valid, and `out` addresses writable
/// storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_libraries(
    reg: *const HostRegistry,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.libraries(alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// Every provider that serves `kind`, as a list of maps — or **every**
/// provider when `kind` is empty.
///
/// The capability question. [`guatiao_registry_provider`] is the identity
/// one.
///
/// # Safety
///
/// As [`guatiao_registry_libraries`], with `kind` readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_providers(
    reg: *const HostRegistry,
    kind: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let kind = match core::str::from_utf8(kind.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.providers(kind, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// One provider by the key this registry filed it under, as a map.
///
/// `GUATIAO_ERR_NOT_FOUND` when nothing answers to that key.
///
/// # Safety
///
/// As [`guatiao_registry_providers`], with `key` readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider(
    reg: *const HostRegistry,
    key: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.provider(key, alloc) {
            Some(built) => match built {
                // SAFETY: `out` is null or the caller's slot, written ABSENT on
                // entry; `alloc` is the caller's own allocator.
                Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_NOT_FOUND,
        }
    })
}

/// Every provider serving `kind` that **can actually run here**, as a list
/// of maps.
///
/// [`guatiao_registry_providers`] is who CLAIMS the kind; this is who can
/// serve it now. Each is asked at the moment of the call and nothing is
/// cached, so a provider whose optional dependency arrived or went away
/// since the last call answers differently — which is the point.
///
/// # Safety
///
/// As [`guatiao_registry_providers`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_available(
    reg: *const HostRegistry,
    kind: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let kind = match core::str::from_utf8(kind.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.available(kind, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// Why nothing can serve `kind`, as a map.
///
/// `{"available": true}` when something can. Otherwise
/// `{"available": false, "why": "nothing-claims-it"}` or
/// `{"available": false, "why": "none-available",
/// "providers": [{"id", "reason"}…]}`.
///
/// **The two refusals are kept apart because the remedies differ**:
/// install something, versus fix what you already have. A single
/// "unsupported" leaves a person with no idea which way to go.
///
/// # Safety
///
/// As [`guatiao_registry_providers`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_why_not(
    reg: *const HostRegistry,
    kind: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let kind = match core::str::from_utf8(kind.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.why_not(kind, alloc) {
            // SAFETY: `out` is null or the caller's slot, written ABSENT on
            // entry; `alloc` is the caller's own allocator.
            Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
            Err(e) => e.into(),
        }
    })
}

/// Whether one provider can run here, writing its reason through `reason`
/// when it cannot.
///
/// `true` when it can, or when no provider answers to `key` — a caller
/// that cares about the difference has [`guatiao_registry_provider`],
/// which reports `NOT_FOUND`. `reason` may be null, and is written only on
/// a refusal; what it points at lives as long as the library.
///
/// **Asked every time, never cached.**
///
/// # Safety
///
/// `reg` is a live handle, `key` is readable for the call, and `reason` is
/// null or addresses writable storage for one `guatiao_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_available(
    reg: *const HostRegistry,
    key: Str,
    reason: *mut Str,
) -> bool {
    guard_with(true, || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(key.into()),
        ) else {
            return true;
        };
        match registry.provider_available(key) {
            Some(Err(why)) => {
                if !reason.is_null() {
                    // SAFETY: the caller's contract says it is writable.
                    unsafe { reason.write(Str::new(why)) };
                }
                false
            }
            _ => true,
        }
    })
}

/// Ranks every provider with this id, now and whenever one loads.
///
/// **This is how a host chooses between two implementations of one
/// kind.** `guatiao_registry_providers` and `_available` answer best
/// first, which is `(priority DESC, key ASC)` — the key tiebreak because
/// load order follows directory iteration, which no filesystem promises
/// to keep stable, so "whichever loaded first" is not a rule anyone can
/// reproduce.
///
/// Absent means 0, so an unranked provider sorts below any raised one and
/// alongside every other unranked one. Negative ranks below them all.
///
/// A frontend reads its own configuration and calls this; nothing in this
/// library reads a file.
///
/// # Safety
///
/// `reg` is a live handle and `id` is readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_set_priority(
    reg: *mut HostRegistry,
    id: Str,
    priority: i32,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get_mut(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let id = match core::str::from_utf8(id.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        registry.set_priority(id, priority);
        Status::GUATIAO_OK
    })
}

/// What this host ranked that id. Zero unless it said otherwise, and zero
/// for a null handle — a rank is not a lookup, and there is no answer to
/// distinguish "unranked" from.
///
/// # Safety
///
/// `reg` is a live handle and `id` is readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_priority(reg: *const HostRegistry, id: Str) -> i32 {
    guard_with(0, || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(id)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(id.into()),
        ) else {
            return 0;
        };
        registry.priority(id)
    })
}

/// The best provider serving `kind` that can actually run here, as a map.
///
/// `GUATIAO_ERR_NOT_FOUND` when nothing can. The question a host usually
/// has; `guatiao_registry_available` is how to see what it passed over,
/// so a frontend can say "ssh -> openssh (also: putty)".
///
/// # Safety
///
/// As [`guatiao_registry_providers`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_best(
    reg: *const HostRegistry,
    kind: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let kind = match core::str::from_utf8(kind.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.best(kind, alloc) {
            Some(built) => match built {
                // SAFETY: `out` is null or the caller's slot, written ABSENT on
                // entry; `alloc` is the caller's own allocator.
                Ok(answer) => unsafe { answer.write_to(out, alloc.as_raw()) },
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_NOT_FOUND,
        }
    })
}

/// One provider's function table, and the size the library compiled it at.
///
/// Null when no provider answers to `key`, or when it declares no table.
/// `size_out` may be null; when it is not, it receives the declared size.
///
/// **This is the moment a caller takes on the kind's contract.** The
/// envelope defines no vtable: check the declared size against the frozen
/// floor of the `kind` you believe this is, project each field through the
/// pointer, and guard every field appended after that floor. A shorter
/// table than you expect is an OLDER library, which is the case the size
/// exists to let you support rather than reject.
///
/// The pointer is this registry's copy of the library's table, valid until
/// that provider's library is closed: unloaded, or collected once retired.
///
/// # Safety
///
/// `reg` is a live handle, `key` is readable for the call, and `size_out`
/// is null or addresses writable storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_vtable(
    reg: *const HostRegistry,
    key: Str,
    size_out: *mut usize,
) -> *const c_void {
    guard_with(core::ptr::null(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(key.into()),
        ) else {
            return core::ptr::null();
        };
        let (ptr, size) = registry.vtable(key).unwrap_or((core::ptr::null(), 0));
        if !size_out.is_null() {
            // SAFETY: the caller's contract says it is writable.
            unsafe { size_out.write(size) };
        }
        ptr
    })
}

/// The table one provider speaks `kind` through, and the size the library
/// compiled it at: its per-kind table when it declares one, else its single
/// table when it claims the kind.
///
/// Null when no provider answers to `key`, or it does not serve `kind`
/// through a table. `size_out` may be null. Check the table's
/// `floor_hash` against `<table>_FLOOR_HASH` from the kind's header, and
/// the size against the fields you read, before calling through it; pass
/// [`guatiao_registry_provider_ctx`] as every slot's `ctx`.
///
/// The pointer is this registry's copy of the library's table, valid until
/// that provider's library is closed: unloaded, or collected once retired.
///
/// # Safety
///
/// `reg` is a live handle, `key` and `kind` are readable for the call, and
/// `size_out` is null or addresses writable storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_table(
    reg: *const HostRegistry,
    key: Str,
    kind: Str,
    size_out: *mut usize,
) -> *const c_void {
    guard_with(core::ptr::null(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key), Ok(kind)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(key.into()),
            core::str::from_utf8(kind.into()),
        ) else {
            return core::ptr::null();
        };
        let (ptr, size) = registry
            .table_for(key, kind)
            .unwrap_or((core::ptr::null(), 0));
        if !size_out.is_null() {
            // SAFETY: the caller's contract says it is writable.
            unsafe { size_out.write(size) };
        }
        ptr
    })
}

/// The context pointer to hand back to every call through that provider's
/// table. Null is a legitimate answer and means the provider needs none.
///
/// # Safety
///
/// `reg` is a live handle and `key` is readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_ctx(
    reg: *const HostRegistry,
    key: Str,
) -> *mut c_void {
    guard_with(core::ptr::null_mut(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(key.into()),
        ) else {
            return core::ptr::null_mut();
        };
        registry.ctx(key).unwrap_or(core::ptr::null_mut())
    })
}

/// One provider's configuration schema, **borrowed** from the registry,
/// or null when the provider declares none.
///
/// Read it with the ordinary value readers: a schema is a value. Do not
/// free it — it is not yours. It is the registry's own copy of what the
/// library declared, valid until that provider's library is retired or
/// the registry is freed.
///
/// # Safety
///
/// `reg` is a live handle and `key` is readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_config(
    reg: *const HostRegistry,
    key: Str,
) -> *const Value {
    guard_with(core::ptr::null(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (
            unsafe { HostRegistry::get(reg) },
            core::str::from_utf8(key.into()),
        ) else {
            return core::ptr::null();
        };
        registry
            .config(key)
            .map_or(core::ptr::null(), |v| v as *const Value)
    })
}

/// One provider's `meta` map, copied into `alloc`: whatever else it
/// declared, such as its legal notices or, under `"form"`, how a person is
/// shown its configuration.
///
/// `out` holds the map, or stays ABSENT when the provider declares no
/// `meta`; both answer `GUATIAO_OK`. `GUATIAO_ERR_NOT_FOUND` when nothing
/// answers to `key`. A copy rather than a borrow, unlike
/// [`guatiao_registry_provider_config`]: the caller frees it with
/// `guatiao_value_free` and may keep it past the library.
///
/// # Safety
///
/// As [`guatiao_registry_provider`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_meta(
    reg: *const HostRegistry,
    key: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(registry) = (unsafe { HostRegistry::get(reg) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let key = match core::str::from_utf8(key.into()) {
            Ok(t) => t,
            Err(e) => return e.into(),
        };
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match registry.meta(key) {
            None => Status::GUATIAO_ERR_NOT_FOUND,
            Some(None) => Status::GUATIAO_OK,
            Some(Some(meta)) => match meta.clone_in(alloc) {
                // SAFETY: `out` is null or the caller's slot, written ABSENT
                // on entry; the copy already records `alloc`.
                Ok(copy) => unsafe { Value::from(copy).write_to(out, alloc.as_raw()) },
                Err(e) => e.into(),
            },
        }
    })
}

// --- building the answers ----------------------------------------------

/// One loaded library, as a map.
fn library_value(alloc: Alloc, one: &crate::library::Loaded) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    map.try_set("key", text_value(alloc, &one.key)?)?;
    map.try_set("id", Text::new_in(alloc, &one.id).map(Value::from)?)?;
    map.try_set(
        "version",
        Text::new_in(alloc, &one.version).map(Value::from)?,
    )?;
    map.try_set(
        "path",
        Text::new_in(alloc, &one.path.to_string_lossy()).map(Value::from)?,
    )?;
    map.try_set("source", Text::new_in(alloc, &one.source).map(Value::from)?)?;
    map.try_set("providers", one.providers)?;

    // What it offered that this host already had. Empty on an ordinary
    // load, and written even then: absent and empty would otherwise be the
    // same answer.
    let mut skipped = List::new_in(alloc);
    for why in &one.skipped {
        skipped.try_push(skip_value(alloc, why)?)?;
    }
    map.try_set("skipped", skipped)?;
    Ok(map.into())
}

/// One provider, as a map. Everything about it that is data; the vtable
/// and `ctx` have their own accessors because they are not.
fn provider_value(alloc: Alloc, one: &Provider) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    map.try_set("key", Text::new_in(alloc, one.key()).map(Value::from)?)?;
    map.try_set("id", Text::new_in(alloc, one.id()).map(Value::from)?)?;
    map.try_set(
        "version",
        Text::new_in(alloc, one.version()).map(Value::from)?,
    )?;
    map.try_set(
        "library",
        Text::new_in(alloc, one.library()).map(Value::from)?,
    )?;
    map.try_set(
        "display_name",
        Text::new_in(alloc, one.display_name()).map(Value::from)?,
    )?;
    map.try_set(
        "from",
        Text::new_in(alloc, &one.from().to_string_lossy()).map(Value::from)?,
    )?;
    map.try_set(
        "source",
        Text::new_in(alloc, one.source()).map(Value::from)?,
    )?;

    let mut kinds = List::new_in(alloc);
    for kind in one.kinds() {
        kinds.try_push(Text::new_in(alloc, kind).map(Value::from)?)?;
    }
    map.try_set("kinds", kinds)?;

    // Whether there is something to fetch, rather than the thing itself: a
    // schema is borrowed from the library's image, and copying one into
    // every listing would be a tree per provider nobody asked for.
    map.try_set("priority", one.priority())?;
    map.try_set("has_config", Value::from(one.config_schema().is_some()))?;
    map.try_set("vtable_size", one.vtable().1)?;
    Ok(map.into())
}

/// A scan's report, as a map: `{"loaded": [path…], "skipped": [{"skipped",
/// "path", …}…], "failed": [{"path", "error"}…]}`, plus `"unreadable":
/// [{"path", "error"}…]` when a search path named a place that could not
/// be read.
fn report_value(alloc: Alloc, report: &LoadReport) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);

    let mut loaded = List::new_in(alloc);
    for (path, _) in &report.loaded {
        loaded.try_push(Text::new_in(alloc, &path.to_string_lossy()).map(Value::from)?)?;
    }
    map.try_set("loaded", loaded)?;

    let mut skipped = List::new_in(alloc);
    for (path, why) in &report.skipped {
        let mut one = Map::try_from(skip_value(alloc, why)?).map_err(|_| ValueError::WrongKind)?;
        one.set_in("path", Text::new_in(alloc, &path.to_string_lossy())?, alloc)?;
        skipped.try_push(one)?;
    }
    map.try_set("skipped", skipped)?;

    let mut failed = List::new_in(alloc);
    for (path, error) in &report.failed {
        let mut one = Map::new_in(alloc);
        one.try_set(
            "path",
            Text::new_in(alloc, &path.to_string_lossy()).map(Value::from)?,
        )?;
        one.try_set(
            "error",
            Text::new_in(alloc, &error.to_string()).map(Value::from)?,
        )?;
        failed.try_push(one)?;
    }
    map.try_set("failed", failed)?;

    // Only when there is something to say: a single-directory scan never
    // has any, and a key that is always present and usually empty is a
    // key every reader has to know about.
    if !report.unreadable.is_empty() {
        let mut unreadable = List::new_in(alloc);
        for (path, error) in &report.unreadable {
            let mut one = Map::new_in(alloc);
            one.try_set(
                "path",
                Text::new_in(alloc, &path.to_string_lossy()).map(Value::from)?,
            )?;
            one.try_set(
                "error",
                Text::new_in(alloc, &error.to_string()).map(Value::from)?,
            )?;
            unreadable.try_push(one)?;
        }
        map.try_set("unreadable", unreadable)?;
    }
    Ok(map.into())
}

/// What a refresh changed, as a map.
fn refreshed_value(alloc: Alloc, report: &Refreshed) -> Result<Value, ValueError> {
    let served = |one: &Served| -> Result<Map, ValueError> {
        let mut map = Map::new_in(alloc);
        map.try_set("source", Text::new_in(alloc, &one.source).map(Value::from)?)?;
        map.try_set("id", Text::new_in(alloc, &one.id).map(Value::from)?)?;
        map.try_set("stamp", Text::new_in(alloc, &one.stamp).map(Value::from)?)?;
        map.try_set("key", Text::new_in(alloc, &one.key).map(Value::from)?)?;
        Ok(map)
    };
    let mut map = Map::new_in(alloc);
    for (name, list) in [("loaded", &report.loaded), ("retired", &report.retired)] {
        let mut out = List::new_in(alloc);
        for one in list {
            out.try_push(served(one)?)?;
        }
        map.try_set(name, out)?;
    }
    let mut skipped = List::new_in(alloc);
    for (source, id, why) in &report.skipped {
        let mut one = Map::try_from(skip_value(alloc, why)?).map_err(|_| ValueError::WrongKind)?;
        one.set_in("source", Text::new_in(alloc, source)?, alloc)?;
        one.set_in("id", Text::new_in(alloc, id)?, alloc)?;
        skipped.try_push(one)?;
    }
    map.try_set("skipped", skipped)?;
    let mut refused = List::new_in(alloc);
    for one in &report.refused {
        let mut entry = Map::new_in(alloc);
        entry.try_set("source", Text::new_in(alloc, &one.source).map(Value::from)?)?;
        if let Some(id) = &one.id {
            entry.try_set("id", Text::new_in(alloc, id).map(Value::from)?)?;
        }
        entry.try_set("why", Text::new_in(alloc, &one.reason).map(Value::from)?)?;
        refused.try_push(entry)?;
    }
    map.try_set("refused", refused)?;
    Ok(map.into())
}

/// Why something was passed over, as a map.
fn skip_value(alloc: Alloc, why: &Skipped) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    let (name, from, id, abi, by) = match why {
        Skipped::NoEntrySymbol => ("no-entry-symbol", None, None, None, None),
        Skipped::DeclinedThisHost => ("declined-this-host", None, None, None, None),
        Skipped::NotExaminable => ("not-examinable", None, None, None, None),
        Skipped::UnsupportedAbi { declared } => {
            ("unsupported-abi", None, None, Some(*declared), None)
        }
        Skipped::AlreadyLoaded { from } => ("already-loaded", Some(from), None, None, None),
        Skipped::ProviderAlreadyLoaded { id, from } => (
            "provider-already-loaded",
            Some(from),
            Some(id.as_str()),
            None,
            None,
        ),
        Skipped::Filtered { by } => ("filtered", None, None, None, Some(by.as_str())),
        Skipped::Source { .. } => ("source", None, None, None, None),
        // No wildcard arm. `Skipped` is `#[non_exhaustive]` for a
        // CONSUMER; in here every variant is known, so appending one is a
        // compile error at this match rather than a reason that silently
        // crosses the boundary unnamed.
    };
    map.try_set("skipped", Text::new_in(alloc, name).map(Value::from)?)?;
    if let Some(from) = from {
        map.try_set(
            "from",
            Text::new_in(alloc, &from.to_string_lossy()).map(Value::from)?,
        )?;
    }
    if let Some(id) = id {
        map.try_set("id", Text::new_in(alloc, id).map(Value::from)?)?;
    }
    if let Some(abi) = abi {
        map.try_set("abi", abi)?;
    }
    if let Some(by) = by {
        map.try_set("by", Text::new_in(alloc, by).map(Value::from)?)?;
    }
    if let Skipped::Source { serves } = why {
        map.try_set("serves", Text::new_in(alloc, serves).map(Value::from)?)?;
    }
    Ok(map.into())
}

/// A [`Text`] this crate owns, copied into the caller's allocator.
///
/// The copy is the point: the answer is the caller's to free, and it must
/// not borrow a key the registry may re-render.
fn text_value(alloc: Alloc, text: &Text) -> Result<Value, ValueError> {
    Text::new_in(alloc, text).map(Value::from)
}
