// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Rust caller of any greeter, whatever language wrote it.
//!
//! Two C functions. `greeter_caller_run`: given the host block of a
//! registry (`guatiao_registry_host`) and a provider's key, it finds the
//! greeter through the host's services and calls every slot of the
//! `greeter` kind through its Rust proxy, as a Rust host does, then
//! answers a map of what each call gave. A host written in another
//! language registers a greeter written in that language and calls this
//! to have Rust call it.
//!
//! ```c
//! guatiao_status greeter_caller_run(const guatiao_host_info *host,
//!                                   guatiao_str key, guatiao_str name,
//!                                   guatiao_value *out);
//! ```
//!
//! The answer, in the caller's to free:
//!
//! | key | what |
//! | --- | --- |
//! | `greet` | `greet(name)`'s greeting, or `{status, message}` |
//! | `refusal` | `greet("")`'s `{status, message}`, or the greeting |
//! | `failure` | the status `greet("panic")` failed with, or 0 |
//! | `shout` | `shout(name)` |
//! | `checked`, `filled` | `check` on a map `{name}` lent to it, and the map after |
//! | `greet_from` | `greet_from` with a map `{name}` handed over |
//! | `heard` | what a listener lent to `greet_into(name)` heard |
//! | `conversation` | `{turns, transcript, heard, dropped}` after `start`, two `say`s and a `read`, or `{status, message}` |
//!
//! `greeter_caller_wide` calls a provider of the `wide` kind the same
//! way, with the 64-bit integers at their limits:
//!
//! ```c
//! guatiao_status greeter_caller_wide(const guatiao_host_info *host,
//!                                    guatiao_str key, guatiao_value *out);
//! ```
//!
//! It answers the map `{i64, u64}`: a list of what `echo_i64` answered for
//! 2^53 + 1, `INT64_MIN` and `INT64_MAX`, and of what `echo_u64` answered
//! for 2^53 + 1, `INT64_MAX` + 1 and `UINT64_MAX`; each a number, or
//! `{status, message}` for a refusal.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use greeter_kind::{Greeter, Listener, Wide};
use guatiao::library::{Host, HostInfo, ProviderError};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status, Str, Value};

/// A listener this caller implements and lends or hands to the greeter.
struct Ear {
    heard: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
}

impl Listener for Ear {
    fn heard(&mut self, what: &str) {
        if let Ok(mut heard) = self.heard.lock() {
            heard.push(what.to_string());
        }
    }
}

