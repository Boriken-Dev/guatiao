// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Providers written in Rust, compiled to WebAssembly, served by the
//! `guatiao-wasm` source.
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
            // SAFETY: the host passes back what `guatiao_guest_alloc`
            // answered for `len`, once, as guatiao ABI 1 requires.
            unsafe { $crate::glue::call($make, object, target, request, len) }
        }

        /// guatiao ABI 1: an object this module answered was destroyed.
        #[unsafe(no_mangle)]
        pub extern "C" fn guatiao_guest_release(object: i64) {
            $crate::glue::release($make, object)
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

    fn library() -> Library {
        Library::new().provider(
            Provider::new("counter", Counter { count: 0 })
                .name("A counter")
                .serves("counter", ["count"]),
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
        assert_eq!(providers.len(), 1);

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
