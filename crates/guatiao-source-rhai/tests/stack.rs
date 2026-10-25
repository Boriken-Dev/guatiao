// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai recurses on the native stack, for calls and for the values it
//! nests. These scripts would overflow a thread's default 2 MiB and abort
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

/// Operations enough for the deepest of these scripts. The engine thread
/// reserves stack for each (3 KiB in a debug build), and Linux refuses a
/// reservation larger than the machine's memory and swap.
const OPS: u64 = if cfg!(debug_assertions) {
    2_000_000
} else {
    4_000_000
};

const LIMITS: [(&str, u64); 3] = [
    ("operations", OPS),
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
// one share it. Each count is past what a thread's default 2 MiB holds.

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
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 2_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let p = greeter(&registry, "rhai_forever_p");
    let stopped = p.greet("0").unwrap_err();
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (2000000)"
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
    // Enough operations for the chain, nine a level: more than the hundred
    // thousand levels a kept value is dropped for without a limit.
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 2_000_000)]))));
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

/// Nesting `n` arrays deep with `take`, which moves rather than copies, so
/// each level costs a few operations and no copy.
const DEEP: &str = "let a = []; for i in 0..n { a = [take(a)]; }";

/// Levels past what dropping or copying a nested value on a thread's
/// default 2 MiB holds (96 and 273 bytes a level in a release build, 321
/// and 1 351 in a debug one).
fn past_the_thread() -> usize {
    if cfg!(debug_assertions) {
        20_000
    } else {
        60_000
    }
}

fn greets(
    id: &str,
    code: &str,
    arg: &str,
    ops: u64,
) -> Result<(), guatiao::library::ProviderError> {
    let dir = scratch(id);
    put(&dir, id, code);
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", ops)]))));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let answer = greeter(&registry, &format!("{id}_p"))
        .greet(arg)
        .map(|_| ());
    let _ = std::fs::remove_dir_all(&dir);
    answer
}

fn library(build: &str, answer: &str) -> String {
    format!(
        r#"
const VERSION = "1.0.0";
let p = #{{ kinds: ["greeter"], greet: |name| {{
    let n = parse_int(name);
    {build}
    {answer}
}} }};
#{{ providers: #{{ p: p }} }}
"#
    )
}

#[test]
fn a_deep_answer_is_refused_and_a_deep_thrown_value_dropped() {
    let n = past_the_thread().to_string();
    let refused = greets(
        "rhai_deep_answer",
        &library(DEEP, "#{ greeting: a }"),
        &n,
        OPS,
    )
    .unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(
        refused.message().contains("nested deeper than"),
        "{}",
        refused.message()
    );
    let thrown = greets("rhai_deep_throw", &library(DEEP, "throw a;"), &n, OPS).unwrap_err();
    assert_eq!(thrown.message(), "a script threw a array");
}

