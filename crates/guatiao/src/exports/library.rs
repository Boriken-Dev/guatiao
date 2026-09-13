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

use std::ffi::c_void;
use std::path::Path;

use super::{as_str, guard, guard_with};
use crate::library::{
    HostInfo, Loading, Order, Provider, Registry, ScanRules, Skipped, WhyNot, scan_dir_rules,
};
use crate::value::ValueError;
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Str, Text, Value};

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
        let mut map = Value::map_in(alloc);
        match self.inner.load_file(Path::new(path)) {
            Ok(Loading::Loaded(one)) => {
                let described = library_value(alloc, one)?;
                map.set("loaded", described)?;
            }
            Ok(Loading::Skipped(why)) => return Ok(skip_value(alloc, &why)?),
            Err(e) => map.set("failed", e.to_string().as_str())?,
        }
        Ok(map)
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

        let mut map = Value::map_in(alloc);

        let mut loaded = Value::list_in(alloc);
        for path in &report.loaded {
            loaded.push(Value::string_in(alloc, &path.to_string_lossy())?)?;
        }
        map.set("loaded", loaded)?;

        let mut skipped = Value::list_in(alloc);
        for (path, why) in &report.skipped {
            let mut one = skip_value(alloc, why)?;
            one.set("path", Value::string_in(alloc, &path.to_string_lossy())?)?;
            skipped.push(one)?;
        }
        map.set("skipped", skipped)?;

        let mut failed = Value::list_in(alloc);
        for (path, error) in &report.failed {
            let mut one = Value::map_in(alloc);
            one.set("path", Value::string_in(alloc, &path.to_string_lossy())?)?;
            one.set("error", Value::string_in(alloc, &error.to_string())?)?;
            failed.push(one)?;
        }
        map.set("failed", failed)?;
        Ok(map)
    }

    /// Every library loaded, as a list of maps.
    fn libraries(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = Value::list_in(alloc);
        for one in self.inner.loaded() {
            list.push(library_value(alloc, one)?)?;
        }
        Ok(list)
    }

    /// Every provider serving `kind`, or all of them when it is empty,
    /// best first.
    fn providers(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = Value::list_in(alloc);
        let ranked: Vec<&Provider> = if kind.is_empty() {
            self.inner.all_ranked().collect()
        } else {
            self.inner.providers(kind).collect()
        };
        for provider in ranked {
            list.push(provider_value(alloc, provider)?)?;
        }
        Ok(list)
    }

    /// One provider by key, or `None` when nothing answers to it.
    fn provider(&self, key: &str, alloc: Alloc) -> Option<Result<Value, ValueError>> {
        self.inner.provider(key).map(|p| provider_value(alloc, p))
    }

    /// Every provider serving `kind` that can actually run here.
    fn available(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = Value::list_in(alloc);
        for provider in self.inner.available(kind) {
            list.push(provider_value(alloc, provider)?)?;
        }
        Ok(list)
    }

    /// Why nothing can serve `kind`, or that something can.
    fn why_not(&self, kind: &str, alloc: Alloc) -> Result<Value, ValueError> {
        let mut map = Value::map_in(alloc);
        let Some(why) = self.inner.why_not(kind) else {
            map.set("available", Value::bool(true))?;
            return Ok(map);
        };
        map.set("available", Value::bool(false))?;
        match why {
            WhyNot::NothingClaimsIt => {
                map.set("why", Value::string_in(alloc, "nothing-claims-it")?)?;
            }
            WhyNot::NoneAvailable(refused) => {
                map.set("why", Value::string_in(alloc, "none-available")?)?;
                let mut list = Value::list_in(alloc);
                for (provider, reason) in refused {
                    let mut one = Value::map_in(alloc);
                    one.set("id", Value::string_in(alloc, provider.id())?)?;
                    one.set("reason", Value::string_in(alloc, reason)?)?;
                    list.push(one)?;
                }
                map.set("providers", list)?;
            }
        }
        Ok(map)
    }

    /// Whether one provider can run here, and why not when it cannot.
    fn provider_available(&self, key: &str) -> Option<Result<(), &'static str>> {
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

    /// One provider's context pointer.
    fn ctx(&self, key: &str) -> Option<*mut c_void> {
        self.inner.provider(key).map(Provider::ctx)
    }

    /// One provider's configuration schema, borrowed from its library.
    fn config(&self, key: &str) -> Option<&'static Value> {
        self.inner.provider(key).and_then(Provider::config_schema)
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
    guard_with(std::ptr::null_mut(), || {
        // SAFETY: the caller's contract.
        let (Ok(id), Ok(version)) = (unsafe { as_str(id) }, unsafe { as_str(version) }) else {
            return std::ptr::null_mut();
        };
        // Null is "no allocator", which is an answer. A malformed one is
        // not, and is refused rather than ignored.
        let alloc = if alloc.is_null() {
            None
        } else {
            // SAFETY: the caller's contract.
            match unsafe { Alloc::from_raw(alloc) } {
                Ok(a) => Some(a),
                Err(_) => return std::ptr::null_mut(),
            }
        };
        Box::into_raw(Box::new(HostRegistry {
            inner: Registry::with_alloc(id, version, alloc),
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key), Some(config), false) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(key) },
            unsafe { config.as_ref() },
            out.is_null(),
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let Some(provider) = registry.inner.provider(key) else {
            return Status::GUATIAO_ERR_NOT_FOUND;
        };
        let view = provider.view();
        let Some(create) = view.create else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: the slot is the descriptor's own; the pointers are the
        // caller's, checked above; the library is never unloaded.
        unsafe { create(view.ctx, config, out, err) }
    })
}

