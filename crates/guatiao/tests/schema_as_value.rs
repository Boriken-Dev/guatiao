// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A schema is a value, and the builder and the reader agree about it.
//!
//! These go through the public API a provider and a consumer each use: one
//! declares a schema, the other reads it back without having seen the
//! declaration. That is the whole contract, so it is what gets tested.

use std::cell::Cell;
use std::ffi::c_void;

use guatiao::Value;
use guatiao::schema::build::{ArmBuilder, KindBuilder, OptionBuilder, SchemaBuilder};
use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::vocab;
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::entries;

// A counting allocator, so every test also proves the schema frees.
#[derive(Default)]
struct Counter {
    outstanding: Cell<isize>,
}

unsafe extern "C" fn c_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Counter` installed below, outliving the call.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn c_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

fn with_alloc(body: impl FnOnce(Alloc)) {
    let counter = Counter::default();
    let vt = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&counter as *const Counter).cast_mut().cast(),
        alloc: Some(c_alloc),
        free: Some(c_free),
        release: None,
    };
    // SAFETY: `vt` is fully initialised and outlives the closure.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");
    body(alloc);
    assert_eq!(
        counter.outstanding.get(),
        0,
        "a schema is a value, so it frees like one"
    );
}

#[test]
fn a_declared_schema_reads_back() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .section("net", "Network", "How to reach it")
            .option(
                OptionBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc))
                    .label("Host")
                    .section("net")
                    .required()
                    .order(1),
            )
            .option(
                OptionBuilder::new_in(alloc, "port", KindBuilder::int_range_in(alloc, 1, 65535))
                    .label("Port")
                    .section("net")
                    .default(Value::int_in(alloc, 5900))
                    .order(2),
            )
            .option(
                OptionBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                    .label("Password")
                    .sensitive()
                    .advanced(),
            )
            .finish()
            .expect("a schema this small does not exhaust an allocator");

        let s = SchemaRef::new(&schema).expect("a schema is a map");

        let keys: Vec<_> = s.options().map(|o| o.key().to_string()).collect();
        assert_eq!(keys, ["host", "port", "password"], "declaration order");

        let host = s.find("host").expect("host is declared");
        assert_eq!(host.label(), "Host");
        assert_eq!(host.section(), "net");
        assert!(host.is_required());
        assert!(!host.is_sensitive());
        assert!(matches!(host.kind(), Kind::Str));
        assert!(host.default().is_none(), "no default is not a null default");

        let port = s.find("port").expect("port is declared");
        assert!(matches!(
            port.kind(),
            Kind::Int {
                min: Some(1),
                max: Some(65535)
            }
        ));
        assert_eq!(
            guatiao::value::read::int_or(port.default(), -1),
            5900,
            "the default reads back as the number it was declared as"
        );

        let password = s.find("password").expect("password is declared");
        assert!(password.is_sensitive());
        assert!(password.is_advanced());

        let section = s.sections().next().expect("one section");
        assert_eq!(section.id(), "net");
        assert_eq!(section.label(), "Network");

        assert!(s.find("nothing").is_none());
    });
}

#[test]
fn an_enum_carries_rows_not_two_parallel_lists() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .option(OptionBuilder::new_in(
                alloc,
                "level",
                KindBuilder::enumeration_in(alloc, &[("off", "Off"), ("on", "On")]),
            ))
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        let kind = s.find("level").unwrap().kind();
        let choices: Vec<_> = kind
            .choices()
            .map(|c| (c.value().to_string(), c.label().to_string()))
            .collect();
        assert_eq!(
            choices,
            [
                ("off".to_string(), "Off".to_string()),
                ("on".to_string(), "On".to_string())
            ],
            "each alternative is one row, so its value and its label cannot drift apart"
        );
    });
}

/// A union is untagged and a variant is tagged, and a reader must not
/// treat them alike: a selector built from a union's arms would store a
/// bare discriminant with no payload.
#[test]
fn a_union_and_a_variant_are_different_features() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .option(OptionBuilder::new_in(
                alloc,
                "port",
                KindBuilder::union_in(
                    alloc,
                    vec![KindBuilder::int_in(alloc), KindBuilder::string_in(alloc)],
                ),
            ))
            .option(OptionBuilder::new_in(
                alloc,
                "auth",
                KindBuilder::variant_in(
                    alloc,
                    "auth",
                    vec![
                        // An arm with no fields is ordinary and complete:
                        // "use the ambient credential" is the common case.
                        ArmBuilder::new_in(alloc, "ambient", "Ambient"),
                        ArmBuilder::new_in(alloc, "userpass", "Username and password")
                            .field(OptionBuilder::new_in(
                                alloc,
                                "username",
                                KindBuilder::string_in(alloc),
                            ))
                            .field(OptionBuilder::new_in(
                                alloc,
                                "password",
                                KindBuilder::string_in(alloc),
                            )),
                    ],
                ),
            ))
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();

        let union = s.find("port").unwrap().kind();
        assert_eq!(union.alternatives().count(), 2, "a union has arms as kinds");
        assert_eq!(union.arms().count(), 0, "and no tagged arms");

        let variant = s.find("auth").unwrap().kind();
        assert!(matches!(variant, Kind::Variant { tag: "auth", .. }));
        assert_eq!(
            variant.alternatives().count(),
            0,
            "a variant's arms are NOT a union's; reporting them here would make a reader \
             draw a selector that stores a bare discriminant with no payload"
        );

        let arms: Vec<_> = variant.arms().collect();
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].value(), "ambient");
        assert_eq!(arms[0].fields().count(), 0, "an empty arm is complete");
        assert_eq!(arms[1].value(), "userpass");
        let fields: Vec<_> = arms[1].fields().map(|f| f.key().to_string()).collect();
        assert_eq!(fields, ["username", "password"]);
    });
}

/// The whole forward-compatibility story: a kind from a newer producer
/// leaves its option readable, and every other option untouched.
#[test]
fn an_unknown_kind_leaves_the_option_readable_and_the_rest_intact() {
    with_alloc(|alloc| {
        let mut schema = SchemaBuilder::new_in(alloc)
            .option(
                OptionBuilder::new_in(alloc, "known", KindBuilder::string_in(alloc)).label("Known"),
            )
            .finish()
            .unwrap();

        // What a newer producer writes: a kind this build has never heard
        // of, on an option that is otherwise ordinary.
        let mut kind = Value::map_in(alloc);
        kind.set(vocab::TYPE, Value::string_in(alloc, "duration").unwrap())
            .unwrap();
        let mut option = Value::map_in(alloc);
        option
            .set(vocab::KEY, Value::string_in(alloc, "timeout").unwrap())
            .unwrap();
        option
            .set(vocab::LABEL, Value::string_in(alloc, "Timeout").unwrap())
            .unwrap();
        option.set(vocab::KIND, kind).unwrap();
        schema.push_into(vocab::OPTIONS, option).unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        assert_eq!(s.options().count(), 2, "both options are still listed");

        let future = s.find("timeout").expect("the option is readable");
        assert!(matches!(future.kind(), Kind::Unknown("duration")));
        assert_eq!(
            future.label(),
            "Timeout",
            "and everything that does not depend on the kind still reads"
        );

        assert_eq!(s.find("known").unwrap().label(), "Known");
    });
}

/// An entry in the options list that is not an option is skipped, rather
/// than taking the schema down with it.
#[test]
fn a_malformed_option_is_skipped_not_fatal() {
    with_alloc(|alloc| {
        let mut schema = SchemaBuilder::new_in(alloc)
            .option(OptionBuilder::new_in(
                alloc,
                "good",
                KindBuilder::bool_in(alloc),
            ))
            .finish()
            .unwrap();

        // No key, so it is not an option.
        let mut keyless = Value::map_in(alloc);
        keyless
            .set(vocab::LABEL, Value::string_in(alloc, "orphan").unwrap())
            .unwrap();
        schema.push_into(vocab::OPTIONS, keyless).unwrap();
        // Not even a map.
        schema.push_into(vocab::OPTIONS, Value::bool(true)).unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        let keys: Vec<_> = s.options().map(|o| o.key().to_string()).collect();
        assert_eq!(keys, ["good"], "the readable option still reads");
    });
}

/// Anything outside the vocabulary is an annotation: carried, and never
/// interpreted.
#[test]
fn an_annotation_is_carried_but_not_interpreted() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .option(
                OptionBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc))
                    .extra("x-widget", Value::string_in(alloc, "combo")),
            )
            .extra("x-origin", Value::string_in(alloc, "test"))
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        let host = s.find("host").unwrap();
        assert_eq!(
            guatiao::value::read::str_or(host.extra("x-widget"), ""),
            "combo"
        );
        assert_eq!(
            guatiao::value::read::str_or(s.extra("x-origin"), ""),
            "test"
        );

        assert!(
            host.extra(vocab::KEY).is_none(),
            "a keyword is not an annotation, even though it is a key"
        );

        // And a schema really is just a map, walkable by anything that
        // walks a value.
        assert!(entries(&schema).count() >= 2);
    });
}

/// **Building a schema names no allocator**, the same rule the value API
/// has -- and the two forms build the same thing.
///
/// The plain form is what a schema written by hand should read like; the
/// `_in` form is for a schema built into a host's arena, which is what
/// the example library does.
#[test]
fn the_plain_builders_and_the_in_builders_agree() {
    let by_hand = SchemaBuilder::new()
        .option(
            OptionBuilder::new("port", KindBuilder::int_range(1, 65535))
                .label("Port")
                .required(),
        )
        .option(OptionBuilder::new("name", KindBuilder::string()))
        .finish()
        .expect("a schema this small does not exhaust an allocator");

    let alloc = Alloc::rust();
    let named = SchemaBuilder::new_in(alloc)
        .option(
            OptionBuilder::new_in(alloc, "port", KindBuilder::int_range_in(alloc, 1, 65535))
                .label("Port")
                .required(),
        )
        .option(OptionBuilder::new_in(
            alloc,
            "name",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .expect("as above");

    assert!(
        guatiao::value::read::equal(&by_hand, &named),
        "the plain form is the `_in` form with the crate's own allocator"
    );
}
