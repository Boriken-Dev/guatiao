// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! guatiao's values as JavaScript's and back: the shared vectors, and the
//! rules for what a script writes itself.

use guatiao::value::convert::TryAsRef;
use guatiao::value::types::Number;
use guatiao::{List, Map, Tag, Value};
use guatiao_bridge::Convert;
use guatiao_bridge::conformance::{Outcome, round_trip};
use guatiao_js::JsValues;
use guatiao_js::rquickjs::{Context, Ctx, Runtime, Value as Js};

/// Runs `f` with conversions in a fresh context.
fn with_values<R>(f: impl for<'js> FnOnce(&Ctx<'js>, &JsValues<'js>) -> R) -> R {
    let runtime = Runtime::new().expect("a runtime");
    let context = Context::full(&runtime).expect("a context");
    context.with(|ctx| {
        let values = JsValues::new(&ctx).expect("the conversions");
        f(&ctx, &values)
    })
}

fn from_js<'js>(ctx: &Ctx<'js>, values: &JsValues<'js>, code: &str) -> Result<Value, String> {
    let v: Js<'js> = ctx
        .eval(code)
        .map_err(|e| format!("{e}: {:?}", ctx.catch()))?;
    values.to_value(v).map_err(|e| e.to_string())
}

#[test]
fn every_shared_vector_comes_back_the_same() {
    with_values(|_, values| {
        let outcomes = round_trip(values);
        assert!(outcomes.len() >= 20, "every vector ran");
        for (name, outcome) in outcomes {
            assert!(matches!(outcome, Outcome::Same), "{name}: {outcome:?}");
        }
    });
}

#[test]
fn numbers_are_javascript_numbers_where_one_holds_them_exactly() {
    with_values(|ctx, values| {
        for (text, ty) in [
            ("0", "number"),
            ("-7", "number"),
            ("9007199254740992", "number"),
            ("9007199254740993", "bigint"),
            ("9223372036854775807", "bigint"),
            ("18446744073709551615", "bigint"),
            ("340282366920938463463374607431768211455", "bigint"),
            ("1.5", "number"),
            ("0.1", "number"),
            ("1.10", "object"),
            ("1e3", "object"),
            ("-0", "number"),
            ("123456789.123456789123", "object"),
        ] {
            let native = values
                .to_native(&Value::from(Number::new(text).expect("a number")))
                .expect("converted");
            ctx.globals().set("n", native).expect("set");
            let seen: String = ctx.eval("typeof n").expect("typed");
            assert_eq!(seen, ty, "{text}");
            let back = values
                .to_value(ctx.globals().get("n").expect("n"))
                .expect("back");
            let back: &Number = TryAsRef::try_as_ref(&back).expect("a number");
            assert_eq!(&**back, text, "comes back as it went");
        }
        // An exact number still computes, through its float.
        let sum: f64 = ctx
            .eval("new (Symbol.for('guatiao.core') in globalThis ? globalThis[Symbol.for('guatiao.core')].ExactNumber : Object)('1.10') * 2")
            .expect("multiplied");
        assert!((sum - 2.2).abs() < 1e-12);
    });
}

#[test]
fn a_script_writes_numbers_as_it_means_them() {
    with_values(|ctx, values| {
        let number = |code: &str| -> String {
            let v = from_js(ctx, values, code).expect("a number");
            let n: &Number = TryAsRef::try_as_ref(&v).expect("a number");
            n.to_string()
        };
        assert_eq!(number("5"), "5");
        assert_eq!(number("5.0"), "5");
        assert_eq!(number("2 ** 40"), "1099511627776");
        assert_eq!(number("0.5"), "0.5");
        assert_eq!(number("12345678901234567890n"), "12345678901234567890");
        assert_eq!(number("-(2n ** 70n)"), "-1180591620717411303424");
        assert_eq!(number("-0"), "-0");
        assert!(
            from_js(ctx, values, "NaN")
                .unwrap_err()
                .contains("not finite")
        );
        assert!(
            from_js(ctx, values, "Infinity")
                .unwrap_err()
                .contains("not finite")
        );
    });
}