/// Releases an instance [`guatiao_registry_provider_create`] built. Null
/// is a no-op; an unknown key or a provider that builds no instances does
/// nothing.
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
        let (Some(registry), Ok(key)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(key) })
        else {
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
pub unsafe extern "C" fn guatiao_registry_host(reg: *mut HostRegistry) -> *const HostInfo {
    guard_with(std::ptr::null(), || {
        // SAFETY: the caller's contract.
        match unsafe { HostRegistry::get_mut(reg) } {
            Some(registry) => registry.inner.host().as_raw(),
            None => std::ptr::null(),
        }
    })
}

/// Releases a registry. Null is a no-op.
///
/// **The libraries it loaded stay mapped.** Nothing in this crate unloads
/// one, because every tree, string and vtable they handed over points into
/// their images; this frees the host's own table and nothing else. The
/// block `guatiao_registry_host` handed out stays too, and answers
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
        let (Some(registry), Ok(template)) = (unsafe { HostRegistry::get_mut(reg) }, unsafe {
            as_str(template)
        }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        registry.rekey(template, false)
    })
}

/// Names libraries with `template` instead of `%id` — the "how many builds
/// of one library may I hold" knob.
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
        let (Some(registry), Ok(template)) = (unsafe { HostRegistry::get_mut(reg) }, unsafe {
            as_str(template)
        }) else {
            return Status::GUATIAO_ERR_NULL;
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(path), Ok(alloc)) = (
            unsafe { HostRegistry::get_mut(reg) },
            unsafe { as_str(path) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.load(path, alloc)) }
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
/// `descending` visits names highest-first, which under a `%id` library
/// key is how a host takes the newest of several builds. That is byte
/// order, not version order; nothing here parses a version.
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(dir), Ok(alloc)) = (
            unsafe { HostRegistry::get_mut(reg) },
            unsafe { as_str(dir) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let order = if descending {
            Order::Descending
        } else {
            Order::Ascending
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.scan(dir, order, &ScanRules::default(), alloc)) }
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(dir), Ok(rules), Ok(alloc)) = (
            unsafe { HostRegistry::get_mut(reg) },
            unsafe { as_str(dir) },
            unsafe { as_str(rules) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let Ok(rules) = ScanRules::parse(&rules.lines().collect::<Vec<_>>()) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let order = if descending {
            Order::Descending
        } else {
            Order::Ascending
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.scan(dir, order, &rules, alloc)) }
    })
}

