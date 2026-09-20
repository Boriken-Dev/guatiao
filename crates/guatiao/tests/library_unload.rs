// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Taking a library back out: retiring it, and unloading it on the host's
//! word.
//!
//! `hello_library` is a real `cdylib`, so these run against a real
//! mapping rather than a descriptor built in this process.
//!
//! Whether the mapping is genuinely gone after an unload is measured in
//! `library_unmaps.rs`, which needs a process of its own.

#![cfg(feature = "load")]

use std::path::PathBuf;

use guatiao::library::{LibraryInfo, Registry, Skipped, UnloadError};
use guatiao::value::status::Status;

/// One load at a time across this binary's tests.
///
/// These map and unmap the same file. A test unloading it while another
/// is calling through a table it took from the same mapping would be
/// reading freed code.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static LOADS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOADS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where cargo put the example library. As `library_load.rs` finds it:
/// from this binary's own path, because an integration test gets no
/// `OUT_DIR` and the example is a dev-dependency already built beside us.
fn library_path() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}hello_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("{name} is not beside {}", deps.display());
}

/// Retiring takes the library's providers with it, and the key is free
/// again.
#[test]
fn retiring_takes_the_providers_and_frees_the_key() {
    let _one = one_at_a_time();
    let path = library_path();
    let mut registry = Registry::new("retire-tests", "1.0");
    registry
        .load_file(&path)
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");
    assert!(registry.providers("greeter").next().is_some());

    let retired = registry.retire("hello_library").expect("it is loaded");
    assert_eq!(retired.id, "hello_library");
    assert_eq!(retired.providers, 4);
    assert!(registry.providers("greeter").next().is_none());
    assert!(registry.loaded().is_empty());
    assert!(registry.provider("hello_library_greeter").is_none());
    assert!(registry.all().is_empty());

    // What a library asks the host for is the same snapshot, republished.
    // `Host::get` reports `GUATIAO_ERR_NOT_FOUND` as `Ok(None)`.
    let host = registry.host();
    assert!(
        host.get("hello_library_greeter")
            .expect("the services answer")
            .is_none()
    );
    assert!(
        host.list("greeter")
            .expect("the services answer")
            .is_empty()
    );

    // And the key is free: a retired library is not `AlreadyLoaded`.
    let again = registry.load_file(&path).expect("it loads a second time");
    assert!(
        again.loaded().is_some(),
        "a retired library may be loaded again"
    );
}

/// Only the retired library's providers leave; a second library keeps
/// its own, and the key map still answers for them.
#[test]
fn retiring_one_library_leaves_the_others() {
    let _one = one_at_a_time();
    let mut registry = Registry::new("retire-tests", "1.0");
    registry
        .register_entry("cli", describing)
        .expect("a linked library registers")
        .loaded()
        .expect("it accepts this host");
    registry
        .load_file(&library_path())
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");

    registry.retire("hello_library").expect("it is loaded");
    assert_eq!(registry.loaded().len(), 1);
    assert_eq!(registry.loaded()[0].id, "linked_library");
    assert!(registry.provider("linked_greeter").is_some());
    assert!(registry.provider("hello_library_greeter").is_none());
}

/// An unknown key is `NotFound`, and nothing moves.
#[test]
fn retiring_an_unknown_key_changes_nothing() {
    let _one = one_at_a_time();
    let mut registry = Registry::new("retire-tests", "1.0");
    registry
        .load_file(&library_path())
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");
    let before = registry.all().len();

    let why = registry.retire("nonesuch").expect_err("nothing answers");
    assert!(matches!(why, UnloadError::NotFound { key } if key == "nonesuch"));
    assert_eq!(registry.all().len(), before);
}

/// A library the host LINKS retires like any other: there is nothing to
/// unmap, and retiring never unmaps anything.
#[test]
fn a_linked_library_retires() {
    let _one = one_at_a_time();
    let mut registry = Registry::new("retire-tests", "1.0");
    registry
        .register_entry("cli", describing)
        .expect("a linked library registers")
        .loaded()
        .expect("it accepts this host");
    assert!(registry.provider("linked_greeter").is_some());

    let retired = registry.retire("linked_library").expect("it is loaded");
    assert_eq!(retired.providers, 1);
    assert!(registry.provider("linked_greeter").is_none());

    // And the name is free again, as a file's path would be.
    assert!(
        registry
            .register_entry("cli", describing)
            .expect("it registers")
            .loaded()
            .is_some()
    );
}