impl Drop for Ear {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn refusal(e: &ProviderError) -> Map {
    let mut map = Map::new();
    map.set("status", i64::from(u32::from(e.status)));
    map.set("message", e.message());
    map
}

fn greeting(answer: Result<Map, ProviderError>) -> Value {
    match answer {
        Ok(map) => map
            .get("greeting")
            .and_then(TryAsRef::<str>::try_as_ref)
            .map_or_else(Value::null, Value::from),
        Err(e) => Value::from(refusal(&e)),
    }
}

fn texts(items: &Mutex<Vec<String>>) -> List {
    let mut list = List::new();
    for item in items.lock().map(|h| h.clone()).unwrap_or_default() {
        list.push(item);
    }
    list
}

fn named(name: &str) -> Map {
    let mut map = Map::new();
    map.set("name", name);
    map
}

fn run(greeter: &dyn Greeter, name: &str) -> Map {
    let mut report = Map::new();
    report.set("greet", greeting(greeter.greet(name)));
    report.set("refusal", greeting(greeter.greet("")));
    report.set(
        "failure",
        i64::from(
            greeter
                .greet("panic")
                .err()
                .map_or(0, |e| u32::from(e.status)),
        ),
    );
    report.set("shout", greeter.shout(name));

    let mut config = named(name);
    match greeter.check(&mut config) {
        Ok(checked) => report.set("checked", checked),
        Err(e) => report.set("checked", refusal(&e)),
    }
    report.set("filled", config);
    report.set("greet_from", greeting(greeter.greet_from(named(name))));

    let heard = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let mut ear = Ear {
        heard: Arc::clone(&heard),
        dropped: Arc::clone(&dropped),
    };
    if let Err(e) = greeter.greet_into(name, &mut ear) {
        report.set("heard", refusal(&e));
    } else {
        report.set("heard", texts(&heard));
    }
    drop(ear);

    let heard = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let listener = Ear {
        heard: Arc::clone(&heard),
        dropped: Arc::clone(&dropped),
    }
    .into_object();
    let conversation = match greeter.start(listener) {
        Ok(mut chat) => {
            let mut said = Map::new();
            for line in ["hi", "again"] {
                if let Err(e) = chat.say(line) {
                    said.set(line, refusal(&e));
                }
            }
            let mut buffer = [0u8; 64];
            let n = usize::try_from(chat.read(&mut buffer)).unwrap_or(0);
            let mut out = Map::new();
            out.set("turns", chat.turns());
            out.set(
                "transcript",
                String::from_utf8_lossy(&buffer[..n.min(buffer.len())]).into_owned(),
            );
            out.set("refused", said);
            drop(chat);
            out.set("heard", texts(&heard));
            out.set("dropped", dropped.load(Ordering::SeqCst));
            out
        }
        Err(e) => refusal(&e),
    };
    report.set("conversation", conversation);
    report
}

/// The integers `greeter_caller_wide` crosses: each type's limits, and
/// the first whole number a double does not hold.
const SIGNED: [i64; 3] = [(1 << 53) + 1, i64::MIN, i64::MAX];
const UNSIGNED: [u64; 3] = [(1 << 53) + 1, 1 << 63, u64::MAX];

fn answered<T: Into<Value>>(answer: Result<T, ProviderError>) -> Value {
    match answer {
        Ok(x) => x.into(),
        Err(e) => Value::from(refusal(&e)),
    }
}

fn run_wide(wide: &dyn Wide) -> Map {
    let mut signed = List::new();
    for x in SIGNED {
        signed.push(answered(wide.echo_i64(x)));
    }
    let mut unsigned = List::new();
    for x in UNSIGNED {
        unsigned.push(answered(wide.echo_u64(x)));
    }
    let mut report = Map::new();
    report.set("i64", signed);
    report.set("u64", unsigned);
    report
}

/// Calls the greeter filed under `key` through the host's services: every
/// slot, through the Rust proxy, with `name`. Writes the report described
/// in this crate's documentation to `out`, which the caller frees.
///
/// `GUATIAO_ERR_NULL` for a null `host` or `out`, `GUATIAO_ERR_BAD_VALUE`
/// for text that is not UTF-8, `GUATIAO_ERR_NOT_FOUND` when the host holds
/// no such provider, `GUATIAO_ERR_WRONG_KIND` when its greeter table is not
/// this kind's, `GUATIAO_ERR_UNSUPPORTED` when it has no instance of its
/// own to call.
///
/// # Safety
///
/// `host` is null or a registry's host block, which lives for the
/// process; `key` and `name` are readable for the call; `out` is null or
/// writable storage for one value that holds nothing the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_caller_run(
    host: *const HostInfo<'static>,
    key: Str,
    name: Str,
    out: *mut Value,
) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    // SAFETY: null or a registry's host block, which outlives the process's
    // use of it, by this function's contract.
    let Some(host) = (unsafe { Host::from_raw(host) }) else {
        return Status::GUATIAO_ERR_NULL;
    };
    let (Ok(key), Ok(name)) = (
        core::str::from_utf8(key.into()),
        core::str::from_utf8(name.into()),
    ) else {
        return Status::GUATIAO_ERR_BAD_VALUE;
    };
    let offer = match host.offer::<dyn Greeter>(key) {
        Ok(Some(Ok(offer))) => offer,
        Ok(Some(Err(_))) => return Status::GUATIAO_ERR_WRONG_KIND,
        Ok(None) => return Status::GUATIAO_ERR_NOT_FOUND,
        Err(status) => return status,
    };
    let Some(greeter) = offer.default_instance() else {
        return Status::GUATIAO_ERR_UNSUPPORTED;
    };
    let report = Value::from(run(greeter, name));
    // SAFETY: `out` is writable and holds nothing the caller owns; a null
    // allocator writes the tree as it was built.
    unsafe { report.write_to(out, core::ptr::null()) }
}

/// Calls the `wide` provider filed under `key` through the host's
/// services, with the integers this crate's documentation lists, and
/// writes the report described there to `out`, which the caller frees.
///
/// The statuses are `greeter_caller_run`'s, for a `wide` table.
///
/// # Safety
///
/// As for `greeter_caller_run`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_caller_wide(
    host: *const HostInfo<'static>,
    key: Str,
    out: *mut Value,
) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    // SAFETY: null or a registry's host block, which outlives the process's
    // use of it, by this function's contract.
    let Some(host) = (unsafe { Host::from_raw(host) }) else {
        return Status::GUATIAO_ERR_NULL;
    };
    let Ok(key) = core::str::from_utf8(key.into()) else {
        return Status::GUATIAO_ERR_BAD_VALUE;
    };
    let offer = match host.offer::<dyn Wide>(key) {
        Ok(Some(Ok(offer))) => offer,
        Ok(Some(Err(_))) => return Status::GUATIAO_ERR_WRONG_KIND,
        Ok(None) => return Status::GUATIAO_ERR_NOT_FOUND,
        Err(status) => return status,
    };
    let Some(wide) = offer.default_instance() else {
        return Status::GUATIAO_ERR_UNSUPPORTED;
    };
    let report = Value::from(run_wide(wide));
    // SAFETY: `out` is writable and holds nothing the caller owns; a null
    // allocator writes the tree as it was built.
    unsafe { report.write_to(out, core::ptr::null()) }
}
