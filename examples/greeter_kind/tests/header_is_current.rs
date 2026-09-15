// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed kind header is what this build rendered, and it carries
//! what a C implementation needs: every table, and the hash it writes
//! into a table's header.

#![cfg(feature = "c-header")]

use std::path::PathBuf;

use greeter_kind::{ConversationVtable, CounterVtable, GreeterVtable, ListenerVtable};

fn committed() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/greeter_kind.h");
    std::fs::read_to_string(&path).expect("the kind header is committed")
}

#[test]
fn the_committed_header_is_what_this_build_rendered() {
    let rendered = std::fs::read_to_string(env!("GREETER_KIND_GENERATED_HEADER"))
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
        "the committed kind header is not what this build renders. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo build -p greeter_kind --features c-header"
    );
    assert_eq!(committed.lines().count(), rendered.lines().count());
}

#[test]
fn the_header_carries_every_table_and_its_hash() {
    let header = committed();
    for (table, hash) in [
        ("greeter_vtable", GreeterVtable::FLOOR_HASH),
        ("counter_vtable", CounterVtable::FLOOR_HASH),
        ("conversation_vtable", ConversationVtable::FLOOR_HASH),
        ("listener_vtable", ListenerVtable::FLOOR_HASH),
    ] {
        assert!(
            header.contains(&format!("typedef struct {table} {{")),
            "{table} is declared"
        );
        assert!(
            header.contains(&format!("#define {table}_FLOOR_HASH {hash}")),
            "{table}'s floor hash is the literal the attribute wrote: {hash}"
        );
    }
    // An object kind's table starts with `destroy`; a provider kind's
    // does not.
    assert!(header.contains("typedef struct conversation_vtable {\n  guatiao_kind_header header;\n  void (*destroy)(void *ctx);"));
    assert!(header.contains(
        "typedef struct greeter_vtable {\n  guatiao_kind_header header;\n  guatiao_status (*greet)("
    ));
}
