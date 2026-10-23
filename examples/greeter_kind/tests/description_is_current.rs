// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed kind description, `include/greeter_kind.kinds.json`, is
//! what this build describes: every kind of the crate as data, which a
//! binding loads, or a generator reads, instead of declaring the kinds
//! again. Written beside the header, with the same switch:
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo test -p greeter_kind --test description_is_current
//! ```

use std::path::PathBuf;

use greeter_kind::{ConversationVtable, CounterVtable, GreeterVtable, ListenerVtable, WideVtable};
use guatiao::Alloc;
use guatiao_serde::Presentation;
use guatiao_serde::text::json;

/// Every kind the crate declares, in declaration order, as one document.
fn rendered() -> String {
    let kinds = [
        GreeterVtable::description(),
        ListenerVtable::description(),
        ConversationVtable::description(),
        CounterVtable::description(),
        WideVtable::description(),
    ];
    let document = format!("{{\"format\":1,\"kinds\":[{}]}}", kinds.join(","));
    let value = json::from_str(&document, Alloc::rust(), Presentation::new())
        .expect("a kind's description is JSON");
    let mut text = json::to_string_pretty(&value, Presentation::new()).expect("written");
    text.push('\n');
    text
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/greeter_kind.kinds.json");
    let fresh = rendered();
    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        std::fs::write(&path, &fresh).expect("the description is written beside the header");
    }
    let committed = std::fs::read_to_string(&path).expect("the kind description is committed");
    assert!(
        committed == fresh,
        "the committed kind description is not what this build describes. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo test -p greeter_kind --test description_is_current"
    );
}

#[test]
fn a_description_carries_the_hash_the_header_defines() {
    for (description, hash) in [
        (GreeterVtable::description(), GreeterVtable::FLOOR_HASH),
        (ListenerVtable::description(), ListenerVtable::FLOOR_HASH),
        (CounterVtable::description(), CounterVtable::FLOOR_HASH),
    ] {
        assert!(
            description.contains(&format!("\"floor_hash\":{hash},")),
            "{description}"
        );
    }
    // An object kind named by another is named as its own kind says.
    let greeter = GreeterVtable::description();
    assert!(
        greeter.contains(
            "{\"name\":\"listener\",\"type\":\"object\",\"kind\":\"listener\",\"pass\":\"taken\"}"
        ),
        "{greeter}"
    );
    assert!(greeter.contains("\"answer\":{\"type\":\"object\",\"kind\":\"conversation\"}"));
    assert!(greeter.contains("{\"name\":\"config\",\"type\":\"map\",\"pass\":\"lent\"}"));
}
