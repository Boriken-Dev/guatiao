// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `export_schema!` names its symbol from the crate's name as Cargo writes
//! it for a library, with `-` as `_`: a hyphen cannot be part of a C
//! identifier. This file's own name has hyphens, so the crate it compiles
//! as does too, and the symbol below links only if the macro followed the
//! crate name and not the package's.

#![cfg(feature = "derive")]

use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::status::Status;
use guatiao::{Schema, Value};

#[derive(Schema)]
#[allow(dead_code)]
struct Connection {
    host: String,
}

guatiao::export_schema!(Connection, "conn");

unsafe extern "C" {
    fn export_schema_crate_name_conn_schema(alloc: *const Allocator, out: *mut Value) -> Status;
}

#[test]
fn the_symbol_is_the_crate_name_with_underscores() {
    assert_eq!(env!("CARGO_CRATE_NAME"), "export_schema_crate_name");

    let mut out = Value::absent();
    // SAFETY: the symbol is the one the macro emitted above; a complete
    // allocator that outlives the tree, and writable storage for one value.
    let status = unsafe { export_schema_crate_name_conn_schema(Alloc::rust().as_raw(), &mut out) };
    assert_eq!(status, Status::GUATIAO_OK);
    let schema = SchemaRef::new(&out).expect("what comes back is a schema");
    assert!(schema.find("host").is_some());
}
