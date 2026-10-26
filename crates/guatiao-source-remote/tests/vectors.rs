// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The protocol against `tests/vectors/remote.json`, written by an
//! encoder of its own: every frame decodes to its fields and encodes to
//! its bytes; arguments and answers likewise, typed by greeter_kind's
//! slots; malformed bodies are refused where the file says.

extern crate alloc;

use alloc::sync::Arc;
use std::path::Path;

use guatiao::library::kind::{Answer, Arg, KindDescription, ObjectRaw, RawObject};
use guatiao::{Alloc, List, Map, Value};
use guatiao_source_remote::protocol::args::{ObjectRef, encode_writebacks};
use guatiao_source_remote::protocol::{
    Decoded, Frame, ProtocolError, Target, Trust, decode_answer, decode_args, encode_answer,
    encode_args,
};
use serde_json::Value as Json;

fn doc() -> Json {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/remote.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the vectors")).expect("JSON")
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A tree as wire.json writes it.
fn tree(t: &Json) -> Value {
    let a = t.as_array().expect("a tree");
    match a[0].as_str().unwrap() {
        "absent" => Value::absent(),
        "null" => Value::null(),
        "bool" => Value::from(a[1].as_bool().unwrap()),
        "number" => Value::from(guatiao::Number::new(a[1].as_str().unwrap()).unwrap()),
        "string" => Value::from(a[1].as_str().unwrap()),
        "list" => {
            let mut l = List::new();
            for x in a[1].as_array().unwrap() {
                l.push(tree(x));
            }
            Value::from(l)
        }
        "map" => {
            let mut m = Map::new();
            for kv in a[1].as_array().unwrap() {
                m.set(kv[0].as_str().unwrap(), tree(&kv[1]));
            }
            Value::from(m)
        }
        other => panic!("{other}"),
    }
}

fn map_of(t: &Json) -> Map {
    Map::try_from(tree(t)).expect("a map")
}

fn nonce(s: &str) -> [u8; 16] {
    unhex(s).try_into().expect("16 bytes")
}

/// The frame a vector's fields name; byte fields are borrowed from
/// `keep`, which the caller holds.
fn frame_of<'a>(f: &Json, keep: &'a [Vec<u8>]) -> Frame<'a> {
    let n = |k: &str| f[k].as_u64().unwrap();
    match f["type"].as_str().unwrap() {
        "hello" => Frame::Hello {
            trust: match f["trust"].as_str().unwrap() {
                "local" => Trust::Local,
                "trusted" => Trust::Trusted,
                "secure" => Trust::Secure,
                "server-tls" => Trust::ServerTls,
                other => panic!("a trust level `{other}`"),
            },
            nonce: nonce(f["nonce"].as_str().unwrap()),
            options: tree(&f["options"]),
        },
        "welcome" => Frame::Welcome {
            nonce: nonce(f["nonce"].as_str().unwrap()),
            proof: &keep[0],
            options: tree(&f["options"]),
        },
        "proof" => Frame::Proof { proof: &keep[0] },
        "list" => Frame::List { call: n("call") },
        "open" => Frame::Open {
            call: n("call"),
            library: leak(f["library"].as_str().unwrap()),
            stamp: leak(f["stamp"].as_str().unwrap()),
        },
        "close" => Frame::Close {
            library: leak(f["library"].as_str().unwrap()),
            stamp: leak(f["stamp"].as_str().unwrap()),
        },
        "call" => {
            let t = f["target"].as_array().unwrap();
            let id = t[1].as_u64().unwrap();
            Frame::Call {
                call: n("call"),
                parent: n("parent"),
                target: if t[0] == "table" {
                    Target::Table(id)
                } else {
                    Target::Object(id)
                },
                slot: n("slot"),
                args: &keep[0],
            }
        }
        "reply" => Frame::Reply {
            call: n("call"),
            status: n("status") as u32,
            body: &keep[0],
        },
        "cancel" => Frame::Cancel { call: n("call") },
        "release" => Frame::Release {
            object: n("object"),
        },
        "ping" => Frame::Ping { n: n("n") },
        "pong" => Frame::Pong { n: n("n") },
        "goodbye" => Frame::Goodbye {
            status: n("status") as u32,
            message: leak(f["message"].as_str().unwrap()),
        },
        other => panic!("{other}"),
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn bytes_field(f: &Json) -> Vec<Vec<u8>> {
    for k in ["proof", "args", "body"] {
        if let Some(h) = f.get(k).and_then(Json::as_str) {
            return vec![unhex(h)];
        }
    }
    Vec::new()
}

#[test]
fn every_frame_decodes_to_its_fields_and_encodes_to_its_bytes() {
    let doc = doc();
    for v in doc["frames"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let bytes = unhex(v["hex"].as_str().unwrap());
        let keep = bytes_field(&v["frame"]);
        let expected = frame_of(&v["frame"], &keep);
        let mut out = Vec::new();
        expected.encode(&mut out, usize::MAX).expect(name);
        assert_eq!(out, bytes, "{name}: encodes");
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len() - 4, "{name}: length");
        let decoded = Frame::decode(&bytes[4..], Alloc::rust()).expect(name);
        assert_eq!(decoded, expected, "{name}: decodes");
    }
}

