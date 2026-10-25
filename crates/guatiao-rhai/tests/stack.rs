// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai recurses on the native stack, for calls and for the values it
//! nests. These scripts would overflow the engine thread's 2 MiB and abort
//! the process; each must instead end in an answer or a stop that names
//! its limit. They run in release builds too, where frames are smaller and
//! the bounds are tighter.

mod common;

use common::*;
use greeter_kind::Listener;
use guatiao::Status;

/// The ways to nest one more expression around the recursive call.
const SHAPES: [(&str, &str, &str); 3] = [
    ("calls", "f1(", ")"),
    ("sum", "1 + (", ")"),
    ("switch", "switch 1 { 1 => ", " }"),
];

/// A library whose `greet` recurses with `nest` expressions wrapped around
/// the call at every level.
fn recursing(open: &str, close: &str, nest: usize) -> String {
    format!(
        r#"
const VERSION = "1.0.0";
fn f1(x) {{ x }}
fn down(n) {{
    if n > 1000000 {{ return 0; }}
    {}down(n + 1){}
}}
let p = #{{ kinds: ["greeter"], greet: |name| {{ down(0); #{{ greeting: "x" }} }} }};
#{{ providers: #{{ p: p }} }}
"#,
        open.repeat(nest),
        close.repeat(nest)
    )
}

/// The deepest `nest` the engine's expression limit lets a script write.
fn deepest(label: &str, open: &str, close: &str) -> usize {
    let mut best = 0;
    for nest in 1..=40 {
        // Two tests probe at once: each in directories of its own.
        let thread = format!("{:?}", std::thread::current().id()).replace(['(', ')'], "");
        let dir = scratch(&format!("stack-probe-{label}-{nest}-{thread}"));
        put(&dir, "rhai_probe", &recursing(open, close, nest));
        let mut registry = serving(&config(&dir, None));
        let loads = registry.refresh().refused.is_empty();
        let _ = std::fs::remove_dir_all(&dir);
        if !loads {
            break;
        }
        best = nest;
    }
    best
}

#[test]
fn recursion_through_the_deepest_expressions_stops_at_the_depth_limit() {
    let dir = scratch("stack-calls");
    for (label, open, close) in SHAPES {
        let nest = deepest(label, open, close);
        assert!(nest >= 3, "{label}: expressions nest {nest} deep at most");
        put(&dir, "rhai_deep", &recursing(open, close, nest));
        // No depth set, then depths a host may ask for.
        for depth in [None, Some(150u64), Some(1000), Some(3000)] {
            let set = depth.map(|d| limits(&[("depth", d)]));
            let mut registry = serving(&config(&dir, set));
            assert!(
                registry
                    .refresh()
                    .refused
                    .iter()
                    .all(|r| r.id.as_deref() != Some("rhai_deep"))
            );
            let stopped = greeter(&registry, "rhai_deep_p").greet("go").unwrap_err();
            assert_eq!(stopped.status, Status::GUATIAO_ERR_INTERNAL, "{label}");
            assert_eq!(
                stopped.message(),
                format!("stopped at its depth limit ({})", depth.unwrap_or(32)),
                "{label} nested {nest} deep"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_expression_deeper_than_the_limit_is_refused_when_the_library_opens() {
    let dir = scratch("stack-refused");
    let nest = deepest("calls", "f1(", ")");
    put(&dir, "rhai_toodeep", &recursing("f1(", ")", nest + 8));
    let mut registry = serving(&config(&dir, None));
    let changed = registry.refresh();
    let reason = changed
        .refused
        .iter()
        .find(|r| r.id.as_deref() == Some("rhai_toodeep"))
        .map(|r| r.reason.clone())
        .expect("refused");
    assert!(reason.contains("maximum complexity"), "{reason}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nests `n` levels with one call and drops them when the call ends.
fn nesting(build: &str) -> String {
    format!(
        r#"
const VERSION = "1.0.0";
let p = #{{ kinds: ["greeter"], greet: |name| {{
    let n = parse_int(name);
    {build}
    #{{ greeting: "built" }}
}} }};
#{{ providers: #{{ p: p }} }}
"#
    )
}

const LIMITS: [(&str, u64); 3] = [
    ("operations", 10_000_000),
    ("memory", 64 << 20),
    ("time_ms", 60_000),
];

/// One call nests `n` levels with `build` and drops them when it ends.
fn nested_in_one_call(id: &str, build: &str, n: usize) {
    let dir = scratch(id);
    put(&dir, id, &nesting(build));
    let mut registry = serving(&config(&dir, Some(limits(&LIMITS))));
    assert!(registry.refresh().refused.is_empty());
    let answer = greeter(&registry, &format!("{id}_p"))
        .greet(&n.to_string())
        .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    assert!(answer.get("greeting").is_some(), "{id}");
    let _ = std::fs::remove_dir_all(&dir);
}

// Arrays and maps copy all they hold at each level, so a script gets less
// deep with them in the time a test has; closures that capture the last
// one share it. Each count is past what the engine thread's 2 MiB holds.

#[test]
fn arrays_nested_deeper_than_the_stack_are_built_and_dropped_inside_a_call() {
    let n = if cfg!(debug_assertions) {
        2_500
    } else {
        11_000
    };
    nested_in_one_call("rhai_arrays", "let a = []; for i in 0..n { a = [a]; }", n);
}

#[test]
fn maps_nested_deeper_than_the_stack_are_built_and_dropped_inside_a_call() {
    let n = if cfg!(debug_assertions) { 1_500 } else { 6_500 };
    nested_in_one_call(
        "rhai_maps",
        "let a = #{}; for i in 0..n { a = #{ x: a }; }",
        n,
    );
}

#[test]
fn closures_nested_deeper_than_the_stack_are_built_and_dropped_inside_a_call() {
    let n = if cfg!(debug_assertions) {
        100_000
    } else {
        300_000
    };
    nested_in_one_call(
        "rhai_closures",
        "let f = || 1; for i in 0..n { let g = f; f = || g.call(); }",
        n,
    );
}

#[test]
fn nesting_without_end_is_stopped_by_the_operations_limit_and_dropped() {
    let dir = scratch("stack-forever");
    put(
        &dir,
        "rhai_forever",
        &nesting("let f = || 1; loop { let g = f; f = || g.call(); }"),
    );
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 3_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let p = greeter(&registry, "rhai_forever_p");
    let stopped = p.greet("0").unwrap_err();
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (3000000)"
    );
    // The engine, and what it dropped, are fine.
    assert_eq!(stopped.status, Status::GUATIAO_ERR_INTERNAL);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_object_that_keeps_deep_values_is_dropped_when_the_host_lets_go() {
    let dir = scratch("stack-object");
    let code = r#"
const VERSION = "1.0.0";
let p = #{ kinds: ["greeter"],
    greet: |name| #{ greeting: name },
    start: |listener| #{
        chain: {
            let f = || 1;
            for i in 0..150000 { let g = f; f = || g.call(); }
            f
        },
        say: |what| (),
        read: |dst| 0,
        turns: || 0,
    },
};
#{ providers: #{ p: p } }
"#;
    put(&dir, "rhai_keeps", code);
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 10_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let p = greeter(&registry, "rhai_keeps_p");
    struct Quiet;
    impl Listener for Quiet {
        fn heard(&mut self, _: &str) {}
    }
    let kept = p.start(Quiet.into_object()).expect("started");
    drop(kept);
    drop(p);
    drop(registry);
    let _ = std::fs::remove_dir_all(&dir);
}