/// Every library loaded, as a list of maps.
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(alloc)) = (unsafe { HostRegistry::get(reg) }, unsafe {
            Alloc::from_raw(alloc)
        }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.libraries(alloc)) }
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(kind), Ok(alloc)) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(kind) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.providers(kind, alloc)) }
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key), Ok(alloc)) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(key) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        match registry.provider(key, alloc) {
            // SAFETY: as above.
            Some(built) => unsafe { deliver(out, built) },
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(kind), Ok(alloc)) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(kind) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.available(kind, alloc)) }
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(kind), Ok(alloc)) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(kind) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: as above.
        unsafe { deliver(out, registry.why_not(kind, alloc)) }
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
        let (Some(registry), Ok(key)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(key) })
        else {
            return true;
        };
        match registry.provider_available(key) {
            Some(Err(why)) => {
                if !reason.is_null() {
                    // SAFETY: the caller's contract says it is writable.
                    unsafe { reason.write(Str::borrowed(why)) };
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
        let (Some(registry), Ok(id)) =
            (unsafe { HostRegistry::get_mut(reg) }, unsafe { as_str(id) })
        else {
            return Status::GUATIAO_ERR_NULL;
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
        let (Some(registry), Ok(id)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(id) })
        else {
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
    guard(|| {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(kind), Ok(alloc)) = (
            unsafe { HostRegistry::get(reg) },
            unsafe { as_str(kind) },
            unsafe { Alloc::from_raw(alloc) },
        ) else {
            return Status::GUATIAO_ERR_NULL;
        };
        match registry.best(kind, alloc) {
            // SAFETY: as above.
            Some(built) => unsafe { deliver(out, built) },
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
/// The pointer stays valid for the life of the process, because a loaded
/// library is never unloaded.
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
    guard_with(std::ptr::null(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(key) })
        else {
            return std::ptr::null();
        };
        let (ptr, size) = registry.vtable(key).unwrap_or((std::ptr::null(), 0));
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
    guard_with(std::ptr::null_mut(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(key) })
        else {
            return std::ptr::null_mut();
        };
        registry.ctx(key).unwrap_or(std::ptr::null_mut())
    })
}

/// One provider's configuration schema, **borrowed** from the library's
/// own image, or null when it declares none.
///
/// Read it with the ordinary value readers: a schema is a value. Do not
/// free it — it is not yours, and it lives as long as the process.
///
/// # Safety
///
/// `reg` is a live handle and `key` is readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_registry_provider_config(
    reg: *const HostRegistry,
    key: Str,
) -> *const Value {
    guard_with(std::ptr::null(), || {
        // SAFETY: the caller's contract.
        let (Some(registry), Ok(key)) = (unsafe { HostRegistry::get(reg) }, unsafe { as_str(key) })
        else {
            return std::ptr::null();
        };
        registry
            .config(key)
            .map_or(std::ptr::null(), |v| v as *const Value)
    })
}

// --- writing an answer out ----------------------------------------------

/// Writes a built value to an out parameter, or reports why not.
///
/// # Safety
///
/// `out` is null, or addresses writable storage for one value whose
/// previous contents are the caller's to have freed.
unsafe fn deliver<E: Into<Status>>(out: *mut Value, built: Result<Value, E>) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    match built {
        Ok(value) => {
            // SAFETY: the caller's contract.
            unsafe { out.write(value) };
            Status::GUATIAO_OK
        }
        Err(e) => e.into(),
    }
}

// --- building the answers ----------------------------------------------

/// One loaded library, as a map.
fn library_value(alloc: Alloc, one: &crate::library::Loaded) -> Result<Value, ValueError> {
    let mut map = Value::map_in(alloc);
    map.set("key", text_value(alloc, &one.key)?)?;
    map.set("id", Value::string_in(alloc, one.id)?)?;
    map.set("version", Value::string_in(alloc, one.version)?)?;
    map.set(
        "path",
        Value::string_in(alloc, &one.path.to_string_lossy())?,
    )?;
    map.set("providers", Value::int(one.providers as i64))?;

    // What it offered that this host already had. Empty on an ordinary
    // load, and written even then: absent and empty would otherwise be the
    // same answer.
    let mut skipped = Value::list_in(alloc);
    for why in &one.skipped {
        skipped.push(skip_value(alloc, why)?)?;
    }
    map.set("skipped", skipped)?;
    Ok(map)
}

/// One provider, as a map. Everything about it that is data; the vtable
/// and `ctx` have their own accessors because they are not.
fn provider_value(alloc: Alloc, one: &Provider) -> Result<Value, ValueError> {
    let mut map = Value::map_in(alloc);
    map.set("key", Value::string_in(alloc, one.key())?)?;
    map.set("id", Value::string_in(alloc, one.id())?)?;
    map.set("version", Value::string_in(alloc, one.version())?)?;
    map.set("library", Value::string_in(alloc, one.library())?)?;
    map.set("display_name", Value::string_in(alloc, one.display_name())?)?;
    map.set(
        "from",
        Value::string_in(alloc, &one.from().to_string_lossy())?,
    )?;

    let mut kinds = Value::list_in(alloc);
    for kind in one.kinds() {
        kinds.push(Value::string_in(alloc, kind)?)?;
    }
    map.set("kinds", kinds)?;

    // Whether there is something to fetch, rather than the thing itself: a
    // schema is borrowed from the library's image, and copying one into
    // every listing would be a tree per provider nobody asked for.
    map.set("priority", Value::int(i64::from(one.priority())))?;
    map.set("has_config", Value::bool(one.config_schema().is_some()))?;
    map.set("vtable_size", Value::int(one.vtable().1 as i64))?;
    Ok(map)
}

/// Why something was passed over, as a map.
fn skip_value(alloc: Alloc, why: &Skipped) -> Result<Value, ValueError> {
    let mut map = Value::map_in(alloc);
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
        // No wildcard arm. `Skipped` is `#[non_exhaustive]` for a
        // CONSUMER; in here every variant is known, so appending one is a
        // compile error at this match rather than a reason that silently
        // crosses the boundary unnamed.
    };
    map.set("skipped", Value::string_in(alloc, name)?)?;
    if let Some(from) = from {
        map.set("from", Value::string_in(alloc, &from.to_string_lossy())?)?;
    }
    if let Some(id) = id {
        map.set("id", Value::string_in(alloc, id)?)?;
    }
    if let Some(abi) = abi {
        map.set("abi", Value::int(i64::from(abi)))?;
    }
    if let Some(by) = by {
        map.set("by", Value::string_in(alloc, by)?)?;
    }
    Ok(map)
}

/// A [`Text`] this crate owns, copied into the caller's allocator.
///
/// The copy is the point: the answer is the caller's to free, and it must
/// not borrow a key the registry may re-render.
fn text_value(alloc: Alloc, text: &Text) -> Result<Value, ValueError> {
    Value::string_in(alloc, text.as_str().unwrap_or_default())
}
