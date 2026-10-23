// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A program that links the `guatiao` crate and also calls the
//! application's own library (`examples/app_library`), which exports
//! `guatiao_*` as well.
//!
//! On Windows the library is linked with `kind = "raw-dylib"`, importing
//! only the application's own symbol. Linking its import library instead
//! defines every `guatiao_*` a second time beside the crate's own, and
//! MSVC's linker stops with `LNK2005`. Elsewhere a shared library's
//! symbols never collide with the program's, and there is nothing to show.

use guatiao::value::wire;
use guatiao::{List, Map, Value};

#[cfg(windows)]
#[link(name = "myapp", kind = "raw-dylib")]
unsafe extern "C" {
    fn myapp_version() -> u32;
}

fn main() {
    let mut map = Map::new();
    map.set("made", Value::from(List::new()));
    let bytes = wire::to_bytes(&Value::from(map)).expect("a map encodes");
    let back = wire::decode(&bytes).expect("and decodes");
    assert_eq!(back.tag(), Ok(guatiao::Tag::GUATIAO_MAP));
    #[cfg(windows)]
    {
        // SAFETY: a function of no arguments the library exports.
        let version = unsafe { myapp_version() };
        println!("guatiao from the crate, myapp {version} from its library");
    }
    #[cfg(not(windows))]
    println!("guatiao from the crate");
}
