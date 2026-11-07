// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed header is what this build rendered, and carries both
//! tables with the hash a C implementation writes into each.

#![cfg(feature = "c-header")]

use std::path::PathBuf;

use guatiao_channel::{ChannelSinkVtable, ChannelVtable};

fn committed() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/guatiao_channel.h");
    std::fs::read_to_string(&path).expect("the header is committed")
}

#[test]
fn the_committed_header_is_what_this_build_rendered() {
    let rendered = std::fs::read_to_string(env!("GUATIAO_CHANNEL_GENERATED_HEADER"))
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
         GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-channel --features c-header"
    );
    assert_eq!(committed.lines().count(), rendered.lines().count());
}

#[test]
fn the_header_carries_both_tables_and_their_hashes() {
    let header = committed();
    for (table, hash) in [
        ("channel_vtable", ChannelVtable::FLOOR_HASH),
        ("channel_sink_vtable", ChannelSinkVtable::FLOOR_HASH),
    ] {
        assert!(
            header.contains(&format!("typedef struct {table} {{\n  guatiao_kind_header header;\n  void (*destroy)(void *ctx);")),
            "{table} is declared, destroy first"
        );
        assert!(
            header.contains(&format!("#define {table}_FLOOR_HASH {hash}")),
            "{table}'s hash is {hash}"
        );
    }
}