#[test]
fn every_malformed_body_is_refused_where_the_file_says() {
    let doc = doc();
    for v in doc["malformed"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let body = unhex(v["body"].as_str().unwrap());
        let e = Frame::decode(&body, Alloc::rust()).expect_err(name);
        let at = v.get("at").and_then(Json::as_u64).map(|a| a as usize);
        let ok = match (v["error"].as_str().unwrap(), &e) {
            ("unknown_frame", ProtocolError::UnknownFrame { .. }) => true,
            ("not_this_protocol", ProtocolError::NotThisProtocol) => true,
            ("version", ProtocolError::Version { theirs: 2 }) => true,
            ("overlong", ProtocolError::Overlong { at: a }) => Some(*a) == at,
            ("truncated", ProtocolError::Truncated { at: a }) => Some(*a) == at,
            ("trailing", ProtocolError::Trailing { at: a }) => Some(*a) == at,
            ("not_utf8", ProtocolError::NotUtf8 { at: a }) => Some(*a) == at,
            ("field", ProtocolError::Field { at: a, .. }) => Some(*a) == at,
            _ => false,
        };
        assert!(ok, "{name}: refused as {e:?}");
    }
}

fn kinds() -> Vec<Arc<KindDescription>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let mut d = guatiao_source_remote::Described::new();
    d.load(&path).unwrap();
    ["greeter", "conversation", "wide", "listener", "counter"]
        .iter()
        .map(|k| Arc::clone(d.kinds().get(k).unwrap()))
        .collect()
}

fn slot<'a>(
    kinds: &'a [Arc<KindDescription>],
    v: &Json,
) -> &'a guatiao::library::kind::SlotDescription {
    let kind = kinds
        .iter()
        .find(|k| k.name() == v["kind"].as_str().unwrap())
        .unwrap();
    kind.slot(v["slot"].as_str().unwrap()).unwrap().1
}

fn object_ref(a: &[Json]) -> ObjectRef {
    let id = a[2].as_u64().unwrap();
    if a[1] == "senders" {
        ObjectRef::Senders(id)
    } else {
        ObjectRef::Receivers(id)
    }
}

#[test]
fn every_argument_list_encodes_to_its_bytes_and_decodes_to_its_values() {
    let kinds = kinds();
    let doc = doc();
    for v in doc["args"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let desc = slot(&kinds, v);
        let bytes = unhex(v["hex"].as_str().unwrap());
        let given = v["args"].as_array().unwrap();
        let mut maps: Vec<Map> = Vec::new();
        let mut buffers: Vec<Vec<u8>> = Vec::new();
        for a in given {
            let a = a.as_array().unwrap();
            match a[0].as_str().unwrap() {
                "map" => maps.push(map_of(&a[1])),
                "buffer" => buffers.push(vec![0; a[1].as_u64().unwrap() as usize]),
                _ => {}
            }
        }
        let (mut maps_at, mut buffers_at) = (maps.iter_mut(), buffers.iter_mut());
        let mut refs = Vec::new();
        let mut args: Vec<Arg<'_>> = Vec::new();
        for (a, d) in given.iter().zip(desc.args()) {
            let a = a.as_array().unwrap();
            let lent = format!("{:?}", d.ty()).contains("Lent");
            refs.push(None);
            args.push(match a[0].as_str().unwrap() {
                "str" => Arg::Str(a[1].as_str().unwrap()),
                "int" => Arg::Int(a[1].as_i64().unwrap()),
                "uint" => Arg::Uint(a[1].as_u64().unwrap()),
                "buffer" => Arg::Buffer(buffers_at.next().unwrap()),
                "map" if lent => Arg::LentMap(maps_at.next().unwrap()),
                "map" => Arg::TakenMap(maps_at.next().unwrap().clone()),
                "object" => {
                    *refs.last_mut().unwrap() = Some(object_ref(a));
                    Arg::Bool(false)
                }
                other => panic!("{other}"),
            });
        }
        let mut out = Vec::new();
        encode_args(desc, &args, &refs, &mut out).expect(name);
        assert_eq!(out, bytes, "{name}: encodes");

        let decoded = decode_args(desc, &bytes, Alloc::rust()).expect(name);
        assert_eq!(decoded.len(), given.len(), "{name}");
        for (d, a) in decoded.iter().zip(given) {
            let a = a.as_array().unwrap();
            let same = match (d, a[0].as_str().unwrap()) {
                (Decoded::Str(s), "str") => *s == a[1].as_str().unwrap(),
                (Decoded::Int(n), "int") => Some(*n) == a[1].as_i64(),
                (Decoded::Uint(n), "uint") => Some(*n) == a[1].as_u64(),
                (Decoded::Buffer(b), "buffer") => Some(b.len() as u64) == a[1].as_u64(),
                (Decoded::Map(m), "map") => *m == map_of(&a[1]),
                (Decoded::Object(r), "object") => *r == object_ref(a),
                _ => false,
            };
            assert!(same, "{name}: decoded {d:?}");
        }
    }
}

