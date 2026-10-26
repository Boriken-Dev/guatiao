// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A derived configuration, a list of floats and a byte string, to wire
//! bytes and back: through a tree (`to_value`, then encode; decode, then
//! `from_value`) and streamed (`to_bytes_of`, `decode_as`). The
//! configuration and the floats to JSON and back the same two ways.

use core::hint::black_box;

use guatiao::value::visit::{Flat, Sink};
use guatiao::value::wire;
use guatiao::{Alloc, Bytes, FromValue, ToValue, Value, ValueError, Walk};
use guatiao_serde::Presentation;
use guatiao_serde::text::json;

use crate::{Libraries, Run, batch};

#[derive(Debug, Clone, PartialEq, ToValue, FromValue)]
pub(crate) struct Server {
    host: String,
    port: u16,
    weight: f64,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, ToValue, FromValue)]
pub(crate) struct Limits {
    connections: u32,
    timeout: f64,
}

/// A configuration a host reads: scalars, a list of records and an
/// optional nested one.
#[derive(Debug, Clone, PartialEq, ToValue, FromValue)]
pub(crate) struct Config {
    name: String,
    enabled: bool,
    ratio: f64,
    servers: Vec<Server>,
    limits: Option<Limits>,
}

/// Twenty servers: about 1.3 KB of wire bytes.
pub(crate) fn config() -> Config {
    Config {
        name: "front door".into(),
        enabled: true,
        ratio: 0.75,
        servers: (0..20u16)
            .map(|i| Server {
                host: format!("10.0.0.{i}"),
                port: 5900 + i,
                weight: 1.5 + f64::from(i) / 8.0,
                tags: vec!["a".into(), format!("rack-{i}")],
            })
            .collect(),
        limits: Some(Limits {
            connections: 1000,
            timeout: 2.5,
        }),
    }
}

/// Ten thousand floats with fractions.
fn floats() -> Vec<f64> {
    (0..10_000).map(|i| f64::from(i) * 0.37 + 0.001).collect()
}

/// 64 KiB.
fn blob() -> Bytes {
    Bytes((0..65_536u32).map(|i| i as u8).collect())
}

fn tree_to_wire<T: ToValue + 'static>(value: T) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            let tree = black_box(&value).to_value().expect("built");
            black_box(wire::to_bytes(&tree).expect("encoded"));
        })
    })
}

fn walk_to_wire<T: guatiao::Walk + 'static>(value: T) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            black_box(wire::to_bytes_of(black_box(&value)).expect("encoded"));
        })
    })
}

fn wire_to_tree<T: ToValue + FromValue + 'static>(value: T) -> Run {
    let bytes = wire::to_bytes(&value.to_value().expect("built")).expect("encoded");
    Box::new(move |iters| {
        batch(iters, || {
            let tree = wire::decode(black_box(&bytes)).expect("decoded");
            black_box(T::from_value(&tree).expect("read"));
        })
    })
}

fn wire_to_walk<T: ToValue + guatiao::FromWalk + 'static>(value: T) -> Run {
    let bytes = wire::to_bytes(&value.to_value().expect("built")).expect("encoded");
    Box::new(move |iters| {
        batch(iters, || {
            black_box(wire::decode_as::<T>(black_box(&bytes)).expect("read"));
        })
    })
}

pub(crate) fn config_wire_tree(_: &Libraries) -> Run {
    tree_to_wire(config())
}

pub(crate) fn config_wire_walk(_: &Libraries) -> Run {
    walk_to_wire(config())
}

pub(crate) fn wire_config_tree(_: &Libraries) -> Run {
    wire_to_tree(config())
}

pub(crate) fn wire_config_walk(_: &Libraries) -> Run {
    wire_to_walk(config())
}

pub(crate) fn floats_wire_tree(_: &Libraries) -> Run {
    tree_to_wire(floats())
}

pub(crate) fn floats_wire_walk(_: &Libraries) -> Run {
    walk_to_wire(floats())
}

pub(crate) fn wire_floats_tree(_: &Libraries) -> Run {
    wire_to_tree(floats())
}

pub(crate) fn wire_floats_walk(_: &Libraries) -> Run {
    wire_to_walk(floats())
}

pub(crate) fn bytes_wire_tree(_: &Libraries) -> Run {
    tree_to_wire(blob())
}

pub(crate) fn bytes_wire_walk(_: &Libraries) -> Run {
    walk_to_wire(blob())
}

pub(crate) fn wire_bytes_tree(_: &Libraries) -> Run {
    wire_to_tree(blob())
}

pub(crate) fn wire_bytes_walk(_: &Libraries) -> Run {
    wire_to_walk(blob())
}

fn tree_to_json<T: ToValue + 'static>(value: T) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            let tree = black_box(&value).to_value().expect("built");
            black_box(json::to_string(&tree, Presentation::new()).expect("written"));
        })
    })
}

fn walk_to_json<T: guatiao::Walk + 'static>(value: T) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            black_box(json::to_string_of(black_box(&value), Presentation::new()).expect("written"));
        })
    })
}

fn json_to_tree<T: ToValue + FromValue + 'static>(value: T) -> Run {
    let text =
        json::to_string(&value.to_value().expect("built"), Presentation::new()).expect("written");
    Box::new(move |iters| {
        batch(iters, || {
            let tree = json::from_str(black_box(&text), Alloc::rust(), Presentation::new())
                .expect("parsed");
            black_box(T::from_value(&tree).expect("read"));
        })
    })
}

fn json_to_walk<T: ToValue + guatiao::FromWalk + 'static>(value: T) -> Run {
    let text =
        json::to_string(&value.to_value().expect("built"), Presentation::new()).expect("written");
    Box::new(move |iters| {
        batch(iters, || {
            black_box(json::from_str_as::<T>(black_box(&text), Presentation::new()).expect("read"));
        })
    })
}

