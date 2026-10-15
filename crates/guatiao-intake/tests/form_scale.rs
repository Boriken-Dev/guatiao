// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A form is a caller's input, so its size is the caller's too: a long
//! chain of `visibleWhen` conditions must neither overflow the stack nor
//! make `check` take longer than the form is long.

use core::time::Duration;
use std::time::Instant;

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::{Map, Value};
use guatiao_intake::{FormError, FormRef, Hints, check, is_visible};

/// `n` boolean fields, `f0 .. f{n-1}`.
fn schema_of(n: usize) -> Value {
    let mut builder = SchemaBuilder::new();
    for i in 0..n {
        builder = builder.field(FieldBuilder::new(&format!("f{i}"), KindBuilder::bool()));
    }
    builder.finish().expect("a schema of booleans")
}

/// A form in which each field is shown when the next one is `true`, the
/// last holding no condition. `links` closes the chain into a ring when
/// it names the field the last one waits on.
fn chain_of(n: usize, ring: Option<usize>) -> Value {
    let mut form = guatiao_intake::Form::new();
    for i in 0..n - 1 {
        form = form.field(
            &format!("f{i}"),
            Hints::new().visible_when(&format!("f{}", i + 1), true),
        );
    }
    if let Some(back) = ring {
        form = form.field(
            &format!("f{}", n - 1),
            Hints::new().visible_when(&format!("f{back}"), true),
        );
    }
    form.finish().expect("a form")
}

fn all_true(n: usize) -> Value {
    let mut values = Map::new();
    for i in 0..n {
        values.set(&format!("f{i}"), true);
    }
    Value::from(values)
}

/// 3 000 links on a stack of 192 KiB, which a walk recursing once per
/// link could not fit.
#[test]
fn a_very_long_chain_of_conditions_is_walked_without_recursion() {
    let walk = || {
        let n = 3_000;
        let schema = schema_of(n);
        let schema = guatiao::schema::read::SchemaRef::new(&schema).unwrap();
        let form = chain_of(n, None);
        let form = FormRef::new(&form).unwrap();

        // Everything above holds `true`, and the last field has no
        // condition, so the head is shown.
        assert_eq!(is_visible(schema, form, "f0", &all_true(n)), Ok(true));
        // One link false anywhere down the chain hides the head.
        let mut values = Map::new();
        for i in 0..n {
            values.set(&format!("f{i}"), i != n / 2);
        }
        assert_eq!(
            is_visible(schema, form, "f0", &Value::from(values)),
            Ok(false)
        );
    };
    std::thread::Builder::new()
        .stack_size(192 * 1024)
        .spawn(walk)
        .expect("a thread")
        .join()
        .expect("the walk finished on a small stack");
}

/// A chain that closes on itself is not shown, wherever it is entered.
#[test]
fn a_ring_of_conditions_is_never_shown_and_check_names_it() {
    let n = 1_000;
    let schema = schema_of(n);
    let schema = guatiao::schema::read::SchemaRef::new(&schema).unwrap();
    let form = chain_of(n, Some(0));
    let form = FormRef::new(&form).unwrap();

    assert_eq!(is_visible(schema, form, "f0", &all_true(n)), Ok(false));
    assert_eq!(is_visible(schema, form, "f500", &all_true(n)), Ok(false));
    assert!(
        matches!(check(schema, form), Err(FormError::CyclicCondition { .. })),
        "the ring is a cycle"
    );
}

/// A tail leading into a cycle is reported at the field that repeats.
#[test]
fn a_chain_leading_into_a_cycle_names_the_field_that_repeats() {
    let schema = schema_of(4);
    let schema = guatiao::schema::read::SchemaRef::new(&schema).unwrap();
    // f0 -> f1 -> f2 -> f3 -> f1
    let form = guatiao_intake::Form::new()
        .field("f0", Hints::new().visible_when("f1", true))
        .field("f1", Hints::new().visible_when("f2", true))
        .field("f2", Hints::new().visible_when("f3", true))
        .field("f3", Hints::new().visible_when("f1", true))
        .finish()
        .unwrap();
    let form = FormRef::new(&form).unwrap();
    assert_eq!(
        check(schema, form),
        Err(FormError::CyclicCondition {
            path: "f1".to_string()
        })
    );
}

/// `check` does not re-walk the chain from every field, which is cubic in
/// its length. An order-of-magnitude check: on a debug build a thousand
/// links take about 0.1 s here and about 13 s when every field re-walks,
/// so the bound sits roughly ten times from each and fails for the
/// algorithm, not for a slow machine.
#[test]
fn check_does_not_rewalk_the_chain_for_every_field() {
    let n = 1_000;
    let schema = schema_of(n);
    let schema = guatiao::schema::read::SchemaRef::new(&schema).unwrap();
    let form = chain_of(n, None);
    let form = FormRef::new(&form).unwrap();

    let started = Instant::now();
    assert_eq!(check(schema, form), Ok(()));
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(1_500),
        "checking a {n}-field chain took {took:?}"
    );
}
