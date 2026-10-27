// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a call costs on the wire, against the same call sent as one
//! self-describing wire value with its names spelled out: the saving the
//! handshake's interned ids and the typed arguments buy.

extern crate alloc;

use alloc::sync::Arc;

use guatiao::library::kind::{Answer, Arg, KindDescription};
use guatiao::value::wire;
use guatiao::{List, Map, Value};
use guatiao_source_remote::protocol::{self, Target, encode_answer, encode_args};

fn greeter() -> Arc<KindDescription> {
    let mut d = guatiao_source_remote::Kinds::new();
    d.add_json(&greeter_kind::GreeterVtable::description())
        .unwrap();
    Arc::clone(d.get("greeter").unwrap())
}

/// A frame: four bytes of length, then the wire value.
fn plain(value: Value) -> usize {
    4 + wire::to_bytes(&value).unwrap().len()
}

fn named_call(args: List) -> Value {
    let mut call = Map::new();
    call.set("call", 3i64);
    call.set("provider", "derived_greeter_hello");
    call.set("kind", "greeter");
    call.set("method", "greet");
    call.set("args", args);
    Value::from(call)
}

#[test]
fn a_greet_call_and_its_answer_cost_a_fraction_of_a_named_one() {
    let kind = greeter();
    let (slot, desc) = kind.slot("greet").unwrap();

    let mut frame = Vec::new();
    let at = protocol::start_call(&mut frame, 3, 0, Target::Table(1), slot as u64);
    encode_args(desc, &[Arg::Str("world")], &[None], &mut frame).unwrap();
    protocol::finish(&mut frame, at, usize::MAX).unwrap();

    let mut answer = Map::new();
    answer.set("greeting", "hello, world");
    let mut reply = Vec::new();
    let at = protocol::start_reply(&mut reply, 3, 0);
    encode_answer(
        desc,
        Answer::Map(answer.clone()),
        &mut reply,
        &mut |_| unreachable!(),
    )
    .unwrap();
    protocol::finish(&mut reply, at, usize::MAX).unwrap();

    let mut args = List::new();
    args.push("world");
    let named = plain(named_call(args));
    let mut named_reply = Map::new();
    named_reply.set("call", 3i64);
    named_reply.set("status", 0i64);
    named_reply.set("answer", answer);
    let named_answer = plain(Value::from(named_reply));

    println!(
        "greet(\"world\"): {} bytes, named {named}; its answer {} bytes, named {named_answer}",
        frame.len(),
        reply.len()
    );
    assert_eq!(frame.len(), 15, "4 length, 1 type, 4 ids, 6 text");
    assert_eq!(reply.len(), 33);
    assert!(frame.len() * 5 < named, "{} against {named}", frame.len());
    assert!(reply.len() < named_answer);
}
