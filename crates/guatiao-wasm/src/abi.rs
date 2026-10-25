// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! **guatiao ABI 1**: what a WebAssembly library exports and imports.
//!
//! A library is a core WebAssembly module (not a component). Every value
//! crosses as its **wire encoding** (`guatiao::value::wire`) in the
//! module's own linear memory; addresses and lengths are `i32` read as
//! unsigned.
//!
//! The module **exports**:
//!
//! | name | signature | does |
//! | --- | --- | --- |
//! | `memory` | memory | the one linear memory, 32-bit |
//! | `guatiao_guest_abi` | `() -> i32` | answers [`VERSION`] |
//! | `guatiao_guest_alloc` | `(len: i32) -> i32` | `len` bytes for the host to write a request into; the guest's from then on |
//! | `guatiao_guest_declare` | `() -> i32` | answers its declaration (below) through `answer`; a status |
//! | `guatiao_guest_call` | `(object: i32, target: i64, request: i32, len: i32) -> i32` | runs a call; a status |
//! | `guatiao_guest_release` | `(object: i64)` | optional: an object it answered was destroyed |
//! | `_initialize` | `()` | optional: called once, first (a WASI reactor's) |
//!
//! It may **import**, from module `guatiao`:
//!
//! | name | signature | does |
//! | --- | --- | --- |
//! | `answer` | `(ptr: i32, len: i32)` | the current call's answer: a wire value when the status is 0, else UTF-8 words; the host copies it |
//! | `write_back` | `(index: i32, ptr: i32, len: i32) -> i32` | a wire value to write back over argument `index`, a lent tree or a buffer (bytes); a status |
//! | `object_call` | `(handle: i64, request: i32, len: i32) -> i32` | calls an object the host passed: the request a wire list `[method, arg...]`; a status, its result kept for `result` |
//! | `result` | `(ptr: i32, cap: i32) -> i32` | copies up to `cap` bytes of the last `object_call`'s result (a wire value, nothing for none, or words); answers its whole length |
//! | `drop` | `(handle: i64)` | lets go of an object the host handed over |
//!
//! and, if it is a WASI preview-1 module, any of `sched_yield`,
//! `proc_exit`, `args_get`, `args_sizes_get`, `environ_get`,
//! `environ_sizes_get`, `clock_time_get`, `fd_write`, `random_get`,
//! `poll_oneoff`, `fd_close`, `fd_fdstat_get`, `fd_fdstat_set_flags`,
//! `fd_prestat_get` and `fd_prestat_dir_name` from
//! `wasi_snapshot_preview1`: no files (no directory is preopened), no
//! sockets, no arguments, no environment; fd 1 is discarded and fd 2 kept
//! to name a failure. Any other import refuses the library.
//!
//! **What the host takes** from a guest at once (an answer, a write-back,
//! a request) is at most [`crate::budget::UNLIMITED`] bytes, or the memory
//! limit when one is set, and decodes to no more than that.
//!
//! **A call.** `object` is 0 for a provider, `target` its position in the
//! declaration; 1 for an object the guest answered, `target` its id. The
//! request is a wire list `[kind, method, arg...]`, one argument per slot
//! argument, in order: a number, bool, text, bytes or tree as itself; a
//! buffer (`bytes_mut`) as bytes of its length; an object as a handle
//! (a number), owned by the guest when taken (`drop` it), valid for the
//! call when lent. A status of 0 answers through `answer` (nothing for a
//! slot that answers nothing; an object kind's answer is the guest's own
//! id for it, a number); any other status is a refusal, its words through
//! `answer`. The statuses are guatiao's (`GUATIAO_ERR_BAD_VALUE` 7, ...);
//! one this crate does not know is `GUATIAO_ERR_INTERNAL`.
//!
//! **The declaration** is a wire map:
//!
//! ```text
//! {"providers": [{"id": "hello", "name": "Hello, in Rust",
//!                 "serves": {"greeter": ["greet", "check"], "counter": ["count"]}}]}
//! ```
//!
//! A provider's id on the host is `<module>_<id>`; a slot it does not
//! name is null in the table, so a caller runs the kind's default. At
//! most [`MOST_PROVIDERS`] providers.
//!
//! **The version** may be stated in a custom section named
//! [`VERSION_SECTION`] (its UTF-8 text), read without compiling the
//! module; a manifest without `version` needs it.

use guatiao::value::convert::TryAsRef;
use guatiao::value::wire;
use guatiao::{List, Map, Status, Value};

/// The ABI this crate speaks: what `guatiao_guest_abi` answers.
pub const VERSION: i32 = 1;

