// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An instance built through the C surface is released by the provider
//! that built it, even once its library is retired, and holds the library
//! open until then. Its own binary: it closes `derived_greeter`.

#![cfg(feature = "load")]

use core::ffi::c_void;
use std::path::PathBuf;

use guatiao::exports::library::{
    guatiao_registry_collect, guatiao_registry_free, guatiao_registry_load_file,
    guatiao_registry_new, guatiao_registry_provider_create, guatiao_registry_provider_destroy,
    guatiao_registry_retire,
};
use guatiao::library::ProviderError;
use guatiao::value::convert::TryAsRef;
use guatiao::{Alloc, List, Map, Status, Str, Value};

fn derived_greeter() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("it is in a directory");
    let name = format!(
        "{}derived_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ]
    .into_iter()
    .map(|dir| dir.join(&name))
    .find(|p| p.is_file())
    .expect("cargo built the dev-dependency cdylib")
}

fn collected(reg: *mut guatiao::exports::library::HostRegistry) -> Vec<String> {
    let mut out = Value::absent();
    // SAFETY: a live registry, the default allocator, a writable slot.
    let status = unsafe { guatiao_registry_collect(reg, Alloc::rust().as_raw(), &mut out) };
    assert_eq!(status, Status::GUATIAO_OK);
    let list: &List = TryAsRef::<List>::try_as_ref(&out).expect("a list");
    list.iter()
        .filter_map(|k| TryAsRef::<str>::try_as_ref(k).map(str::to_string))
        .collect()
}

#[test]
fn an_instance_is_released_by_its_own_provider_after_a_retire() {
    let path = derived_greeter().to_string_lossy().into_owned();
    // SAFETY: literal text and the default allocator.
    let reg = unsafe {
        guatiao_registry_new(Str::new("c-instances"), Str::new("1.0"), core::ptr::null())
    };
    let mut answer = Value::absent();
    // SAFETY: a live registry, readable text, a writable slot.
    let status = unsafe {
        guatiao_registry_load_file(reg, Str::new(&path), Alloc::rust().as_raw(), &mut answer)
    };
    assert_eq!(status, Status::GUATIAO_OK);
    drop(answer);

    let mut config = Map::new();
    config.set("prefix", "hey");
    let config = Value::from(config);
    let mut instance: *mut c_void = core::ptr::null_mut();
    let mut err = ProviderError::none();
    // SAFETY: a live registry, a well-formed configuration, writable slots.
    let status = unsafe {
        guatiao_registry_provider_create(
            reg,
            Str::new("derived_greeter_shouter"),
            &config,
            &mut instance,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK, "{}", err.message());
    assert!(!instance.is_null());

    // SAFETY: a live registry and its library key.
    assert_eq!(
        unsafe { guatiao_registry_retire(reg, Str::new("derived_greeter")) },
        Status::GUATIAO_OK
    );
    assert!(
        collected(reg).is_empty(),
        "the instance holds its library open"
    );

    // The key no longer names a provider; the instance still finds its own.
    // SAFETY: the instance `create` built, released once.
    unsafe {
        guatiao_registry_provider_destroy(reg, Str::new("derived_greeter_shouter"), instance)
    };
    assert_eq!(
        collected(reg),
        ["derived_greeter"],
        "released, and nothing else held it"
    );

    // SAFETY: the handle made above, freed once.
    unsafe { guatiao_registry_free(reg) };
}
