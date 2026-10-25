// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a script has beside Rhai's own: the `guatiao` module (`refuse`,
//! `number`, `ABSENT`) and the exact number type's operators.

use guatiao::Status;
use guatiao::value::types::Number;
use rhai::plugin::RhaiResult;
use rhai::{
    Dynamic, Engine, EvalAltResult, FLOAT, FuncRegistration, INT, ImmutableString, Module, Position,
};

use crate::calls::{Refusal, STATUSES, raised, status_name};
use crate::values::{Absent, Exact, number_to_dynamic};

/// A status from the name or number a script gives.
fn status_of(name: &str) -> Result<Status, Box<EvalAltResult>> {
    STATUSES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .ok_or_else(|| {
            raised(Refusal {
                status: Status::GUATIAO_ERR_BAD_VALUE,
                message: format!("`{name}` is not a status name"),
            })
        })
}

fn status_number(code: INT) -> Result<Status, Box<EvalAltResult>> {
    u32::try_from(code).map(Status::from).map_err(|_| {
        raised(Refusal {
            status: Status::GUATIAO_ERR_BAD_VALUE,
            message: "a status is a name or a number from 0".into(),
        })
    })
}

/// The `guatiao` module a script reaches as `guatiao::refuse(..)`.
fn module() -> Module {
    let mut m = Module::new();
    m.set_var("ABSENT", Absent);
    m.set_var("VERSION", env!("CARGO_PKG_VERSION"));
    let mut status = rhai::Map::new();
    for (name, s) in STATUSES {
        status.insert(name.into(), Dynamic::from(INT::from(u32::from(s))));
    }
    m.set_var("STATUS", status);
    FuncRegistration::new("refuse").set_into_module(&mut m, |why: ImmutableString| Refusal {
        status: Status::GUATIAO_ERR_BAD_VALUE,
        message: why.to_string(),
    });
    FuncRegistration::new("refuse").set_into_module(
        &mut m,
        |status: ImmutableString, why: ImmutableString| -> Result<Refusal, Box<EvalAltResult>> {
            Ok(Refusal {
                status: status_of(&status)?,
                message: why.to_string(),
            })
        },
    );
    FuncRegistration::new("refuse").set_into_module(
        &mut m,
        |status: INT, why: ImmutableString| -> Result<Refusal, Box<EvalAltResult>> {
            Ok(Refusal {
                status: status_number(status)?,
                message: why.to_string(),
            })
        },
    );
    FuncRegistration::new("number").set_into_module(
        &mut m,
        |text: ImmutableString| -> RhaiResult {
            Number::new(&text)
                .map(|_| number_to_dynamic(&text))
                .map_err(|_| {
                    EvalAltResult::ErrorRuntime(
                        Dynamic::from(Refusal {
                            status: Status::GUATIAO_ERR_BAD_VALUE,
                            message: format!("`{text}` is not a number"),
                        }),
                        Position::NONE,
                    )
                    .into()
                })
        },
    );
    m
}

macro_rules! arithmetic {
    ($engine:expr, $op:literal, $f:expr) => {{
        let f: fn(FLOAT, FLOAT) -> FLOAT = $f;
        $engine.register_fn($op, move |a: Exact, b: Exact| f(a.float(), b.float()));
        $engine.register_fn($op, move |a: Exact, b: INT| f(a.float(), b as FLOAT));
        $engine.register_fn($op, move |a: INT, b: Exact| f(a as FLOAT, b.float()));
        $engine.register_fn($op, move |a: Exact, b: FLOAT| f(a.float(), b));
        $engine.register_fn($op, move |a: FLOAT, b: Exact| f(a, b.float()));
    }};
}

macro_rules! comparison {
    ($engine:expr, $op:literal, $f:expr) => {{
        let f: fn(FLOAT, FLOAT) -> bool = $f;
        $engine.register_fn($op, move |a: Exact, b: Exact| f(a.float(), b.float()));
        $engine.register_fn($op, move |a: Exact, b: INT| f(a.float(), b as FLOAT));
        $engine.register_fn($op, move |a: INT, b: Exact| f(a as FLOAT, b.float()));
        $engine.register_fn($op, move |a: Exact, b: FLOAT| f(a.float(), b));
        $engine.register_fn($op, move |a: FLOAT, b: Exact| f(a, b.float()));
    }};
}

