// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The claim the schema rewrite exists for, measured rather than asserted.
//!
//! Everything in `guatiao` supports "a schema IS a JSON Schema" by
//! inspection: the vocabulary names the specification's keys and a test
//! there asserts the document key for key. Nothing had watched a
//! *third-party validator* accept one, which is a different question — it
//! is the one a consumer actually asks.
//!
//! So this builds a schema through the ordinary builders, writes it out
//! through this crate the way a consumer would, and hands the text to
//! `boon`. What it proves is that the bytes are a JSON Schema, not that
//! we think they are.
//!
//! # Why it lives here and not in `guatiao`
//!
//! The core crate has no serialisation and no dependencies -- `cargo tree
//! -p guatiao -e normal` prints one line, and a validator in its dev
//! dependencies would still be a dependency of its test build. This crate
//! is where a schema becomes text, so it is where the question can be
//! asked at all.

use boon::{CompileError, Compiler, SchemaIndex, Schemas};
use guatiao::Value;
use guatiao::schema::build::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao_serde::{Presentation, text::json};

/// A schema as a consumer would receive it: through this crate, as text,
/// parsed back by somebody else's parser.
///
/// The round trip through text is the point. Handing `boon` a value built
/// in memory would test our reader against our writer; handing it the
/// bytes tests what actually crosses.
fn as_document(schema: &Value) -> serde_json::Value {
    let text = json::to_string(schema, Presentation::default()).expect("a schema serialises");
    serde_json::from_str(&text).expect("and what comes out is JSON")
}

/// Compiles the document as a schema, which is where a validator checks
/// it against the meta-schema.
///
/// Boxed because `CompileError` is large, and a test helper is no reason
/// to move 160 bytes around on every `Ok`.
fn compile(doc: serde_json::Value) -> Result<(Schemas, SchemaIndex), Box<CompileError>> {
    let mut compiler = Compiler::new();
    compiler.add_resource("guatiao://schema", doc)?;
    let mut schemas = Schemas::new();
    let index = compiler.compile("guatiao://schema", &mut schemas)?;
    Ok((schemas, index))
}

