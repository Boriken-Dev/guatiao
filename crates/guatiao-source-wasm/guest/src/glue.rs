// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the exports [`export!`](crate::export) writes call: the library
//! made once, a call decoded and dispatched, its answer handed over.

#![allow(unsafe_code)]

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cell::RefCell;

use guatiao::value::convert::TryAsRef;
use guatiao::value::wire;
use guatiao::{List, Status, Value};

use crate::call::{Answer, Call, Handler, Refusal};
use crate::host;
use crate::library::{Available, Build, Library};

/// The ABI this crate speaks.
pub const ABI: i32 = 1;

/// What a call's `object` names: a provider, an object one answered, or
/// an instance one built.
const PROVIDER: i32 = 0;
const OBJECT: i32 = 1;
const INSTANCE: i32 = 2;

/// One provider as the exports reach it.
struct Served {
    handler: Box<dyn Handler>,
    build: Option<Build>,
    available: Option<Available>,
}

/// The library's providers, the objects they answered and the instances
/// they built, by id.
struct Registry {
    providers: Vec<Served>,
    declaration: Value,
    objects: BTreeMap<i64, Box<dyn Handler>>,
    instances: BTreeMap<i64, Box<dyn Handler>>,
    next: i64,
}

std::thread_local! {
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
    /// What [`alloc`] handed out and no call has taken back: address to
    /// length.
    static HANDED_OUT: RefCell<BTreeMap<usize, usize>> = const { RefCell::new(BTreeMap::new()) };
}

fn with<R>(make: fn() -> Library, f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|cell| {
        let mut slot = cell.borrow_mut();
        let registry = slot.get_or_insert_with(|| {
            hook_panics();
            let library = make();
            let declaration = library.declaration();
            Registry {
                providers: library
                    .providers
                    .into_iter()
                    .map(|p| Served {
                        handler: p.handler,
                        build: p.build,
                        available: p.available,
                    })
                    .collect(),
                declaration,
                objects: BTreeMap::new(),
                instances: BTreeMap::new(),
                next: 0,
            }
        });
        f(registry)
    })
}

/// A panic aborts the module; its message goes to the host first, which
/// names it in the error the caller gets.
fn hook_panics() {
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let what = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| {
                payload
                    .downcast_ref::<alloc::string::String>()
                    .map(|s| s.as_str())
            })
            .unwrap_or("a panic");
        let words = match info.location() {
            Some(at) => alloc::format!("{what} ({}:{})", at.file(), at.line()),
            None => what.to_string(),
        };
        host::answer(words.as_bytes());
    }));
}

/// `guatiao_guest_alloc`: `len` zeroed bytes, the guest's.
pub fn alloc(len: usize) -> usize {
    let bytes: Box<[u8]> = alloc::vec![0u8; len].into_boxed_slice();
    let address = Box::into_raw(bytes).cast::<u8>() as usize;
    HANDED_OUT.with(|out| out.borrow_mut().insert(address, len));
    address
}

/// Takes back what [`alloc`] handed out for `len`, once; anything else is
/// refused, so no address a caller makes up is freed.
fn reclaim(address: usize, len: usize) -> Option<Box<[u8]>> {
    let taken = HANDED_OUT.with(|out| {
        let mut out = out.borrow_mut();
        let ours = out.get(&address) == Some(&len);
        if ours {
            out.remove(&address);
        }
        ours
    });
    if !taken {
        return None;
    }
    let slice = core::ptr::slice_from_raw_parts_mut(address as *mut u8, len);
    // SAFETY: `alloc` gave up a boxed slice of exactly `len` bytes at
    // `address` with `Box::into_raw`, and it was in `HANDED_OUT` until just
    // now, so nothing has taken it back.
    Some(unsafe { Box::from_raw(slice) })
}

fn refuse(refusal: &Refusal) -> u32 {
    host::answer(refusal.message().as_bytes());
    u32::from(refusal.status())
}

/// `guatiao_guest_declare`.
pub fn declare(make: fn() -> Library) -> u32 {
    with(make, |registry| {
        match wire::to_bytes(&registry.declaration) {
            Ok(bytes) => {
                host::answer(&bytes);
                0
            }
            Err(e) => refuse(&Refusal::bad_value(e.to_string())),
        }
    })
}

/// A request's kind, method and arguments.
fn read(request: &[u8]) -> Result<Call, Refusal> {
    let value = wire::decode(request)
        .map_err(|e| Refusal::bad_value(alloc::format!("the request: {e}")))?;
    let list =
        List::try_from(value).map_err(|_| Refusal::bad_value("the request is not a list"))?;
    let mut items = list.into_iter();
    let mut text = || {
        items
            .next()
            .and_then(|v| TryAsRef::<str>::try_as_ref(&v).map(ToString::to_string))
            .ok_or_else(|| Refusal::bad_value("the request names no kind and method"))
    };
    let kind = text()?;
    let method = text()?;
    Ok(Call::new(kind, method, items.collect()))
}

