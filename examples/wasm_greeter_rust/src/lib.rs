// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Greeters written in Rust and compiled to WebAssembly: the same kinds the
//! Rust, C, Python and Lua greeters serve, offered to any host through the
//! guatiao-source-wasm source, sandboxed.
//!
//! The module is the library: its id is the manifest's `module`, its
//! version the crate's (a `guatiao.version` section), its providers those
//! [`library`] builds. `wasm_greeter_rust.guatiao-wasm.toml` beside this
//! crate is what the source reads before running anything.

use guatiao_wasm_guest::guatiao::value::convert::TryAsRef;
use guatiao_wasm_guest::{
    Answer, Call, Handler, HostObject, Library, Map, Provider, Refusal, Status, Value,
};

/// The word a greeting starts with; the tests build the module again with
/// another to show a refresh serving new code.
const GREETING: &str = match option_env!("WASM_GREETER_WORD") {
    Some(word) => word,
    None => "hello",
};

fn greeting(name: &str) -> Result<String, Refusal> {
    if name.is_empty() {
        return Err("nobody to greet".into());
    }
    Ok(format!("{GREETING}, {name}"))
}

fn as_answer(text: &str) -> Answer {
    let mut answer = Map::new();
    answer.set("greeting", text);
    answer.into()
}

fn unknown(call: &Call) -> Refusal {
    Refusal::unsupported(format!("no `{}` for `{}`", call.method(), call.kind()))
}

/// One conversation `start` hands back: the listener it was handed, and
/// what was said.
struct Chat {
    listener: HostObject,
    said: Vec<String>,
}

impl Handler for Chat {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        match call.method() {
            "say" => {
                let what = call.text(0)?.to_string();
                if what.is_empty() {
                    return Err("nothing to say".into());
                }
                self.listener.call("heard", [Value::from(what.as_str())])?;
                self.said.push(what);
                Ok(Answer::Nothing)
            }
            "read" => {
                let text = self.said.join(" ");
                let dst = call.buffer(0)?;
                let n = text.len().min(dst.len());
                dst[..n].copy_from_slice(&text.as_bytes()[..n]);
                Ok((n as i64).into())
            }
            "turns" => Ok((self.said.len() as i64).into()),
            _ => Err(unknown(call)),
        }
    }
}

/// One provider serving two kinds, keeping its count in itself.
#[derive(Default)]
struct Hello {
    asked: i64,
}

impl Handler for Hello {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        match (call.kind(), call.method()) {
            ("greeter", "greet") => {
                let name = call.text(0)?;
                if name == "panic" {
                    panic!("a provider bug, on request");
                }
                Ok(as_answer(&greeting(name)?))
            }
            ("greeter", "check") => {
                let config = call.lent_map(0)?;
                if config.get("name").is_none() {
                    config.set("name", "ana");
                }
                config.set("checked_by", "Rust, in wasm");
                Ok(true.into())
            }
            ("greeter", "greet_from") => {
                let config = call.take_map(0)?;
                let name = config
                    .get("name")
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("");
                Ok(as_answer(&greeting(name)?))
            }
            ("greeter", "greet_into") => {
                let text = greeting(call.text(0)?)?;
                let listener = call.lent_object(1)?;
                listener.call("heard", [Value::from(text.as_str())])?;
                Ok(Answer::Nothing)
            }
            ("greeter", "start") => Ok(Answer::object(Chat {
                listener: call.object(0)?,
                said: Vec::new(),
            })),
            ("counter", "count") => {
                self.asked += 1;
                Ok(self.asked.into())
            }
            _ => Err(unknown(call)),
        }
    }
}

/// A greeter answering `greet` alone: `shout` and the rest run the kind's
/// defaults at the caller.
struct Polite;

impl Handler for Polite {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        match call.method() {
            "greet" => {
                let name = call.text(0)?;
                if name.is_empty() {
                    return Err(Refusal::new(
                        Status::GUATIAO_ERR_BAD_VALUE,
                        "nobody to greet, politely",
                    ));
                }
                Ok(as_answer(&format!("good day, {name}")))
            }
            _ => Err(unknown(call)),
        }
    }
}

/// 64-bit integers, both ways, at their limits.
struct Wide;

impl Handler for Wide {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        match call.method() {
            "echo_i64" => Ok(call.int(0)?.into()),
            "echo_u64" => Ok(call.uint(0)?.into()),
            _ => Err(unknown(call)),
        }
    }
}

/// The library: its providers, in the order the host addresses them.
fn library() -> Library {
    Library::new()
        .provider(
            Provider::new("hello", Hello::default())
                .name("Hello, in Rust (wasm)")
                .serves(
                    "greeter",
                    ["greet", "check", "greet_from", "greet_into", "start"],
                )
                .serves("counter", ["count"]),
        )
        .provider(
            Provider::new("polite", Polite)
                .name("Polite, in Rust (wasm)")
                .serves("greeter", ["greet"]),
        )
        .provider(
            Provider::new("wide", Wide)
                .name("Wide, in Rust (wasm)")
                .serves("wide", ["echo_i64", "echo_u64"]),
        )
}

guatiao_wasm_guest::export!(library, version = env!("CARGO_PKG_VERSION"));
