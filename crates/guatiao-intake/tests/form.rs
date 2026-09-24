// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A form beside its schema: built, read, checked, laid out and evaluated.
//!
//! Every rule the crate states has a test here that fails without it.

use std::cell::Cell;
use std::ffi::c_void;

use guatiao::schema::read::SchemaRef;
use guatiao::schema::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::str_or;
use guatiao::{Map, Text, Value};
use guatiao_intake::{
    Form, FormBuilder, FormError, FormFieldBuilder, FormRef, Hints, Section, check, for_field,
    for_schema, form_for, is_visible, layout, vocab,
};

/// A schema with a bit of everything: sections, an order, a boolean that
/// guards another field, a default, and a variant with an arm field.
fn schema() -> Value {
    SchemaBuilder::new()
        .field(
            FieldBuilder::new("host", KindBuilder::string())
                .section("net")
                .required(),
        )
        .field(
            FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
                .section("net")
                .order(1),
        )
        .field(FieldBuilder::new("verify", KindBuilder::bool()).default(true))
        .field(FieldBuilder::new("ca", KindBuilder::string()))
        .field(
            FieldBuilder::new(
                "auth",
                KindBuilder::variant(
                    "auth",
                    vec![
                        ArmBuilder::new("ambient", "Ambient"),
                        ArmBuilder::new("userpass", "Username and password")
                            .field(FieldBuilder::new("username", KindBuilder::string())),
                    ],
                ),
            )
            .section("login"),
        )
        .finish()
        .expect("a schema this small builds")
}

fn views<'a>(schema: &'a Value, form: &'a Value) -> (SchemaRef<'a>, FormRef<'a>) {
    (
        SchemaRef::new(schema).expect("a schema is a map"),
        FormRef::new(form).expect("a form is a map"),
    )
}

fn keys(group: &guatiao_intake::Group<'_>) -> Vec<String> {
    group
        .fields
        .iter()
        .map(|p| p.field.key().to_string())
        .collect()
}

// --- building and reading -------------------------------------------------

#[test]
fn a_built_form_reads_back() {
    let form = Form::new()
        .section(
            Section::new("net")
                .label("Network")
                .help("Where to connect."),
        )
        .section(Section::new("login").label("Login"))
        .field(
            "port",
            Hints::new()
                .widget(vocab::widget::NUMBER)
                .option("x-step", 10),
        )
        .field(
            "ca",
            Hints::new()
                .placeholder("/etc/ssl/ca.pem")
                .visible_when("verify", true),
        )
        .option("x-origin", "test")
        .finish()
        .unwrap();
    let f = FormRef::new(&form).unwrap();

    let sections: Vec<(&str, &str, &str)> = f
        .sections()
        .map(|s| (s.id(), s.label(), s.help()))
        .collect();
    assert_eq!(
        sections,
        [
            ("net", "Network", "Where to connect."),
            ("login", "Login", "")
        ],
        "declaration order is display order"
    );

    assert_eq!(f.hints("port").widget(), "number");
    assert_eq!(
        guatiao::value::read::int_or(f.hints("port").extra("x-step"), 0),
        10,
        "an annotation is carried"
    );
    assert!(
        f.hints("port").extra(vocab::WIDGET).is_none(),
        "a keyword is not an annotation"
    );

    let ca = f.hints("ca");
    assert_eq!(ca.placeholder(), "/etc/ssl/ca.pem");
    let condition = ca.visible_when().expect("ca has a condition");
    assert_eq!(condition.field(), "verify");
    assert_eq!(condition.equals().try_into().ok(), Some(true));

    assert!(
        f.hints("host").is_empty(),
        "no hints is ordinary, not missing"
    );
    let paths: Vec<&str> = f.fields().map(|(path, _)| path).collect();
    assert_eq!(paths, ["port", "ca"]);
    assert_eq!(str_or(f.extra("x-origin"), ""), "test");
}

#[test]
fn an_empty_map_is_a_complete_form() {
    let schema = schema();
    let form = Form::new().finish().unwrap();
    let (s, f) = views(&schema, &form);

    check(s, f).expect("an empty form fits any schema");
    let groups = layout(s, f);
    assert_eq!(
        groups.len(),
        1,
        "the schema names `net` and `login`, but a form that declares no section \
         puts every field in the default group"
    );
    assert!(groups[0].section.is_none());
    assert_eq!(
        keys(&groups[0]),
        ["port", "host", "verify", "ca", "auth"],
        "`port` has the only explicit order; the rest keep the schema's"
    );
}