pub(crate) fn config_json_tree(_: &Libraries) -> Run {
    tree_to_json(config())
}

pub(crate) fn config_json_walk(_: &Libraries) -> Run {
    walk_to_json(config())
}

pub(crate) fn json_config_tree(_: &Libraries) -> Run {
    json_to_tree(config())
}

pub(crate) fn json_config_walk(_: &Libraries) -> Run {
    json_to_walk(config())
}

pub(crate) fn floats_json_tree(_: &Libraries) -> Run {
    tree_to_json(floats())
}

pub(crate) fn floats_json_walk(_: &Libraries) -> Run {
    walk_to_json(floats())
}

pub(crate) fn json_floats_tree(_: &Libraries) -> Run {
    json_to_tree(floats())
}

pub(crate) fn json_floats_walk(_: &Libraries) -> Run {
    json_to_walk(floats())
}

// --- written from a tree or from its wire bytes ---------------------------------

/// Two hundred keys in one map, each holding text, a number or a bool.
fn keys() -> Value {
    let entries: Vec<String> = (0..200)
        .map(|i| match i % 3 {
            0 => format!("\"setting-{i}\": \"value {i}\""),
            1 => format!("\"setting-{i}\": {i}.5"),
            _ => format!("\"setting-{i}\": {}", i % 2 == 0),
        })
        .collect();
    let text = format!("{{{}}}", entries.join(", "));
    json::from_str(&text, Alloc::rust(), Presentation::new()).expect("parsed")
}

/// Takes every event and keeps nothing: what a walk costs on its own.
struct Discard;

impl Sink for Discard {
    type Error = ValueError;

    fn absent(&mut self) -> Result<(), ValueError> {
        Ok(())
    }
    fn null(&mut self) -> Result<(), ValueError> {
        Ok(())
    }
    fn bool(&mut self, value: bool) -> Result<(), ValueError> {
        black_box(value);
        Ok(())
    }
    fn number(&mut self, text: &str) -> Result<(), ValueError> {
        black_box(text);
        Ok(())
    }
    fn string(&mut self, text: &str) -> Result<(), ValueError> {
        black_box(text);
        Ok(())
    }
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ValueError> {
        black_box(bytes);
        Ok(())
    }
    fn list_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        black_box(len);
        Ok(())
    }
    fn list_end(&mut self) -> Result<(), ValueError> {
        Ok(())
    }
    fn map_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        black_box(len);
        Ok(())
    }
    fn key(&mut self, key: &str) -> Result<(), ValueError> {
        black_box(key);
        Ok(())
    }
    fn map_end(&mut self) -> Result<(), ValueError> {
        Ok(())
    }
}

fn tree_json(tree: Value) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            black_box(json::to_string_of(black_box(&tree), Presentation::new()).expect("written"));
        })
    })
}

fn bytes_json(tree: Value) -> Run {
    let bytes = wire::to_bytes(&tree).expect("encoded");
    Box::new(move |iters| {
        batch(iters, || {
            let walk = wire::Encoded(black_box(&bytes));
            black_box(json::to_string_of(&walk, Presentation::new()).expect("written"));
        })
    })
}

fn tree_events(tree: Value) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            black_box(&tree).walk(Flat(&mut Discard)).expect("walked");
        })
    })
}

fn bytes_events(tree: Value) -> Run {
    let bytes = wire::to_bytes(&tree).expect("encoded");
    Box::new(move |iters| {
        batch(iters, || {
            wire::Encoded(black_box(&bytes))
                .walk(Flat(&mut Discard))
                .expect("walked");
        })
    })
}

fn config_tree() -> Value {
    config().to_value().expect("built")
}

pub(crate) fn tree_json_config(_: &Libraries) -> Run {
    tree_json(config_tree())
}

pub(crate) fn bytes_json_config(_: &Libraries) -> Run {
    bytes_json(config_tree())
}

pub(crate) fn tree_json_keys(_: &Libraries) -> Run {
    tree_json(keys())
}

pub(crate) fn bytes_json_keys(_: &Libraries) -> Run {
    bytes_json(keys())
}

pub(crate) fn tree_events_config(_: &Libraries) -> Run {
    tree_events(config_tree())
}

pub(crate) fn bytes_events_config(_: &Libraries) -> Run {
    bytes_events(config_tree())
}

pub(crate) fn tree_events_keys(_: &Libraries) -> Run {
    tree_events(keys())
}

pub(crate) fn bytes_events_keys(_: &Libraries) -> Run {
    bytes_events(keys())
}

/// A tree to JSON through its own serde impl: what `guatiao_json_emit`
/// runs, where `guatiao_json_from_wire` runs `bytes_json`.
fn value_json(tree: Value) -> Run {
    Box::new(move |iters| {
        batch(iters, || {
            black_box(json::to_string(black_box(&tree), Presentation::new()).expect("written"));
        })
    })
}

/// One map holding a 1 MiB string.
fn blob_tree() -> Value {
    let text = format!("{{\"blob\": \"{}\"}}", "x".repeat(1 << 20));
    json::from_str(&text, Alloc::rust(), Presentation::new()).expect("parsed")
}

pub(crate) fn value_json_config(_: &Libraries) -> Run {
    value_json(config_tree())
}

pub(crate) fn value_json_keys(_: &Libraries) -> Run {
    value_json(keys())
}

pub(crate) fn value_json_blob(_: &Libraries) -> Run {
    value_json(blob_tree())
}

pub(crate) fn tree_json_blob(_: &Libraries) -> Run {
    tree_json(blob_tree())
}

pub(crate) fn bytes_json_blob(_: &Libraries) -> Run {
    bytes_json(blob_tree())
}