/// Retiring through the C surface answers the same, and an unknown key is
/// `GUATIAO_ERR_NOT_FOUND`.
#[test]
fn the_c_surface_retires() {
    use guatiao::value::types::Str;

    let _one = one_at_a_time();
    let path = library_path();
    let path = path.to_string_lossy().into_owned();

    // SAFETY: the two names are readable for the call and the allocator
    // is this crate's own.
    let reg = unsafe {
        guatiao::exports::library::guatiao_registry_new(
            Str::borrowed("c-retire"),
            Str::borrowed("1.0"),
            guatiao::Alloc::rust().as_raw(),
        )
    };
    assert!(!reg.is_null());
    let mut answer = guatiao::value::types::Value::absent();
    // SAFETY: `reg` is live, the path is readable, `answer` is writable.
    let status = unsafe {
        guatiao::exports::library::guatiao_registry_load_file(
            reg,
            Str::borrowed(&path),
            guatiao::Alloc::rust().as_raw(),
            &mut answer,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    drop(answer);

    // SAFETY: `reg` is live and the key is readable.
    assert_eq!(
        unsafe {
            guatiao::exports::library::guatiao_registry_retire(reg, Str::borrowed("hello_library"))
        },
        Status::GUATIAO_OK
    );
    // SAFETY: as above.
    assert_eq!(
        unsafe {
            guatiao::exports::library::guatiao_registry_retire(reg, Str::borrowed("hello_library"))
        },
        Status::GUATIAO_ERR_NOT_FOUND,
        "it left with the first call"
    );
    // SAFETY: the handle is live and used nowhere else.
    unsafe { guatiao::exports::library::guatiao_registry_free(reg) };
}

/// A library the host loaded twice under two keys, and retiring one.
#[test]
fn a_second_registration_of_the_same_file_is_still_skipped_until_it_retires() {
    let _one = one_at_a_time();
    let path = library_path();
    let mut registry = Registry::new("retire-tests", "1.0");
    registry
        .load_file(&path)
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");
    assert!(matches!(
        registry
            .load_file(&path)
            .expect("a repeat is an answer")
            .skipped(),
        Some(Skipped::AlreadyLoaded { .. })
    ));

    registry.retire("hello_library").expect("it is loaded");
    assert!(
        registry
            .load_file(&path)
            .expect("it loads")
            .loaded()
            .is_some()
    );
}

// --- a library this binary links, for the `Linked` cases ---------------

/// A `Str` array holds raw pointers, so it is not `Sync` without saying
/// why. These address string literals in this test binary.
struct Names<const N: usize>([guatiao::value::types::Str; N]);
// SAFETY: a constant never written, whose pointers address literals in
// this binary.
unsafe impl<const N: usize> Sync for Names<N> {}

static LINKED_KINDS: Names<1> = Names([guatiao::value::types::Str::borrowed("greeter")]);

/// Built once and leaked, which is the contract an entry point makes: the
/// descriptor lives as long as the code that answered with it.
///
/// # Safety
///
/// Called by the registry with this host's descriptor.
unsafe extern "C" fn describing(_host: *const guatiao::library::HostInfo) -> *const LibraryInfo {
    use guatiao::library::{Kinds, ProviderInfo, Providers};
    use guatiao::value::types::{MaybeNull, Str};

    struct Described {
        #[allow(dead_code)]
        providers: Vec<ProviderInfo>,
        desc: LibraryInfo,
    }
    // SAFETY: built once, never written again, and every pointer in it
    // addresses something this struct owns and keeps.
    unsafe impl Sync for Described {}
    unsafe impl Send for Described {}

    static DESCRIBED: std::sync::OnceLock<Described> = std::sync::OnceLock::new();
    let described = DESCRIBED.get_or_init(|| {
        let providers = vec![ProviderInfo {
            struct_size: size_of::<ProviderInfo>() as u32,
            vtable_size: 0,
            kinds: Kinds::new(&LINKED_KINDS.0),
            id: Str::borrowed("linked_greeter"),
            display_name: Str::borrowed("Linked"),
            config: std::ptr::null(),
            vtable: std::ptr::null(),
            ctx: std::ptr::null_mut(),
            meta: MaybeNull::null(),
            version: Str::borrowed(""),
            available: None,
            tables: guatiao::library::KindTables::empty(),
            create: None,
            destroy: None,
        }];
        let desc = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: guatiao::library::ABI_VERSION,
            id: Str::borrowed("linked_library"),
            version: Str::borrowed("1.0.0"),
            providers: Providers {
                ptr: providers.as_ptr(),
                len: providers.len(),
                stride: size_of::<ProviderInfo>(),
            },
            meta: MaybeNull::null(),
        };
        Described { providers, desc }
    });
    &described.desc
}
