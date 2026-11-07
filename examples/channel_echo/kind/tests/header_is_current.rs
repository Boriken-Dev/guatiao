// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed header is what this build rendered, and carries both
//! tables with the hash a C implementation writes into each.

#![cfg(feature = "c-header")]

use std::path::PathBuf;

use channel_echo_kind::ChannelEchoVtable;

fn committed() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/channel_echo.h");
    std::fs::read_to_string(&path).expect("the header is committed")
}

#[test]
fn the_committed_header_is_what_this_build_rendered() {
    let rendered = std::fs::read_to_string(env!("CHANNEL_ECHO_GENERATED_HEADER"))
        .expect("build.rs rendered the header into OUT_DIR");
    let committed = committed();
    let first_difference = committed
        .lines()
        .zip(rendered.lines())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| format!("line {}: committed {a:?} vs fresh {b:?}", i + 1));
    assert_eq!(
        first_difference, None,
        "the committed header is not what this build renders. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo build -p channel_echo_kind --features c-header"
    );
    assert_eq!(committed.lines().count(), rendered.lines().count());
}

#[test]
fn the_header_carries_the_table_and_its_hash() {
    let header = committed();
    assert!(
        header.contains("typedef struct channel_echo_vtable {\n  guatiao_kind_header header;\n"),
        "the table is declared"
    );
    let hash = ChannelEchoVtable::FLOOR_HASH;
    assert!(
        header.contains(&format!("#define channel_echo_vtable_FLOOR_HASH {hash}")),
        "its hash is {hash}"
    );
}