// --- the default section --------------------------------------------------

/// A field naming a section the form does not declare joins the default
/// group, and the default group comes first unless the form places it.
#[test]
fn undeclared_sections_join_the_default_group_which_comes_first() {
    let schema = schema();
    let form = Form::new()
        .section(Section::new("net").label("Network"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    check(s, f).expect("naming an undeclared section is not an error");
    let groups = layout(s, f);
    assert_eq!(groups.len(), 2, "{groups:#?}");

    assert!(groups[0].section.is_none(), "the default group is first");
    assert_eq!(
        keys(&groups[0]),
        ["verify", "ca", "auth"],
        "`auth` names `login`, which this form does not declare"
    );
    assert_eq!(groups[1].section.unwrap().label(), "Network");
}

/// Declaring the default section gives the ungrouped fields a title and a
/// position.
#[test]
fn declaring_the_default_section_places_and_titles_it() {
    let schema = schema();
    let form = Form::new()
        .section(Section::new("net").label("Network"))
        .section(Section::new(vocab::DEFAULT_SECTION).label("General"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let groups = layout(s, f);
    let titles: Vec<&str> = groups
        .iter()
        .map(|g| g.section.map_or("<none>", |s| s.label()))
        .collect();
    assert_eq!(titles, ["Network", "General"], "where the form put it");
}

// --- order within a group --------------------------------------------------

/// Explicit `x-order` first and ascending; the rest in declaration order.
#[test]
fn an_explicit_order_comes_first_then_declaration_order() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("a", KindBuilder::string()))
        .field(FieldBuilder::new("b", KindBuilder::string()).order(5))
        .field(FieldBuilder::new("c", KindBuilder::string()))
        .field(FieldBuilder::new("d", KindBuilder::string()).order(-2))
        .finish()
        .unwrap();
    let form = Form::new().finish().unwrap();
    let (s, f) = views(&schema, &form);

    let groups = layout(s, f);
    assert_eq!(keys(&groups[0]), ["d", "b", "a", "c"]);
}

/// A declared section nothing is in is not drawn.
#[test]
fn an_empty_section_is_left_out() {
    let schema = schema();
    let form = Form::new()
        .section(Section::new("nobody-uses-this").label("Empty"))
        .section(Section::new("net"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let ids: Vec<&str> = layout(s, f)
        .iter()
        .map(|g| g.section.map_or("<default>", |s| s.id()))
        .collect();
    assert_eq!(ids, ["<default>", "net"]);
}

/// Hints ride along with the field they belong to.
#[test]
fn a_placed_field_carries_its_hints() {
    let schema = schema();
    let form = Form::new()
        .section(Section::new("net"))
        .field("port", Hints::new().widget(vocab::widget::SLIDER))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let groups = layout(s, f);
    let net = groups
        .iter()
        .find(|g| g.section.is_some_and(|s| s.id() == "net"))
        .unwrap();
    assert_eq!(net.fields[0].field.key(), "port");
    assert_eq!(net.fields[0].hints.widget(), "slider");
    assert!(net.fields[1].hints.is_empty());
}

// --- check ------------------------------------------------------------------

#[test]
fn a_hint_for_a_field_the_schema_does_not_declare_is_refused() {
    let schema = schema();
    let form = Form::new().field("hots", Hints::new()).finish().unwrap();
    let (s, f) = views(&schema, &form);
    assert_eq!(
        check(s, f),
        Err(FormError::UnknownField {
            at: "fields".into(),
            path: "hots".into()
        })
    );
}

#[test]
fn an_arm_field_is_addressed_by_its_dotted_path() {
    let schema = schema();
    let form = Form::new()
        .field("auth.username", Hints::new().placeholder("root"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    check(s, f).expect("`auth.username` resolves through the variant's arm");
    assert_eq!(f.hints("auth.username").placeholder(), "root");
}

#[test]
fn a_section_declared_twice_is_refused() {
    let schema = schema();
    let form = Form::new()
        .section(Section::new("net"))
        .section(Section::new("net"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    assert_eq!(
        check(s, f),
        Err(FormError::DuplicateSection { id: "net".into() })
    );
}

#[test]
fn every_shape_mistake_names_where_it_is() {
    let schema = schema();
    let cases: Vec<(Value, &str, &str)> = vec![
        (
            Form::new().option(vocab::SECTIONS, 3).finish().unwrap(),
            "sections",
            "a list",
        ),
        (
            {
                let mut section = Map::new();
                section.set(vocab::TITLE, "no id").unwrap();
                Form::new()
                    .option(vocab::SECTIONS, {
                        let mut l = guatiao::List::new();
                        l.push(section).unwrap();
                        l
                    })
                    .finish()
                    .unwrap()
            },
            "sections[0].id",
            "text",
        ),
        (
            Form::new()
                .field("port", Hints::new().option(vocab::WIDGET, 7))
                .finish()
                .unwrap(),
            "fields[\"port\"].widget",
            "text",
        ),
        (
            Form::new().option(vocab::FIELDS, 3).finish().unwrap(),
            "fields",
            "a map",
        ),
        (
            {
                let mut fields = Map::new();
                fields.set("port", true).unwrap();
                Form::new().option(vocab::FIELDS, fields).finish().unwrap()
            },
            "fields[\"port\"]",
            "a map",
        ),
        (
            {
                let mut condition = Map::new();
                condition.set(vocab::FIELD, "verify").unwrap();
                Form::new()
                    .field("ca", Hints::new().option(vocab::VISIBLE_WHEN, condition))
                    .finish()
                    .unwrap()
            },
            "fields[\"ca\"].visibleWhen.equals",
            "a value to compare with",
        ),
    ];
    for (form, at, expected) in cases {
        let (s, f) = views(&schema, &form);
        assert_eq!(
            check(s, f),
            Err(FormError::Malformed {
                at: at.into(),
                expected
            }),
            "for {at}"
        );
    }
}

#[test]
fn a_condition_on_a_field_the_schema_does_not_declare_is_refused() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("verfy", true))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    assert_eq!(
        check(s, f),
        Err(FormError::UnknownField {
            at: "fields[\"ca\"].visibleWhen".into(),
            path: "verfy".into()
        })
    );
}

/// A condition that could never be met is a form bug — the field it guards
/// would never appear — and the message never quotes the value.
#[test]
fn a_condition_the_field_could_never_meet_is_refused_without_quoting_it() {
    let schema = schema();
    let cases = [
        ("port", Value::from(Text::new("hunter2"))),
        ("port", Value::from(70000i64)),
        ("auth", Value::from(Text::new("kerberos"))),
        ("auth", Value::from(true)),
    ];
    for (field, equals) in cases {
        let form = Form::new()
            .field("ca", Hints::new().visible_when(field, equals))
            .finish()
            .unwrap();
        let (s, f) = views(&schema, &form);
        let error = check(s, f).expect_err("refused");
        assert!(
            matches!(&error, FormError::ConditionRefused { field: got, .. } if got == field),
            "{error:?}"
        );
        let message = error.to_string();
        for secret in ["hunter2", "70000", "kerberos"] {
            assert!(
                !message.contains(secret),
                "an error never quotes the value: {message}"
            );
        }
    }

    // And the arm name that exists is accepted.
    let form = Form::new()
        .field("ca", Hints::new().visible_when("auth", "userpass"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    check(s, f).expect("`userpass` is an arm of `auth`");
}

#[test]
fn a_field_waiting_on_itself_is_refused_naming_the_cycle() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("host", "a"))
        .field("host", Hints::new().visible_when("port", 1))
        .field("port", Hints::new().visible_when("host", "b"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    assert_eq!(
        check(s, f),
        Err(FormError::CyclicCondition {
            path: "host".into()
        }),
        "`ca` only leads into the cycle; `host` is on it"
    );
}

// --- visibility ----------------------------------------------------------------

/// `is_visible` for a path the schema declares, which every case below
/// is. A path it does not declare is an error, and has its own test.
fn shown(s: SchemaRef<'_>, f: FormRef<'_>, path: &str, values: &Value) -> bool {
    is_visible(s, f, path, values).expect("the path names a field the schema declares")
}

fn values(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.set(key, value).unwrap();
    }
    map.into()
}

#[test]
fn a_field_with_no_condition_is_shown() {
    let schema = schema();
    let form = Form::new().finish().unwrap();
    let (s, f) = views(&schema, &form);
    assert!(shown(s, f, "host", &values(vec![])));
}

#[test]
fn a_condition_shows_the_field_only_while_it_is_met() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("verify", true))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    assert!(shown(
        s,
        f,
        "ca",
        &values(vec![("verify", Value::from(true))])
    ));
    assert!(!shown(
        s,
        f,
        "ca",
        &values(vec![("verify", Value::from(false))])
    ));
    assert!(
        shown(s, f, "ca", &values(vec![])),
        "`verify` holds nothing yet, so it reads as its schema default, which is true"
    );
}

#[test]
fn a_field_with_no_value_and_no_default_does_not_meet_a_condition() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("host", "example.test"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    assert!(!shown(s, f, "ca", &values(vec![])));
}

/// A variant named by its own key reads as the arm a person picked.
#[test]
fn a_variant_is_compared_by_its_discriminant() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("auth", "userpass"))
        .field("host", Hints::new().visible_when("auth.username", "root"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let mut userpass = Map::new();
    userpass.set("auth", "userpass").unwrap();
    userpass.set("username", "root").unwrap();
    let picked = values(vec![("auth", userpass.into())]);
    assert!(shown(s, f, "ca", &picked));
    assert!(
        shown(s, f, "host", &picked),
        "an arm's field is read inside its variant's value"
    );

    let mut ambient = Map::new();
    ambient.set("auth", "ambient").unwrap();
    let other = values(vec![("auth", ambient.into())]);
    assert!(!shown(s, f, "ca", &other));
    assert!(!shown(s, f, "host", &other));
}

/// A condition on a hidden field is not met, so hiding a field hides
/// everything that waits on it — even when a stale value would satisfy it.
#[test]
fn hiding_a_field_hides_what_waits_on_it() {
    let schema = schema();
    let form = Form::new()
        .field("port", Hints::new().visible_when("verify", true))
        .field("ca", Hints::new().visible_when("port", 443))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let stale = values(vec![
        ("verify", Value::from(false)),
        ("port", Value::from(443i64)),
    ]);
    assert!(!shown(s, f, "port", &stale));
    assert!(
        !shown(s, f, "ca", &stale),
        "`port` holds 443, but `port` is hidden, so `ca` is too"
    );

    let both = values(vec![
        ("verify", Value::from(true)),
        ("port", Value::from(443i64)),
    ]);
    assert!(shown(s, f, "ca", &both));
}

#[test]
fn a_cycle_is_never_shown() {
    let schema = schema();
    let form = Form::new()
        .field("host", Hints::new().visible_when("port", 1))
        .field("port", Hints::new().visible_when("host", "a"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    let v = values(vec![
        ("host", Value::from(Text::new("a"))),
        ("port", Value::from(1i64)),
    ]);
    assert!(!shown(s, f, "host", &v));
    assert!(!shown(s, f, "port", &v));
}

// --- allocation --------------------------------------------------------------

#[derive(Default)]
struct Counter {
    outstanding: Cell<isize>,
}

unsafe extern "C" fn counted_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Counter` installed below, outliving the call.
    let counter = unsafe { &*(ctx as *const Counter) };
    counter.outstanding.set(counter.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counted_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let counter = unsafe { &*(ctx as *const Counter) };
    counter.outstanding.set(counter.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

/// A form built into a named allocator is built entirely there and frees
/// every block. The `_in` constructors throughout: a plain one would put
/// the allocation on Rust's heap and this would pass while proving nothing.
#[test]
fn a_form_built_through_a_named_allocator_frees_every_block() {
    let counter = Counter::default();
    let vt = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&counter as *const Counter).cast_mut().cast(),
        alloc: Some(counted_alloc),
        free: Some(counted_free),
        release: None,
    };
    // SAFETY: `vt` is fully initialised and outlives every use below.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");

    {
        let form = Form::new_in(alloc)
            .section(Section::new_in(alloc, "net").label("Network"))
            .field(
                "ca",
                Hints::new_in(alloc)
                    .placeholder("/etc/ssl/ca.pem")
                    .visible_when("verify", Value::from(true)),
            )
            .finish()
            .unwrap();
        assert!(
            counter.outstanding.get() > 0,
            "the form was built through the counting allocator"
        );
        let schema = schema();
        let (s, f) = views(&schema, &form);
        check(s, f).unwrap();
    }
    assert_eq!(
        counter.outstanding.get(),
        0,
        "a form is a value, so it frees like one"
    );
}

/// **A path the schema does not declare is an error, not `true`.**
///
/// The same `UnknownField` `check` gives it. Answering `true` would show a
/// field that does not exist — a misspelling in a form renders as a
/// control nothing is behind — and answering `false` would hide it for a
/// reason the caller cannot tell from a condition being unmet.
#[test]
fn a_path_the_schema_does_not_declare_is_refused() {
    let schema = schema();
    let form = Form::new().finish().unwrap();
    let (s, f) = views(&schema, &form);

    assert_eq!(
        is_visible(s, f, "hots", &values(vec![])),
        Err(FormError::UnknownField {
            at: "fields".into(),
            path: "hots".into()
        })
    );
    assert_eq!(
        is_visible(s, f, "auth.nonesuch", &values(vec![])),
        Err(FormError::UnknownField {
            at: "fields".into(),
            path: "auth.nonesuch".into()
        }),
        "a dotted path is resolved through the arm, and this one resolves to nothing"
    );

    // And the refusal is not everything-is-an-error: a declared path still
    // answers.
    assert_eq!(is_visible(s, f, "host", &values(vec![])), Ok(true));
}

/// A condition reading a path the schema does not declare is refused where
/// it is met, not silently unmet.
#[test]
fn a_condition_reading_an_unknown_path_is_refused_too() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("verfy", true))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);
    assert_eq!(
        is_visible(s, f, "ca", &values(vec![])),
        Err(FormError::UnknownField {
            at: "fields".into(),
            path: "verfy".into()
        })
    );
}

/// **An arm's field is read only while its arm is the one picked.**
///
/// `values[owner][member]` alone would answer with whatever the last arm
/// left behind: a variant's value is one map, and moving from `userpass`
/// to `ambient` does not erase `username` from it. The arm that declares
/// the member is what gives it a meaning, so a stale one reads as nothing
/// and the field falls back to its schema default.
#[test]
fn an_arm_field_is_read_only_while_its_arm_is_picked() {
    let schema = schema();
    let form = Form::new()
        .field("ca", Hints::new().visible_when("auth.username", "root"))
        .finish()
        .unwrap();
    let (s, f) = views(&schema, &form);

    let mut picked = Map::new();
    picked.set("auth", "userpass").unwrap();
    picked.set("username", "root").unwrap();
    assert!(
        shown(s, f, "ca", &values(vec![("auth", picked.into())])),
        "`userpass` declares `username`, so the condition reads it"
    );

    // The same map, with the discriminant moved to an arm that declares no
    // `username`. The key is still there; it belongs to nothing now.
    let mut stale = Map::new();
    stale.set("auth", "ambient").unwrap();
    stale.set("username", "root").unwrap();
    assert!(
        !shown(s, f, "ca", &values(vec![("auth", stale.into())])),
        "`ambient` declares no `username`, so the value left behind is not read"
    );
}

// --- a field that is an object carries a form of its own ----------------

/// A schema with three shapes a sub-form can be given to, and one it
/// cannot: an object, a list of objects, an open map of objects, and a
/// plain text.
fn nesting_schema() -> Value {
    let member = || {
        KindBuilder::map(vec![
            FieldBuilder::new("host", KindBuilder::string()),
            FieldBuilder::new("verify", KindBuilder::bool()),
            FieldBuilder::new("ca", KindBuilder::string()),
        ])
    };
    SchemaBuilder::new()
        .field(FieldBuilder::new("connection", member()))
        .field(FieldBuilder::new("agents", KindBuilder::list(member())))
        .field(FieldBuilder::new("named", KindBuilder::map_of(member())))
        .field(FieldBuilder::new("label", KindBuilder::string()))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// The form a member is shown with: its paths are **relative to the
/// field**, so it names `ca`, not `connection.ca`.
fn member_form() -> Form {
    Form::new()
        .section(Section::new("net").label("Network"))
        .field("host", Hints::new().placeholder("10.0.0.1"))
        .field("ca", Hints::new().visible_when("verify", true))
}

/// An object, a list of objects and a map of them each take one.
#[test]
fn a_form_may_be_given_to_anything_with_members() {
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");

    for (field, widget) in [
        ("connection", vocab::widget::DIALOG),
        ("agents", vocab::widget::GROUP),
        ("named", vocab::widget::DIALOG),
    ] {
        let form = Form::new()
            .field(field, Hints::new().widget(widget).form(member_form()))
            .finish()
            .expect("it builds");
        let f = FormRef::new(&form).expect("a form");
        check(s, f).unwrap_or_else(|e| panic!("`{field}` should take a form: {e}"));

        // And a renderer reaches it by asking the hints.
        let nested = f.hints(field).form().expect("the form is there");
        assert_eq!(
            nested.sections().next().expect("one section").label(),
            "Network"
        );
        assert_eq!(nested.hints("host").placeholder(), "10.0.0.1");
        assert_eq!(f.hints(field).widget(), widget);
    }
}

/// A form on a field with no members is refused: there is nothing for
/// its paths to name.
#[test]
fn a_form_on_a_text_field_is_refused() {
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");
    let form = Form::new()
        .field("label", Hints::new().form(member_form()))
        .finish()
        .expect("it builds");
    let refused =
        check(s, FormRef::new(&form).expect("a form")).expect_err("a text has no members");
    match refused {
        FormError::Malformed { at, expected } => {
            assert_eq!(at, "fields[\"label\"].form");
            assert!(expected.contains("object"), "{expected}");
        }
        other => panic!("expected the malformed shape: {other}"),
    }
}

/// A sub-form's paths are checked against the MEMBER, so a path naming
/// the outer form's field is unknown here.
#[test]
fn a_sub_forms_paths_name_the_member_and_nothing_else() {
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");
    let form = Form::new()
        .field(
            "connection",
            Hints::new().form(Form::new().field("label", Hints::new().widget("text"))),
        )
        .finish()
        .expect("it builds");

    let refused = check(s, FormRef::new(&form).expect("a form"))
        .expect_err("`label` is the OUTER form's field, not a member of `connection`");
    match refused {
        FormError::UnknownField { at, path } => {
            assert_eq!(path, "label");
            assert!(
                at.starts_with("fields[\"connection\"].form"),
                "the refusal says whose form it came from: {at}"
            );
        }
        other => panic!("expected the unknown-field shape: {other}"),
    }
}

/// A condition inside a sub-form may only read that form's own fields,
/// which falls out of checking it against the member schema.
#[test]
fn a_condition_cannot_reach_out_of_a_sub_form() {
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");

    // Reaching a sibling INSIDE the member is fine.
    let inside = Form::new()
        .field("connection", Hints::new().form(member_form()))
        .finish()
        .expect("it builds");
    check(s, FormRef::new(&inside).expect("a form")).expect("`ca` may watch `verify`");

    // Reaching the outer form's `label` is not.
    let outward = Form::new()
        .field(
            "connection",
            Hints::new()
                .form(Form::new().field("ca", Hints::new().visible_when("label", "anything"))),
        )
        .finish()
        .expect("it builds");
    let refused =
        check(s, FormRef::new(&outward).expect("a form")).expect_err("`label` is not a member");
    match refused {
        FormError::UnknownField { path, .. } => assert_eq!(path, "label"),
        other => panic!("expected the unknown-field shape: {other}"),
    }
}

/// A sub-form is a form, so everything a form must be, it must be.
#[test]
fn a_sub_form_obeys_every_rule_a_form_obeys() {
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");

    // A section declared twice, one level down.
    let form = Form::new()
        .field(
            "connection",
            Hints::new().form(
                Form::new()
                    .section(Section::new("net"))
                    .section(Section::new("net")),
            ),
        )
        .finish()
        .expect("it builds");
    let refused = check(s, FormRef::new(&form).expect("a form")).expect_err("two sections, one id");
    assert!(
        matches!(refused, FormError::DuplicateSection { ref id } if id == "net"),
        "{refused}"
    );
}

// --- the form a schema implies ------------------------------------------

/// A created form says what the schema says: which sections exist, in
/// first-appearance order, and what a member's own form would be.
#[test]
fn a_created_form_carries_the_sections_the_schema_names() {
    let alloc = Alloc::rust();
    let tls = KindBuilder::map(vec![
        FieldBuilder::new("verify", KindBuilder::bool()).section("trust"),
        FieldBuilder::new("ca", KindBuilder::string()).section("trust"),
        FieldBuilder::new("note", KindBuilder::string()),
    ]);
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("host", KindBuilder::string()).section("net"))
        .field(FieldBuilder::new("tls", tls))
        .field(FieldBuilder::new("port", KindBuilder::int()).section("net"))
        .field(FieldBuilder::new("label", KindBuilder::string()).section("meta"))
        .finish()
        .expect("it builds");
    let s = SchemaRef::new(&schema).expect("a schema");

    let form = for_schema(s, alloc).expect("a form this small builds");
    let f = FormRef::new(&form).expect("what it made is a form");

    // First-appearance order, ids only: what a section is CALLED is a
    // form's business and a schema has no opinion.
    let ids: Vec<&str> = f.sections().map(|s| s.id()).collect();
    assert_eq!(ids, ["net", "meta"]);
    assert_eq!(f.sections().next().expect("one").label(), "");

    // The nested object carries its own created form, with its own
    // sections, under the `form` hint.
    let nested = f.hints("tls").form().expect("tls has members");
    let nested_ids: Vec<&str> = nested.sections().map(|s| s.id()).collect();
    assert_eq!(nested_ids, ["trust"]);

    // And what it made fits what it was made from.
    check(s, f).expect("a created form fits its schema");
}

/// A schema whose fields name no section implies nothing, and says so:
/// an empty form, which is a complete one.
#[test]
fn a_created_form_is_empty_when_the_schema_groups_nothing() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("host", KindBuilder::string()))
        .field(FieldBuilder::new("port", KindBuilder::int()))
        .finish()
        .expect("it builds");
    let s = SchemaRef::new(&schema).expect("a schema");

    let form = for_schema(s, alloc).expect("it builds");
    let f = FormRef::new(&form).expect("a form");
    assert!(f.sections().next().is_none());
    assert!(f.fields().next().is_none(), "no member had anything to say");
    check(s, f).expect("an empty form is a complete one");
}

/// `for_field` answers for what a field HAS, and `None` for what it does
/// not: a text is shown by a control, not by a form.
#[test]
fn for_field_answers_only_where_there_are_members() {
    let alloc = Alloc::rust();
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");

    for key in ["connection", "agents", "named"] {
        let field = s.find(key).expect("declared");
        let made = for_field(field, alloc)
            .unwrap_or_else(|| panic!("`{key}` has members"))
            .expect("it builds");
        assert!(FormRef::new(&made).is_some(), "`{key}` made a form");
    }
    assert!(
        for_field(s.find("label").expect("declared"), alloc).is_none(),
        "a text has no members to make a form out of"
    );
}

/// `form_for` is the question a renderer asks: the assigned form, or the
/// one the schema implies.
#[test]
fn form_for_prefers_what_somebody_wrote() {
    let alloc = Alloc::rust();
    let schema = nesting_schema();
    let s = SchemaRef::new(&schema).expect("a schema");

    // Nothing assigned: the schema still groups the member's fields, so
    // there is an answer.
    let bare = Form::new().finish().expect("an empty form");
    let bare = FormRef::new(&bare).expect("a form");
    assert!(
        form_for(bare, s, "connection", alloc).is_some(),
        "an object is a form whether or not anybody wrote one"
    );
    assert!(
        form_for(bare, s, "label", alloc).is_none(),
        "and a text is not"
    );
    assert!(form_for(bare, s, "nonesuch", alloc).is_none());

    // Assigned: that one wins, and comes back owned.
    let written = Form::new()
        .field(
            "connection",
            Hints::new().form(Form::new().section(Section::new("written").label("Written"))),
        )
        .finish()
        .expect("it builds");
    let written = FormRef::new(&written).expect("a form");
    let chosen = form_for(written, s, "connection", alloc)
        .expect("assigned")
        .expect("it copies");
    let chosen = FormRef::new(&chosen).expect("a form");
    assert_eq!(
        chosen.sections().next().expect("one section").label(),
        "Written",
        "assignment wins: somebody wrote it down"
    );
}