#[test]
fn every_answer_encodes_to_its_bytes_and_decodes_with_its_writebacks() {
    let kinds = kinds();
    let doc = doc();
    for v in doc["answers"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let desc = slot(&kinds, v);
        let bytes = unhex(v["hex"].as_str().unwrap());
        let a = v["answer"].as_array().unwrap();
        let answer = match a[0].as_str().unwrap() {
            "map" => Answer::Map(map_of(&a[1])),
            "text" => Answer::Text(a[1].as_str().unwrap().to_string()),
            "bool" => Answer::Bool(a[1].as_bool().unwrap()),
            "int" => Answer::Int(a[1].as_i64().unwrap()),
            "uint" => Answer::Uint(a[1].as_u64().unwrap()),
            // SAFETY: a null table: dropping it destroys nothing.
            "object" => Answer::Object(unsafe { RawObject::from_raw(ObjectRaw::null()) }),
            "nothing" => Answer::Nothing,
            other => panic!("{other}"),
        };
        let mut out = Vec::new();
        let wanted = if a[0] == "object" {
            Some(object_ref(a))
        } else {
            None
        };
        encode_answer(desc, answer, &mut out, &mut |_| wanted.unwrap()).expect(name);
        // The write-backs follow, in the order of the slot's arguments.
        let wb = v["writebacks"].as_array().unwrap();
        let mut decoded: Vec<Decoded<'_>> = Vec::new();
        for d in desc.args() {
            let lent_map = format!("{:?}", d.ty()).contains("Lent");
            let buffer = format!("{:?}", d.ty()) == "BytesMut";
            decoded.push(if lent_map {
                let w = wb.iter().find(|w| w[0] == "map").unwrap();
                Decoded::Map(map_of(&w[1]))
            } else if buffer {
                let w = wb.iter().find(|w| w[0] == "buffer").unwrap();
                Decoded::Buffer(unhex(w[1].as_str().unwrap()))
            } else {
                Decoded::Bool(false)
            });
        }
        encode_writebacks(desc, &decoded, &mut out).expect(name);
        assert_eq!(out, bytes, "{name}: encodes");

        // And back: the answer, then each write-back into the caller's own.
        let mut lent = Map::new();
        let mut buffer = vec![0u8; 16];
        let mut args: Vec<Arg<'_>> = Vec::new();
        let mut lent_slot = Some(&mut lent);
        let mut buffer_slot = Some(&mut buffer);
        for d in desc.args() {
            let ty = format!("{:?}", d.ty());
            args.push(if ty.contains("Lent") && ty.contains("Map") {
                Arg::LentMap(lent_slot.take().unwrap())
            } else if ty == "BytesMut" {
                Arg::Buffer(buffer_slot.take().unwrap())
            } else {
                Arg::Bool(false)
            });
        }
        let mut objects = Vec::new();
        let back = decode_answer(desc, &bytes, &mut args, Alloc::rust(), &mut |kind, r| {
            objects.push((kind.to_string(), r));
            // SAFETY: as above.
            Ok(unsafe { RawObject::from_raw(ObjectRaw::null()) })
        })
        .expect(name);
        drop(args);
        let same = match (&back, a[0].as_str().unwrap()) {
            (Answer::Map(m), "map") => *m == map_of(&a[1]),
            (Answer::Text(t), "text") => t == a[1].as_str().unwrap(),
            (Answer::Bool(b), "bool") => Some(*b) == a[1].as_bool(),
            (Answer::Int(n), "int") => Some(*n) == a[1].as_i64(),
            (Answer::Uint(n), "uint") => Some(*n) == a[1].as_u64(),
            (Answer::Object(_), "object") => {
                objects == [("conversation".to_string(), object_ref(a))]
            }
            (Answer::Nothing, "nothing") => true,
            _ => false,
        };
        assert!(same, "{name}: decoded {back:?}");
        for w in wb {
            match w[0].as_str().unwrap() {
                "map" => assert_eq!(lent, map_of(&w[1]), "{name}: the lent map"),
                "buffer" => {
                    let b = unhex(w[1].as_str().unwrap());
                    assert_eq!(&buffer[..b.len()], &b[..], "{name}: the buffer");
                }
                _ => {}
            }
        }
    }
}
