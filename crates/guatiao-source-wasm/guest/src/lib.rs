// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Providers written in Rust, compiled to WebAssembly, served by the
//! `guatiao-source-wasm` source.
//!
//! A library is a `cdylib` built for `wasm32-unknown-unknown`. It builds
//! its [`Library`] in a function and hands it to [`export!`], which writes
//! the exports of guatiao ABI 1; every call reaches a [`Handler`] with the
//! kind and slot named and guatiao's own [`Value`]s as arguments.
//!
//! ```
//! use guatiao_wasm_guest::{Answer, Call, Library, Map, Provider, Refusal};
//!
//! struct Hello;
//!
//! impl guatiao_wasm_guest::Handler for Hello {
//!     fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
//!         match call.method() {
//!             "greet" => {
//!                 let name = call.text(0)?;
//!                 if name.is_empty() {
//!                     return Err("nobody to greet".into());
//!                 }
//!                 let mut answer = Map::new();
//!                 answer.set("greeting", format!("hello, {name}").as_str());
//!                 Ok(answer.into())
//!             }
//!             other => Err(Refusal::unsupported(format!("no `{other}`"))),
//!         }
//!     }
//! }
//!
//! fn library() -> Library {
//!     Library::new().provider(Provider::new("hello", Hello).serves("greeter", ["greet"]))
//! }
//!
//! guatiao_wasm_guest::export!(library, version = "1.0.0");
//! # fn main() {}
//! ```

#![deny(unsafe_code)]

extern crate alloc;

mod call;
#[doc(hidden)]
pub mod glue;
mod host;
mod library;

pub use call::{Answer, Call, Handler, HostObject, Refusal};
/// guatiao itself: its values, their conversions (`guatiao::value::convert`)
/// and their wire encoding.
pub use guatiao;
pub use guatiao::{List, Map, Status, Value};
pub use library::{Library, Provider};

/// Writes the module's exports for the library `make` builds (once, on the
/// first call), and with `version = "..."` the `guatiao.version` section a
/// host reads without running the module.
///
/// The exports are `#[unsafe(no_mangle)]`, so the crate calling it cannot
/// `forbid(unsafe_code)`.
#[macro_export]
macro_rules! export {
    ($make:path, version = $version:expr) => {
        $crate::export!($make);

        #[unsafe(link_section = "guatiao.version")]
        #[used]
        static __GUATIAO_VERSION: [u8; $version.len()] = {
            let text: &str = $version;
            let mut out = [0u8; $version.len()];
            let mut i = 0;
            while i < out.len() {
                out[i] = text.as_bytes()[i];
                i += 1;
            }
            out
        };
    };
    ($make:path) => {
        /// guatiao ABI 1: the version this module speaks.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_abi() -> i32 {
            $crate::glue::ABI
        }

        /// guatiao ABI 1: bytes for the host to write a request into.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_alloc(len: usize) -> usize {
            $crate::glue::alloc(len)
        }

        /// guatiao ABI 1: the declaration, through the host's `answer`.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_declare() -> u32 {
            $crate::glue::declare($make)
        }

        /// guatiao ABI 1: one call.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_call(
            object: i32,
            target: i64,
            request: usize,
            len: usize,
        ) -> u32 {
            $crate::glue::call($make, object, target, request, len)
        }

        /// guatiao ABI 1: an object this module answered was destroyed.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_release(object: i64) {
            $crate::glue::release($make, object)
        }

        /// guatiao ABI 1: an instance of a provider, from a configuration.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_create(provider: i64, config: usize, len: usize) -> u32 {
            $crate::glue::create($make, provider, config, len)
        }

        /// guatiao ABI 1: an instance this module built was dropped.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_destroy(instance: i64) {
            $crate::glue::destroy($make, instance)
        }

        /// guatiao ABI 1: whether a provider can run now.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_available(provider: i64) -> u32 {
            $crate::glue::available($make, provider)
        }
    };
}

