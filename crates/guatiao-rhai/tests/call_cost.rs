// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A trivial call costs the same whatever stack the limits reserve: nothing
//! in proportion to the stack is paid per call. On Windows, a stack set up
//! for a call is committed whole, which costs time with its size.

mod common;

use core::time::Duration;
use std::time::Instant;

use common::*;

const CODE: &str = r#"
const VERSION = "1.0.0";
let p = #{ kinds: ["greeter"], greet: |name| #{ greeting: name } };
#{ providers: #{ p: p } }
"#;

/// The median time of a trivial call under `operations`.
fn per_call(operations: Option<u64>) -> Duration {
    let dir = scratch(&format!("cost-{}", operations.unwrap_or(0)));
    put(&dir, "rhai_cost", CODE);
    let set = operations.map(|n| limits(&[("operations", n)]));
    let mut registry = serving(&config(&dir, set));
    assert!(registry.refresh().refused.is_empty());
    let p = greeter(&registry, "rhai_cost_p");
    for _ in 0..20 {
        p.greet("warm").expect("answered");
    }
    let mut times: Vec<Duration> = (0..200)
        .map(|_| {
            let started = Instant::now();
            p.greet("x").expect("answered");
            started.elapsed()
        })
        .collect();
    times.sort();
    drop(p);
    drop(registry);
    let _ = std::fs::remove_dir_all(&dir);
    times[times.len() / 2]
}

#[test]
fn a_trivial_call_costs_the_same_under_any_operations_limit() {
    let free = per_call(None);
    let mut limits = vec![1_000_000, 2_000_000];
    // The cap reserves 11 GB (51 GB in a debug build). Windows reserves
    // address space alone; Linux refuses more than memory and swap.
    if cfg!(windows) {
        limits.extend([10_000_000, 16_777_216]);
    }
    for ops in limits {
        let limited = per_call(Some(ops));
        println!("operations {ops}: {limited:?} a call, {free:?} with no limit");
        assert!(
            limited <= free * 3 + Duration::from_micros(50),
            "operations {ops}: {limited:?} a call against {free:?} with no limit"
        );
    }
}