fn provider(registry: &mut Registry, target: i64) -> Result<&mut Served, Refusal> {
    usize::try_from(target)
        .ok()
        .and_then(|i| registry.providers.get_mut(i))
        .ok_or_else(|| Refusal::not_found(alloc::format!("no provider {target}")))
}

fn dispatch(
    registry: &mut Registry,
    object: i32,
    target: i64,
    mut call: Call,
) -> Result<Answer, Refusal> {
    let gone = |what: &str| {
        Refusal::new(
            Status::GUATIAO_ERR_GONE,
            alloc::format!("no {what} {target}"),
        )
    };
    let handler = match object {
        PROVIDER => &mut provider(registry, target)?.handler,
        OBJECT => registry
            .objects
            .get_mut(&target)
            .ok_or_else(|| gone("object"))?,
        INSTANCE => registry
            .instances
            .get_mut(&target)
            .ok_or_else(|| gone("instance"))?,
        other => {
            return Err(Refusal::unsupported(alloc::format!(
                "a call on a target of sort {other}"
            )));
        }
    };
    let answer = handler.call(&mut call)?;
    call.settle()?;
    Ok(answer)
}

/// `guatiao_guest_call`: `request` and `len` are what [`alloc`] answered
/// and the length asked for, the host having written the request there;
/// any other address is refused.
pub fn call(make: fn() -> Library, object: i32, target: i64, request: usize, len: usize) -> u32 {
    let Some(request) = reclaim(request, len) else {
        return refuse(&Refusal::bad_value(alloc::format!(
            "the request at {request} ({len} bytes) is not one guatiao_guest_alloc handed out"
        )));
    };
    with(make, |registry| {
        let outcome = read(&request).and_then(|call| dispatch(registry, object, target, call));
        match outcome {
            Ok(Answer::Nothing) => 0,
            Ok(Answer::Value(value)) => match wire::to_bytes(&value) {
                Ok(bytes) => {
                    host::answer(&bytes);
                    0
                }
                Err(e) => refuse(&Refusal::bad_value(e.to_string())),
            },
            Ok(Answer::Object(handler)) => {
                registry.next += 1;
                registry.objects.insert(registry.next, handler);
                match wire::to_bytes(&Value::from(registry.next)) {
                    Ok(bytes) => {
                        host::answer(&bytes);
                        0
                    }
                    Err(e) => refuse(&Refusal::bad_value(e.to_string())),
                }
            }
            Err(refusal) => refuse(&refusal),
        }
    })
}

/// `guatiao_guest_release`.
pub fn release(make: fn() -> Library, object: i64) {
    let gone = with(make, |registry| registry.objects.remove(&object));
    // Dropped outside the registry: a handler may drop host objects.
    drop(gone);
}

/// Answers `value` through the host: status 0, or a refusal when it does
/// not encode.
fn answer(value: &Value) -> u32 {
    match wire::to_bytes(value) {
        Ok(bytes) => {
            host::answer(&bytes);
            0
        }
        Err(e) => refuse(&Refusal::bad_value(e.to_string())),
    }
}

/// `guatiao_guest_create`: `config` and `len` as [`call`] takes a
/// request; answers the instance's id.
pub fn create(make: fn() -> Library, target: i64, config: usize, len: usize) -> u32 {
    let Some(config) = reclaim(config, len) else {
        return refuse(&Refusal::bad_value(alloc::format!(
            "the configuration at {config} ({len} bytes) is not one guatiao_guest_alloc handed out"
        )));
    };
    with(make, |registry| {
        let built = wire::decode(&config)
            .map_err(|e| Refusal::bad_value(alloc::format!("the configuration: {e}")))
            .and_then(|config| {
                let served = provider(registry, target)?;
                let build = served.build.as_mut().ok_or_else(|| {
                    Refusal::unsupported(alloc::format!("provider {target} builds no instances"))
                })?;
                build(config)
            });
        match built {
            Ok(handler) => {
                registry.next += 1;
                registry.instances.insert(registry.next, handler);
                answer(&Value::from(registry.next))
            }
            Err(refusal) => refuse(&refusal),
        }
    })
}

/// `guatiao_guest_destroy`.
pub fn destroy(make: fn() -> Library, instance: i64) {
    let gone = with(make, |registry| registry.instances.remove(&instance));
    // Dropped outside the registry: a handler may drop host objects.
    drop(gone);
}

/// `guatiao_guest_available`: nothing answered is available, text the
/// reason it is not.
pub fn available(make: fn() -> Library, target: i64) -> u32 {
    with(make, |registry| {
        let served = match provider(registry, target) {
            Ok(served) => served,
            Err(refusal) => return refuse(&refusal),
        };
        match served.available.as_mut().map_or(Ok(()), |f| f()) {
            Ok(()) => 0,
            Err(why) => answer(&Value::from(why.as_str())),
        }
    })
}

/// Reads a number the host answered, for the native tests.
#[cfg(test)]
pub(crate) fn number(bytes: &[u8]) -> Option<i64> {
    use guatiao::value::convert::FromValue;
    i64::from_value(&wire::decode(bytes).ok()?).ok()
}