#[test]
fn what_a_script_ends_with_and_does_not_keep_is_dropped() {
    let dir = scratch("stack-open-leftover");
    let code = format!(
        r#"
const VERSION = "1.0.0";
let n = {};
{DEEP}
let p = #{{ kinds: ["greeter"], greet: |name| #{{ greeting: name }} }};
#{{ providers: #{{ p: p }}, left: a }}
"#,
        past_the_thread()
    );
    put(&dir, "rhai_leftover", &code);
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", OPS)]))));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert!(greeter(&registry, "rhai_leftover_p").greet("x").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether `answer` is an answer, or a stop at the depth or operations
/// limit. A thread can be given more stack than it asked for (glibc hands
/// a new thread a larger stack a finished one left), and then a recursion
/// the check would have stopped finishes, or runs out of operations.
fn answered_or_stopped(label: &str, answer: Result<(), guatiao::library::ProviderError>) {
    if let Err(e) = answer {
        assert!(
            e.message() == "stopped at its depth limit (32)"
                || e.message().starts_with("stopped at its operations limit"),
            "{label}: {}",
            e.message()
        );
    }
}

#[test]
fn operations_that_recurse_over_nested_values_stop_at_the_depth_limit() {
    // Comparing and writing a nested value recurse through Rhai's own
    // calls, about 2 KiB of native stack a level in a release build, more
    // than the stack holds for each operation: the check between
    // operations stops them.
    let n: u64 = if cfg!(debug_assertions) {
        60_000
    } else {
        200_000
    };
    for (label, op) in [
        ("eq", "let same = a == a;"),
        ("string", "let s = a.to_string();"),
        ("debug", "let s = a.to_debug();"),
        ("interpolation", "let s = `${a}`;"),
    ] {
        let code = library(&format!("{DEEP} {op}"), "#{ greeting: \"done\" }");
        let answer = greets(
            &format!("rhai_recurse_{label}"),
            &code,
            &n.to_string(),
            8 * n + 2_000,
        );
        answered_or_stopped(label, answer);
    }
}

#[test]
fn comparing_a_value_nested_nearly_as_deep_as_the_operations_allow_stops() {
    // Twelve literals a statement build a level for 17/12 operations, and
    // comparing takes one a level: with just enough operations for both,
    // the stack holds about 1.5 KiB a level, less than comparing takes
    // through Rhai's calls in a release build. Without the check between
    // operations this overflows the engine thread's stack there; a debug
    // build's 7.3 KiB a level holds the comparison.
    let steps: u64 = if cfg!(debug_assertions) {
        25_000
    } else {
        100_000
    };
    let ops = 30 * steps + 5_000;
    let build = format!(
        "let a = (); for i in 0..n {{ a = {}take(a){}; }}",
        "[".repeat(12),
        "]".repeat(12)
    );
    for (label, op) in [
        ("eq", "let same = a == a;"),
        ("contains", "let has = [a].contains(a);"),
    ] {
        let id = format!("rhai_near_limit_{label}");
        let code = library(&format!("{build} {op}"), "#{ greeting: \"done\" }");
        // Built and compared, or stopped short of the stack's end; never
        // out of operations, which would mean nothing deep was compared.
        if let Err(e) = greets(&id, &code, &steps.to_string(), ops) {
            assert_eq!(e.message(), "stopped at its depth limit (32)", "{id}");
        }
    }
}

#[test]
fn literals_nested_a_dozen_a_statement_are_copied_written_and_compared() {
    // Twelve literals a statement nest a level for about 1.4 operations.
    // Copying and dropping the result recurse within one operation, which
    // the check between operations cannot see: the stack must hold them.
    // Writing and comparing it recurse through Rhai's calls, which the
    // check stops (or, on a thread given more stack, finish).
    for (shape, open, close, steps, ops) in [
        ("arrays", "[", "]", 100_000, 2_000_000),
        ("maps", "#{ x: ", " }", 10_000, 200_000),
    ] {
        let build = format!(
            "let a = (); for i in 0..n {{ a = {}take(a){}; }}",
            open.repeat(12),
            close.repeat(12)
        );
        for (label, op, answers) in [
            ("dropped", "", true),
            ("copied", "let b = a;", true),
            ("written", "let s = a.to_debug();", false),
            ("compared", "let same = a == a;", false),
        ] {
            let id = format!("rhai_literals_{shape}_{label}");
            let code = library(&format!("{build} {op}"), "#{ greeting: \"done\" }");
            let answer = greets(&id, &code, &steps.to_string(), ops);
            if answers {
                answer.unwrap_or_else(|e| panic!("{id}: {}", e.message()));
            } else {
                answered_or_stopped(&id, answer);
            }
        }
    }
}

#[test]
fn a_limit_past_its_cap_is_refused_before_any_stack_is_reserved() {
    // At the cap the stack would be gigabytes, which a small Linux machine
    // refuses as the thread starts: the limit's own words come first.
    let dir = scratch("stack-caps");
    for (key, past, words) in [
        ("operations", 16_777_217, "`operations` is at most 16777216"),
        ("depth", 8193, "`depth` is at most 8192"),
    ] {
        let refused = guatiao_bridge::BridgeSource::<guatiao_source_rhai::RhaiEngine>::pinned(
            &config(&dir, Some(limits(&[(key, past)]))),
            "rhai-caps",
        )
        .err()
        .unwrap_or_else(|| panic!("{key} {past} is refused"));
        assert!(refused.to_string().contains(words), "{refused}");
        let status: guatiao::library::ProviderError = refused.into();
        assert_eq!(status.status, Status::GUATIAO_ERR_BAD_VALUE, "{key}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_of_a_map_nested_too_deep_is_refused() {
    let code = library(
        &format!("{DEEP} let m = #{{ a: a }};"),
        "#{ greeting: m.to_json() }",
    );
    let refused = greets("rhai_json", &code, "2000", 1_000_000).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        refused.message(),
        "a map nested deeper than 512 levels, or holding more than 1000000 values, is not written as JSON"
    );
    // Closures sharing what they capture: forty levels are 2^40 values as
    // JSON writes them, refused rather than walked.
    let shared = library(
        "let a = #{}; for i in 0..40 { let b = a; let f = || b; a = #{ x: f, y: f }; } let m = #{ a: a };",
        "#{ greeting: m.to_json() }",
    );
    let started = std::time::Instant::now();
    let refused = greets("rhai_json_shared", &shared, "0", 1_000_000).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(started.elapsed() < core::time::Duration::from_secs(20));
    let fine = library("let m = #{ a: [[1]] };", "#{ greeting: m.to_json() }");
    assert!(greets("rhai_json_fine", &fine, "0", 1_000_000).is_ok());
}

#[test]
fn recursion_through_eval_stops_at_the_depth_limit() {
    let dir = scratch("stack-eval");
    put(
        &dir,
        "rhai_eval",
        r#"
const VERSION = "1.0.0";
fn e(n) { if n == 0 { 1 } else { 1 + eval("e(" + (n - 1) + ")") } }
let p = #{ kinds: ["greeter"], greet: |name| { e(parse_int(name)); #{ greeting: name } } };
#{ providers: #{ p: p } }
"#,
    );
    let mut registry = serving(&config(&dir, Some(limits(&[("depth", 4000)]))));
    assert!(registry.refresh().refused.is_empty());
    let stopped = greeter(&registry, "rhai_eval_p")
        .greet("20000")
        .unwrap_err();
    assert_eq!(stopped.message(), "stopped at its depth limit (4000)");
    let _ = std::fs::remove_dir_all(&dir);
}