/// The schema under test: one of every kind that has a JSON Schema
/// spelling.
fn connection() -> Value {
    use guatiao::schema::{FormBuilder, FormFieldBuilder};

    SchemaBuilder::new()
        .label("Connection")
        .help("Where to connect, and how.")
        .field(
            FieldBuilder::new("host", KindBuilder::string())
                .label("Host")
                .required(),
        )
        .field(
            FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
                .label("Port")
                .default(Value::int(5900))
                .order(2),
        )
        .field(FieldBuilder::new(
            "level",
            KindBuilder::enumeration(&[("off", "Off"), ("on", "On")]),
        ))
        .field(FieldBuilder::new(
            "tags",
            KindBuilder::list(KindBuilder::string()),
        ))
        .field(
            FieldBuilder::new(
                "auth",
                KindBuilder::variant(
                    "auth",
                    vec![
                        ArmBuilder::new("ambient", "Ambient"),
                        ArmBuilder::new("userpass", "Username and password")
                            .field(FieldBuilder::new("username", KindBuilder::string()))
                            .field(
                                FieldBuilder::new("password", KindBuilder::string()).sensitive(),
                            ),
                    ],
                ),
            )
            .label("Authentication"),
        )
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// A real 2020-12 validator compiles what we write.
///
/// Compiling is the meta-schema check: `boon` reads `$schema`, fetches
/// the dialect it names, and refuses a document that is not a schema.
#[test]
fn a_validator_compiles_the_document_we_write() {
    let schema = connection();
    let doc = as_document(&schema);
    assert_eq!(
        doc["$schema"],
        serde_json::json!("https://json-schema.org/draft/2020-12/schema"),
        "the dialect declaration is what tells a validator which rules to apply"
    );
    compile(doc).expect("a validator compiles it as a 2020-12 schema");
}

/// And it agrees with us about which documents the schema accepts.
///
/// Each rejection names the keyword doing the work, because that is the
/// claim: `required`, `minimum`/`maximum`, `enum` and `oneOf` are being
/// read by somebody else's implementation of the specification.
#[test]
fn a_validator_accepts_and_rejects_the_same_documents_we_do() {
    let schema = connection();
    let (schemas, index) = compile(as_document(&schema)).expect("it compiles");

    let ok = serde_json::json!({
        "host": "example.test",
        "port": 5900,
        "level": "on",
        "tags": ["a", "b"],
        "auth": { "auth": "userpass", "username": "ana", "password": "hunter2" }
    });
    assert!(
        schemas.validate(&ok, index).is_ok(),
        "a conforming document is accepted"
    );

    let cases: [(&str, serde_json::Value); 5] = [
        // `required`, on the object rather than the field.
        (
            "a missing required field",
            serde_json::json!({ "port": 5900 }),
        ),
        // `minimum` / `maximum`.
        (
            "a port outside the declared range",
            serde_json::json!({ "host": "h", "port": 70000 }),
        ),
        // `enum`.
        (
            "a choice nobody declared",
            serde_json::json!({ "host": "h", "level": "maybe" }),
        ),
        // `type`.
        (
            "text where an integer was declared",
            serde_json::json!({ "host": "h", "port": "5900" }),
        ),
        // `oneOf` plus the arm's `const` discriminant.
        (
            "a variant naming no declared arm",
            serde_json::json!({ "host": "h", "auth": { "auth": "kerberos" } }),
        ),
    ];
    for (why, instance) in cases {
        assert!(
            schemas.validate(&instance, index).is_err(),
            "a validator should reject {why}: {instance}"
        );
    }
}

/// An arm's payload belongs to that arm, and a validator enforces it from
/// the `const` discriminant alone.
///
/// The whole argument for spelling a variant as a discriminated `oneOf`
/// rather than as an annotation nobody reads: a consumer with no guatiao
/// at all still gets the rule.
#[test]
fn a_validator_tells_the_arms_apart_by_their_discriminant() {
    let schema = connection();
    let (schemas, index) = compile(as_document(&schema)).expect("it compiles");

    for (why, auth) in [
        (
            "the empty arm is complete",
            serde_json::json!({ "auth": "ambient" }),
        ),
        (
            "and so is the one with a payload",
            serde_json::json!({ "auth": "userpass", "username": "ana" }),
        ),
    ] {
        let instance = serde_json::json!({ "host": "h", "auth": auth });
        assert!(
            schemas.validate(&instance, index).is_ok(),
            "{why}: {instance}"
        );
    }
}

/// **`type: "bytes"` is ours, and a strict validator says so.**
///
/// Pinned rather than avoided. JSON Schema's meta-schema fixes `type` to
/// a closed set of seven, so a document declaring an eighth is not a
/// valid schema — readable by anything, refused by a schema-of-the-schema
/// check. That is the cost the user accepted when choosing to describe
/// bytes as bytes rather than as text that happens to survive.
///
/// This test is what makes it stay a decision. If it ever starts passing,
/// somebody changed the type name, and that is a thing to notice.
#[test]
fn the_one_type_we_invented_is_the_one_a_strict_validator_refuses() {
    let plain = SchemaBuilder::new()
        .field(FieldBuilder::new("cert", KindBuilder::string()))
        .finish()
        .unwrap();
    compile(as_document(&plain)).expect("the same schema without bytes compiles");

    let with_bytes = SchemaBuilder::new()
        .field(FieldBuilder::new("cert", KindBuilder::bytes()))
        .finish()
        .unwrap();
    let doc = as_document(&with_bytes);
    assert_eq!(
        doc["properties"]["cert"]["type"],
        serde_json::json!("bytes"),
        "the document says what it means"
    );

    let Err(refused) = compile(doc) else {
        panic!(
            "a 2020-12 validator must refuse `type: \"bytes\"` — the meta-schema's \
             type list is closed. See guatiao::schema::vocab::TYPE_BYTES for why we keep it"
        );
    };
    // The alternate form carries the meta-schema's own reasoning. Pinned to
    // the LOCATION, because "the document failed somewhere" would also
    // pass for a regression that broke something else entirely.
    let complaint = format!("{refused:#}");
    assert!(
        complaint.contains("'/properties/cert/type'"),
        "refused at the bytes field's `type`, not somewhere else: {complaint}"
    );
    assert!(
        complaint.contains("'array', 'boolean', 'integer', 'null', 'number', 'object', 'string'"),
        "and for the reason on record: the meta-schema's type list is closed: {complaint}"
    );
}
