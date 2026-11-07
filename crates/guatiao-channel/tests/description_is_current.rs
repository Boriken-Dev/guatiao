// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed description, `include/guatiao_channel.kinds.json`, is
//! what this build describes, and each kind's floor hash is pinned: a
//! change to either is a change every provider and host of a channel sees.
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo test -p guatiao-channel --test description_is_current
//! ```

use std::path::PathBuf;

use guatiao::Alloc;
use guatiao_channel::{ChannelSinkVtable, ChannelVtable};
use guatiao_serde::Presentation;
use guatiao_serde::text::json;

fn rendered() -> String {
    let kinds = [
        ChannelVtable::description(),
        ChannelSinkVtable::description(),
    ];
    let document = format!("{{\"kinds\":[{}]}}", kinds.join(","));
    let value = json::from_str(&document, Alloc::rust(), Presentation::new())
        .expect("a kind's description is JSON");
    let mut text = json::to_string_pretty(&value, Presentation::new()).expect("written");
    text.push('\n');
    text
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/guatiao_channel.kinds.json");
    let fresh = rendered();
    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        std::fs::write(&path, &fresh).expect("the description is written beside the header");
    }
    let committed = std::fs::read_to_string(&path).expect("the description is committed");
    assert!(
        committed == fresh,
        "the committed description is not what this build describes. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo test -p guatiao-channel --test description_is_current"
    );
}

#[test]
fn each_kind_keeps_its_name_and_floor_hash() {
    for (description, name, hash) in [
        (
            ChannelVtable::description(),
            "channel",
            ChannelVtable::FLOOR_HASH,
        ),
        (
            ChannelSinkVtable::description(),
            "channel_sink",
            ChannelSinkVtable::FLOOR_HASH,
        ),
    ] {
        assert!(
            description.contains(&format!("\"name\":\"{name}\"")),
            "{description}"
        );
        assert!(
            description.contains(&format!("\"floor_hash\":{hash},")),
            "{description}"
        );
        assert!(description.contains("\"object\":true"), "{description}");
    }
    assert_eq!(
        (ChannelVtable::FLOOR_HASH, ChannelSinkVtable::FLOOR_HASH),
        (PINNED_CHANNEL, PINNED_SINK),
        "a floor hash moved: every built provider and host of a channel is refused until rebuilt"
    );
}

const PINNED_CHANNEL: u32 = 2843269355;
const PINNED_SINK: u32 = 3528315804;