/// The module a guest imports the host's functions from.
pub const IMPORT_MODULE: &str = "guatiao";

/// The custom section a guest may state its version in.
pub const VERSION_SECTION: &str = "guatiao.version";

/// The names a guest exports.
pub mod export {
    /// `() -> i32`: the ABI version.
    pub const ABI: &str = "guatiao_guest_abi";
    /// `(len: i32) -> i32`: bytes for a request.
    pub const ALLOC: &str = "guatiao_guest_alloc";
    /// `() -> i32`: the declaration, through `answer`.
    pub const DECLARE: &str = "guatiao_guest_declare";
    /// `(object: i32, target: i64, request: i32, len: i32) -> i32`.
    pub const CALL: &str = "guatiao_guest_call";
    /// `(object: i64)`, optional.
    pub const RELEASE: &str = "guatiao_guest_release";
    /// `()`, optional, called first.
    pub const INITIALIZE: &str = "_initialize";
    /// The linear memory.
    pub const MEMORY: &str = "memory";
}

/// The names a guest imports from [`IMPORT_MODULE`].
pub mod import {
    /// `(ptr: i32, len: i32)`.
    pub const ANSWER: &str = "answer";
    /// `(index: i32, ptr: i32, len: i32) -> i32`.
    pub const WRITE_BACK: &str = "write_back";
    /// `(handle: i64, request: i32, len: i32) -> i32`.
    pub const OBJECT_CALL: &str = "object_call";
    /// `(ptr: i32, cap: i32) -> i32`.
    pub const RESULT: &str = "result";
    /// `(handle: i64)`.
    pub const DROP: &str = "drop";
}

const LIST_TAG: u8 = 6;