// The tests are a guest: `export!` writes `no_mangle` exports.
#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec::Vec;

    use guatiao::value::convert::TryAsRef;
    use guatiao::value::wire;

    use super::*;
    use crate::host::native::TOLD;

    struct Counter {
        count: i64,
    }

    impl Handler for Counter {
        fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
            match call.method() {
                "count" => {
                    self.count += 1;
                    Ok(self.count.into())
                }
                "fill" => {
                    let map = call.lent_map(0)?;
                    map.set("filled", true);
                    let buffer = call.buffer(1)?;
                    buffer[..2].copy_from_slice(b"ok");
                    Ok(().into())
                }
                "start" => {
                    let listener = call.object(0)?;
                    Ok(Answer::object(move |call: &mut Call| {
                        let what = call.text(0)?.to_string();
                        listener.call("heard", [Value::from(what.as_str())])?;
                        Ok(Answer::Nothing)
                    }))
                }
                other => Err(Refusal::unsupported(other)),
            }
        }
    }

    /// An instance: greets with the word it was configured with.
    struct Word(alloc::string::String);

    impl Handler for Word {
        fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
            let name = call.text(0)?;
            Ok(alloc::format!("{}, {name}", self.0).as_str().into())
        }
    }

    fn word(config: Value) -> Result<Word, Refusal> {
        let word = TryAsRef::<Map>::try_as_ref(&config)
            .and_then(|m| m.get("word"))
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or("");
        if word.is_empty() {
            return Err("no word".into());
        }
        Ok(Word(word.to_string()))
    }

    fn library() -> Library {
        let mut schema = Map::new();
        schema.set("type", "object");
        Library::new()
            .provider(
                Provider::new("counter", Counter { count: 0 })
                    .name("A counter")
                    .serves("counter", ["count"]),
            )
            .provider(
                Provider::new("tuned", Counter { count: 0 })
                    .serves("greeter", ["greet"])
                    .config(schema.clone(), word)
                    .available(|| Ok(())),
            )
            .provider(
                Provider::instances("only", schema, word)
                    .serves("greeter", ["greet"])
                    .available(|| Err("asleep".to_string())),
            )
    }

    export!(library, version = "1.2.3");

    fn request(kind: &str, method: &str, args: Vec<Value>) -> (usize, usize) {
        let mut list = List::new();
        list.push(kind);
        list.push(method);
        for a in args {
            list.push(a);
        }
        let bytes = wire::to_bytes(&Value::from(list)).expect("encoded");
        let at = guatiao_guest_alloc(bytes.len());
        // SAFETY: `at` is `bytes.len()` writable bytes, just allocated.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len()) };
        (at, bytes.len())
    }

    fn answers() -> Vec<Vec<u8>> {
        TOLD.with(|t| core::mem::take(&mut t.borrow_mut().answers))
    }

    #[test]
    fn the_exports_declare_dispatch_and_refuse() {
        assert_eq!(guatiao_guest_abi(), 1);
        assert_eq!(__GUATIAO_VERSION, *b"1.2.3");
        assert_eq!(guatiao_guest_declare(), 0);
        let declaration = wire::decode(&answers()[0]).expect("a value");
        let providers: &List = TryAsRef::<Map>::try_as_ref(&declaration)
            .and_then(|m| m.get("providers"))
            .and_then(TryAsRef::try_as_ref)
            .expect("providers");
        assert_eq!(providers.len(), 3);
        let keys = |i: usize| -> Vec<alloc::string::String> {
            let entry: &Map =
                TryAsRef::try_as_ref(providers.get(i).expect("declared")).expect("a map");
            entry.iter().map(|e| e.key().to_string()).collect()
        };
        assert_eq!(keys(0), ["id", "name", "serves"]);
        assert_eq!(
            keys(1),
            ["id", "name", "serves", "config", "may_be_unavailable"]
        );
        assert_eq!(
            keys(2),
            [
                "id",
                "name",
                "serves",
                "config",
                "instances_only",
                "may_be_unavailable"
            ]
        );

        let (at, len) = request("counter", "count", Vec::new());
        assert_eq!(guatiao_guest_call(0, 0, at, len), 0);
        let (at, len) = request("counter", "count", Vec::new());
        assert_eq!(guatiao_guest_call(0, 0, at, len), 0);
        let counted: Vec<Option<i64>> = answers().iter().map(|b| glue::number(b)).collect();
        assert_eq!(counted, [Some(1), Some(2)]);

        let (at, len) = request("counter", "nope", Vec::new());
        assert_eq!(guatiao_guest_call(0, 0, at, len), 13);
        assert_eq!(answers(), [b"nope".to_vec()]);
        let (at, len) = request("counter", "count", Vec::new());
        assert_eq!(guatiao_guest_call(0, 7, at, len), 10, "no provider 7");
        let garbage = guatiao_guest_alloc(3);
        assert_eq!(guatiao_guest_call(0, 0, garbage, 3), 7, "not a request");
        answers();
    }

    fn configuration(word: &str) -> (usize, usize) {
        let mut config = Map::new();
        config.set("word", word);
        let bytes = wire::to_bytes(&Value::from(config)).expect("encoded");
        let at = guatiao_guest_alloc(bytes.len());
        // SAFETY: `at` is `bytes.len()` writable bytes, just allocated.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len()) };
        (at, bytes.len())
    }

    fn text(bytes: &[u8]) -> alloc::string::String {
        let value = wire::decode(bytes).expect("a value");
        TryAsRef::<str>::try_as_ref(&value)
            .expect("text")
            .to_string()
    }

    #[test]
    fn instances_are_built_called_and_destroyed_and_availability_is_answered() {
        let (at, len) = configuration("howdy");
        assert_eq!(guatiao_guest_create(1, at, len), 0);
        let howdy = glue::number(&answers()[0]).expect("an instance id");
        let (at, len) = configuration("hey");
        assert_eq!(guatiao_guest_create(2, at, len), 0);
        let hey = glue::number(&answers()[0]).expect("an instance id");
        assert_ne!(howdy, hey);

        let (at, len) = request("greeter", "greet", alloc::vec![Value::from("ana")]);
        assert_eq!(guatiao_guest_call(2, howdy, at, len), 0);
        let (at, len) = request("greeter", "greet", alloc::vec![Value::from("bo")]);
        assert_eq!(guatiao_guest_call(2, hey, at, len), 0);
        let greeted: Vec<_> = answers().iter().map(|b| text(b)).collect();
        assert_eq!(greeted, ["howdy, ana", "hey, bo"]);

        guatiao_guest_destroy(howdy);
        let (at, len) = request("greeter", "greet", alloc::vec![Value::from("cy")]);
        assert_eq!(guatiao_guest_call(2, howdy, at, len), 12, "destroyed");
        let (at, len) = request("greeter", "greet", alloc::vec![Value::from("cy")]);
        assert_eq!(guatiao_guest_call(2, hey, at, len), 0);
        assert_eq!(text(&answers()[1]), "hey, cy");
        let (at, len) = request("greeter", "greet", alloc::vec![Value::from("di")]);
        assert_eq!(
            guatiao_guest_call(0, 2, at, len),
            13,
            "only through instances"
        );
        answers();

        let (at, len) = configuration("");
        assert_eq!(guatiao_guest_create(1, at, len), 7, "refused");
        assert_eq!(answers(), [b"no word".to_vec()]);
        let (at, len) = configuration("x");
        assert_eq!(guatiao_guest_create(0, at, len), 13, "builds no instances");
        let (at, len) = configuration("x");
        assert_eq!(guatiao_guest_create(9, at, len), 10, "no provider 9");
        assert_eq!(guatiao_guest_create(1, at, len), 7, "taken back already");
        answers();

        assert_eq!(guatiao_guest_available(0), 0);
        assert_eq!(guatiao_guest_available(1), 0);
        assert!(answers().is_empty(), "available: nothing answered");
        assert_eq!(guatiao_guest_available(2), 0);
        assert_eq!(text(&answers()[0]), "asleep");
        assert_eq!(guatiao_guest_available(9), 10);
        answers();
    }

    #[test]
    fn a_request_not_handed_out_is_refused_and_none_is_taken_twice() {
        let (at, len) = request("counter", "count", Vec::new());
        assert_eq!(
            guatiao_guest_call(0, 0, at + 1, len),
            7,
            "not an address alloc answered"
        );
        assert_eq!(
            guatiao_guest_call(0, 0, at, len + 1),
            7,
            "not the length asked for"
        );
        assert_eq!(guatiao_guest_call(0, 0, at, len), 0);
        assert_eq!(guatiao_guest_call(0, 0, at, len), 7, "taken back already");
        let told = answers();
        assert!(
            core::str::from_utf8(&told[0])
                .is_ok_and(|w| w.contains("not one guatiao_guest_alloc handed out")),
            "{told:?}"
        );
    }

    #[test]
    fn lent_arguments_are_written_back_and_objects_kept_until_released() {
        let (at, len) = request(
            "counter",
            "fill",
            alloc::vec![Value::from(Map::new()), Value::from(alloc::vec![0u8; 4])],
        );
        assert_eq!(guatiao_guest_call(0, 0, at, len), 0);
        let written = TOLD.with(|t| core::mem::take(&mut t.borrow_mut().written));
        assert_eq!(written.len(), 2);
        assert_eq!(written[1].0, 1);
        let buffer = wire::decode(&written[1].1).expect("a value");
        assert_eq!(TryAsRef::<[u8]>::try_as_ref(&buffer), Some(&b"ok\0\0"[..]));

        let heard = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new()));
        let kept = alloc::rc::Rc::clone(&heard);
        TOLD.with(|t| {
            t.borrow_mut().object_call = Some(alloc::boxed::Box::new(move |handle, request| {
                kept.borrow_mut().push((handle, request.to_vec()));
                (0, Vec::new())
            }))
        });
        let (at, len) = request("counter", "start", alloc::vec![Value::from(42i64)]);
        assert_eq!(guatiao_guest_call(0, 0, at, len), 0);
        let id = glue::number(&answers()[0]).expect("an object id");
        let (at, len) = request("conversation", "say", alloc::vec![Value::from("hi")]);
        assert_eq!(guatiao_guest_call(1, id, at, len), 0);
        assert_eq!(heard.borrow().len(), 1);
        assert_eq!(heard.borrow()[0].0, 42);
        guatiao_guest_release(id);
        let dropped = TOLD.with(|t| core::mem::take(&mut t.borrow_mut().dropped));
        assert_eq!(dropped, [42], "the listener went with the object");
        let (at, len) = request("conversation", "say", alloc::vec![Value::from("hi")]);
        assert_eq!(guatiao_guest_call(1, id, at, len), 12, "gone");
        answers();
    }
}
