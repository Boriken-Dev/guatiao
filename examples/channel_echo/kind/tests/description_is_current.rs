// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed description, `include/channel_echo.kinds.json`, is what
//! this build describes: the echo kind with the two channel kinds it
//! names, one document the other languages' generators read.
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo test -p channel_echo_kind --test description_is_current
//! ```

use std::path::PathBuf;

use channel_echo_kind::ChannelEchoVtable;
use guatiao::Alloc;
use guatiao_channel::{ChannelSinkVtable, ChannelVtable};
use guatiao_serde::Presentation;
use guatiao_serde::text::json;

fn rendered() -> String {
    let kinds = [
        ChannelEchoVtable::description(),
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
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/channel_echo.kinds.json");
    let fresh = rendered();
    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        std::fs::write(&path, &fresh).expect("the description is written beside the header");
    }
    let committed = std::fs::read_to_string(&path).expect("the description is committed");
    assert!(
        committed == fresh,
        "the committed description is not what this build describes. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo test -p channel_echo_kind --test description_is_current"
    );
}

#[test]
fn start_takes_a_sink_and_answers_a_channel() {
    let echo = ChannelEchoVtable::description();
    assert!(
        echo.contains(
            "{\"name\":\"sink\",\"type\":\"object\",\"kind\":\"channel_sink\",\"pass\":\"taken\"}"
        ),
        "{echo}"
    );
    assert!(
        echo.contains("\"answer\":{\"type\":\"object\",\"kind\":\"channel\"}"),
        "{echo}"
    );
}