fn put_len(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A request's bytes: the wire list `[kind, method, arg...]`, written
/// without copying the arguments into a list first.
pub(crate) fn request<'a>(
    kind: &str,
    method: &str,
    args: impl ExactSizeIterator<Item = &'a Value>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.push(LIST_TAG);
    put_len(&mut out, 2 + args.len() as u64);
    for text in [kind, method] {
        wire::encode(&Value::from(text), &mut out).map_err(|e| e.to_string())?;
    }
    for arg in args {
        wire::encode(arg, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A request a guest sent [`import::OBJECT_CALL`], decoded: its method
/// and arguments.
pub(crate) fn read_request(value: Value) -> Result<(String, Vec<Value>), String> {
    let list = List::try_from(value)
        .map_err(|_| "the request is not a list `[method, arg...]`".to_string())?;
    let mut items = list.into_iter();
    let method = items
        .next()
        .and_then(|m| TryAsRef::<str>::try_as_ref(&m).map(str::to_string))
        .ok_or_else(|| "the request names no method".to_string())?;
    Ok((method, items.collect()))
}

/// One provider a guest declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    pub id: String,
    pub name: String,
    pub serves: Vec<(String, Vec<String>)>,
}

fn texts(value: &Value, what: &str) -> Result<Vec<String>, String> {
    let list: &List =
        TryAsRef::try_as_ref(value).ok_or_else(|| format!("{what} is not a list of names"))?;
    list.iter()
        .map(|v| {
            TryAsRef::<str>::try_as_ref(v)
                .map(str::to_string)
                .ok_or_else(|| format!("{what} holds something that is not a name"))
        })
        .collect()
}

/// The most providers one library may declare: each costs the host its
/// tables when the library opens.
pub const MOST_PROVIDERS: usize = 4096;

/// The providers a declaration names, in order.
pub(crate) fn read_declaration(bytes: &[u8], budget: u64) -> Result<Vec<Declared>, String> {
    let value = crate::budget::decode(bytes, budget).map_err(|e| match e {
        crate::budget::Undecoded::Refused(refused) => refused.words("the declaration"),
        crate::budget::Undecoded::Malformed(e) => format!("the declaration is not a value: {e}"),
    })?;
    let map: &Map =
        TryAsRef::try_as_ref(&value).ok_or_else(|| "the declaration is not a map".to_string())?;
    let providers: &List = map
        .get("providers")
        .and_then(TryAsRef::try_as_ref)
        .ok_or_else(|| "the declaration has no `providers` list".to_string())?;
    if providers.len() > MOST_PROVIDERS {
        return Err(format!(
            "the declaration names {} providers, more than {MOST_PROVIDERS}",
            providers.len()
        ));
    }
    let mut out: Vec<Declared> = Vec::with_capacity(providers.len());
    for (n, provider) in providers.iter().enumerate() {
        let provider: &Map =
            TryAsRef::try_as_ref(provider).ok_or_else(|| format!("provider {n} is not a map"))?;
        let id = provider
            .get("id")
            .and_then(TryAsRef::<str>::try_as_ref)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("provider {n} has no `id`"))?
            .to_string();
        if out.iter().any(|d| d.id == id) {
            return Err(format!("two providers are declared `{id}`"));
        }
        let name = provider
            .get("name")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or(&id)
            .to_string();
        let serves: &Map = provider
            .get("serves")
            .and_then(TryAsRef::try_as_ref)
            .ok_or_else(|| format!("the provider `{id}` has no `serves` map"))?;
        let mut kinds = Vec::with_capacity(serves.len());
        for entry in serves.iter() {
            let what = format!("`{id}`'s slots of `{}`", entry.key());
            kinds.push((entry.key().to_string(), texts(entry.value(), &what)?));
        }
        out.push(Declared {
            id,
            name,
            serves: kinds,
        });
    }
    Ok(out)
}

/// The status a guest answered, when it is one guatiao defines.
pub(crate) fn status_of(code: i32) -> Option<Status> {
    [
        Status::GUATIAO_ERR_BAD_VALUE,
        Status::GUATIAO_ERR_ALLOC,
        Status::GUATIAO_ERR_WRONG_KIND,
        Status::GUATIAO_ERR_NOT_FOUND,
        Status::GUATIAO_ERR_NULL,
        Status::GUATIAO_ERR_GONE,
        Status::GUATIAO_ERR_UNSUPPORTED,
        Status::GUATIAO_ERR_BUSY,
        Status::GUATIAO_ERR_INTERNAL,
    ]
    .into_iter()
    .find(|s| i64::from(u32::from(*s)) == i64::from(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_the_wire_list_it_says() {
        let args = [Value::from(5i64), Value::from("ana")];
        let bytes = request("greeter", "greet", args.iter()).expect("encoded");
        let value = wire::decode(&bytes).expect("a value");
        let list: &List = TryAsRef::try_as_ref(&value).expect("a list");
        let items: Vec<&Value> = list.iter().collect();
        assert_eq!(items.len(), 4);
        assert_eq!(TryAsRef::<str>::try_as_ref(items[0]), Some("greeter"));
        assert_eq!(TryAsRef::<str>::try_as_ref(items[1]), Some("greet"));
        assert_eq!(items[2], &args[0]);
        assert_eq!(items[3], &args[1]);
        // 200 arguments: a count past one LEB128 byte.
        let many: Vec<Value> = (0..200).map(|i| Value::from(i as i64)).collect();
        let bytes = request("k", "m", many.iter()).expect("encoded");
        let back = List::try_from(wire::decode(&bytes).expect("a value")).expect("a list");
        assert_eq!(back.len(), 202);
    }

    #[test]
    fn a_declaration_is_read_and_its_faults_named() {
        let mut serves = Map::new();
        let mut slots = List::new();
        slots.push("greet");
        serves.set("greeter", slots);
        let mut provider = Map::new();
        provider.set("id", "hello");
        provider.set("serves", serves);
        let mut providers = List::new();
        providers.push(provider.clone());
        let mut declaration = Map::new();
        declaration.set("providers", providers);
        let bytes = wire::to_bytes(&Value::from(declaration.clone())).expect("encoded");
        assert_eq!(
            read_declaration(&bytes, 1 << 20).expect("read"),
            [Declared {
                id: "hello".into(),
                name: "hello".into(),
                serves: vec![("greeter".into(), vec!["greet".into()])],
            }]
        );

        let mut twice = List::new();
        twice.push(provider.clone());
        twice.push(provider);
        declaration.set("providers", twice);
        let bytes = wire::to_bytes(&Value::from(declaration)).expect("encoded");
        assert_eq!(
            read_declaration(&bytes, 1 << 20).unwrap_err(),
            "two providers are declared `hello`"
        );
        assert!(
            read_declaration(&[0xff], 1 << 20)
                .unwrap_err()
                .contains("not a value")
        );
        let empty = wire::to_bytes(&Value::from(Map::new())).expect("encoded");
        assert!(
            read_declaration(&empty, 1 << 20)
                .unwrap_err()
                .contains("`providers`")
        );
    }

    #[test]
    fn only_guatiaos_statuses_are_known() {
        assert_eq!(status_of(7), Some(Status::GUATIAO_ERR_BAD_VALUE));
        assert_eq!(status_of(100), Some(Status::GUATIAO_ERR_INTERNAL));
        assert_eq!(status_of(0), None);
        assert_eq!(status_of(42), None);
        assert_eq!(status_of(-1), None);
    }
}