#[test]
fn plain_data_is_a_list_or_a_map_and_anything_else_is_refused() {
    with_values(|ctx, values| {
        let list = from_js(ctx, values, "[1, 'two', true]").expect("a list");
        assert_eq!(
            TryAsRef::<List>::try_as_ref(&list).map(|l| l.len()),
            Some(3)
        );
        let map = from_js(ctx, values, "({ b: 1, a: 2 })").expect("a map");
        let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&map)
            .expect("a map")
            .keys()
            .collect();
        assert_eq!(
            keys,
            ["b", "a"],
            "a plain object's keys in the order it has them"
        );
        let empty = from_js(ctx, values, "({})").expect("empty");
        assert!(TryAsRef::<Map>::try_as_ref(&empty).is_some());
        let empty = from_js(ctx, values, "[]").expect("empty");
        assert!(TryAsRef::<List>::try_as_ref(&empty).is_some());
        let bare =
            from_js(ctx, values, "Object.assign(Object.create(null), { x: 1 })").expect("a map");
        assert!(
            TryAsRef::<Map>::try_as_ref(&bare).is_some(),
            "no prototype is plain too"
        );

        let cycle = from_js(
            ctx,
            values,
            "(() => { const t = {}; t.me = t; return t; })()",
        )
        .unwrap_err();
        assert!(cycle.contains("contains itself"), "{cycle}");
        let function = from_js(ctx, values, "({ f() {} })").unwrap_err();
        assert!(function.contains("function has no value"), "{function}");
        let symbol = from_js(ctx, values, "Symbol('x')").unwrap_err();
        assert!(symbol.contains("symbol has no value"), "{symbol}");
        let date = from_js(ctx, values, "new Date(0)").unwrap_err();
        assert!(date.contains("a Date is not plain data"), "{date}");
        let class = from_js(ctx, values, "new (class Point { x = 1 })()").unwrap_err();
        assert!(class.contains("a Point is not plain data"), "{class}");
        let deep = from_js(
            ctx,
            values,
            "(() => { let t = []; for (let i = 0; i < 600; i++) t = [t]; return t; })()",
        )
        .unwrap_err();
        assert!(deep.contains("deeper than 128"), "{deep}");
        let shared = from_js(
            ctx,
            values,
            "(() => { const a = {}; return { x: a, y: a }; })()",
        );
        assert!(shared.is_ok(), "the same object twice is not a cycle");
    });
}

#[test]
fn text_bytes_null_and_absent_keep_apart() {
    with_values(|ctx, values| {
        let v = from_js(
            ctx,
            values,
            "['text', new Uint8Array([116, 101]), new Uint8Array([1, 2, 3, 4]).subarray(1, 3).buffer.slice(0, 2), null, undefined, '\\u00e9\\u{1f600}']",
        )
        .expect("a list");
        let list: &List = TryAsRef::try_as_ref(&v).expect("a list");
        let items: Vec<&Value> = list.iter().collect();
        assert_eq!(TryAsRef::<str>::try_as_ref(items[0]), Some("text"));
        assert_eq!(TryAsRef::<[u8]>::try_as_ref(items[1]), Some(&b"te"[..]));
        assert_eq!(TryAsRef::<[u8]>::try_as_ref(items[2]), Some(&[1u8, 2][..]));
        assert_eq!(items[3].tag(), Ok(Tag::GUATIAO_NULL));
        assert_eq!(items[4].tag(), Ok(Tag::GUATIAO_ABSENT));
        assert_eq!(
            TryAsRef::<str>::try_as_ref(items[5]),
            Some("\u{e9}\u{1f600}")
        );
        let none = from_js(ctx, values, "undefined").expect("undefined");
        assert_eq!(none.tag(), Ok(Tag::GUATIAO_ABSENT));
        let lone = from_js(ctx, values, "'\\ud800'").unwrap_err();
        assert!(lone.contains("valid Unicode"), "{lone}");
        // A view of a larger buffer gives its window, not the buffer.
        let window = from_js(
            ctx,
            values,
            "new Uint8Array(new Uint8Array([9, 8, 7, 6]).buffer, 1, 2)",
        )
        .expect("a window");
        assert_eq!(TryAsRef::<[u8]>::try_as_ref(&window), Some(&[8u8, 7][..]));
    });
}

#[test]
fn a_key_named_proto_is_data() {
    with_values(|ctx, values| {
        let mut map = Map::new();
        map.set("__proto__", 1i64);
        map.set("z", 2i64);
        let native = values.to_native(&Value::from(map)).expect("converted");
        ctx.globals().set("m", native).expect("set");
        let own: bool = ctx
            .eval("Object.prototype.hasOwnProperty.call(m, '__proto__') && Object.getPrototypeOf(m) === Object.prototype")
            .expect("checked");
        assert!(own);
        let back = values
            .to_value(ctx.globals().get("m").expect("m"))
            .expect("back");
        let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&back)
            .expect("a map")
            .keys()
            .collect();
        assert_eq!(keys, ["__proto__", "z"]);
    });
}

#[test]
fn a_conversion_leaves_no_exception_pending() {
    // Asking whether an object is an ArrayBuffer throws inside QuickJS;
    // the conversion must clear it, or a later import takes it for its own.
    with_values(|ctx, values| {
        from_js(ctx, values, "({ a: [1, { b: 2 }] })").expect("converted");
        assert!(!ctx.has_exception());
    });
}