/// Registers the types and functions a script has beside Rhai's own.
pub(crate) fn register(engine: &mut Engine) {
    engine.register_static_module("guatiao", module().into());

    engine.register_type_with_name::<Refusal>("guatiao::error");
    engine.register_get("status", |r: &mut Refusal| status_name(r.status));
    engine.register_get("code", |r: &mut Refusal| INT::from(u32::from(r.status)));
    engine.register_get("message", |r: &mut Refusal| r.message.clone());
    engine.register_fn("to_string", |r: &mut Refusal| r.to_string());
    engine.register_fn("to_debug", |r: &mut Refusal| r.to_string());

    engine.register_type_with_name::<Absent>("guatiao::absent");
    engine.register_fn("to_string", |_: &mut Absent| "guatiao::ABSENT".to_string());
    engine.register_fn("to_debug", |_: &mut Absent| "guatiao::ABSENT".to_string());
    engine.register_fn("==", |_: Absent, _: Absent| true);
    engine.register_fn("!=", |_: Absent, _: Absent| false);

    engine.register_type_with_name::<Exact>("guatiao::number");
    engine.register_fn("to_string", |e: &mut Exact| e.0.clone());
    engine.register_fn("to_debug", |e: &mut Exact| e.0.clone());
    engine.register_fn("to_float", |e: &mut Exact| e.float());
    arithmetic!(engine, "+", |a, b| a + b);
    arithmetic!(engine, "-", |a, b| a - b);
    arithmetic!(engine, "*", |a, b| a * b);
    arithmetic!(engine, "/", |a, b| a / b);
    comparison!(engine, "<", |a, b| a < b);
    comparison!(engine, "<=", |a, b| a <= b);
    comparison!(engine, ">", |a, b| a > b);
    comparison!(engine, ">=", |a, b| a >= b);
    // The same number is the same text; a number against Rhai's own is
    // compared as floats.
    engine.register_fn("==", |a: Exact, b: Exact| a.0 == b.0);
    engine.register_fn("!=", |a: Exact, b: Exact| a.0 != b.0);
    engine.register_fn("==", |a: Exact, b: INT| a.float() == b as FLOAT);
    engine.register_fn("==", |a: INT, b: Exact| a as FLOAT == b.float());
    engine.register_fn("==", |a: Exact, b: FLOAT| a.float() == b);
    engine.register_fn("==", |a: FLOAT, b: Exact| a == b.float());
    engine.register_fn("!=", |a: Exact, b: INT| a.float() != b as FLOAT);
    engine.register_fn("!=", |a: INT, b: Exact| a as FLOAT != b.float());
    engine.register_fn("!=", |a: Exact, b: FLOAT| a.float() != b);
    engine.register_fn("!=", |a: FLOAT, b: Exact| a != b.float());
}

#[cfg(test)]
mod tests {
    use rhai::Dynamic;

    use super::*;

    fn engine() -> Engine {
        let mut engine = Engine::new();
        register(&mut engine);
        engine
    }

    /// Evaluates `code` and casts the answer to `T`.
    fn eval<T: 'static + Clone>(code: &str) -> T {
        engine()
            .eval::<Dynamic>(code)
            .unwrap_or_else(|e| panic!("{code}: {e}"))
            .cast::<T>()
    }

    /// The words a script's error comes to for a host.
    fn why(e: &EvalAltResult) -> String {
        crate::calls::provider_error(e, &guatiao_bridge::Limits::default())
            .message()
            .to_string()
    }

    #[test]
    fn an_exact_number_prints_as_its_text_and_computes_through_its_float() {
        let n = "guatiao::number(\"1.10\")";
        assert_eq!(eval::<String>(&format!("{n}.to_string()")), "1.10");
        assert_eq!(eval::<String>(&format!("`${{{n}}}`")), "1.10");
        assert!((eval::<FLOAT>(&format!("{n} + 1")) - 2.1).abs() < 1e-12);
        assert!((eval::<FLOAT>(&format!("2 * {n}")) - 2.2).abs() < 1e-12);
        assert!((eval::<FLOAT>(&format!("{n} / 2.0")) - 0.55).abs() < 1e-12);
        assert!(eval::<bool>(&format!("{n} == {n}")));
        assert!(eval::<bool>(&format!("{n} != guatiao::number(\"1.2\")")));
        assert!(eval::<bool>(&format!("{n} < 2 && {n} >= 1.1 && 3 > {n}")));
        // A number Rhai holds exactly is not wrapped.
        assert!(eval::<Dynamic>("guatiao::number(\"7\")").is_int());
        assert!(eval::<Dynamic>("guatiao::number(\"1.5\")").is_float());
    }

    #[test]
    fn a_text_that_is_not_a_number_is_refused() {
        let e = engine()
            .eval::<Dynamic>("guatiao::number(\"one\")")
            .unwrap_err();
        assert!(why(&e).contains("`one` is not a number"), "{}", why(&e));
    }

    #[test]
    fn a_refusal_has_a_status_by_name_or_number_and_words() {
        let words = |code: &str| {
            eval::<String>(&format!(
                "let r = {code}; `${{r.status}} ${{r.code}} ${{r.message}}`"
            ))
        };
        assert_eq!(
            words("guatiao::refuse(\"why\")"),
            format!("bad_value {} why", u32::from(Status::GUATIAO_ERR_BAD_VALUE))
        );
        assert_eq!(
            words("guatiao::refuse(\"gone\", \"w\")"),
            format!("gone {} w", u32::from(Status::GUATIAO_ERR_GONE))
        );
        assert_eq!(words("guatiao::refuse(77, \"w\")"), "77 77 w");
        let e = engine()
            .eval::<Dynamic>("guatiao::refuse(\"nope\", \"w\")")
            .unwrap_err();
        assert!(
            why(&e).contains("`nope` is not a status name"),
            "{}",
            why(&e)
        );
        assert_eq!(
            eval::<INT>("guatiao::STATUS.busy"),
            INT::from(u32::from(Status::GUATIAO_ERR_BUSY))
        );
    }

    #[test]
    fn absent_is_a_value_of_its_own() {
        assert!(eval::<bool>("guatiao::ABSENT == guatiao::ABSENT"));
        assert!(eval::<bool>("guatiao::ABSENT != ()"));
        assert_eq!(
            eval::<String>("guatiao::ABSENT.to_string()"),
            "guatiao::ABSENT"
        );
    }
}
