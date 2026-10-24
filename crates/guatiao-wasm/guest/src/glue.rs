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
use crate::library::Library;

/// The ABI this crate speaks.
pub const ABI: i32 = 1;

/// The library's providers and the objects they answered, by id.
struct Registry {
    providers: Vec<Box<dyn Handler>>,
    declaration: Value,
    objects: BTreeMap<i64, Box<dyn Handler>>,
    next: i64,
}

std::thread_local! {
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
}

fn with<R>(make: fn() -> Library, f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|cell| {
        let mut slot = cell.borrow_mut();
        let registry = slot.get_or_insert_with(|| {
            hook_panics();
            let library = make();
            let declaration = library.declaration();
            Registry {
                providers: library.providers.into_iter().map(|p| p.handler).collect(),
                declaration,
                objects: BTreeMap::new(),
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
    Box::into_raw(bytes).cast::<u8>() as usize
}

/// Takes back what [`alloc`] handed out.
///
/// # Safety
///
/// `address` and `len` are an address [`alloc`] answered for `len` and
/// that length, not taken back before.
unsafe fn reclaim(address: usize, len: usize) -> Box<[u8]> {
    let slice = core::ptr::slice_from_raw_parts_mut(address as *mut u8, len);
    // SAFETY: by the contract, a boxed slice of exactly `len` bytes that
    // `alloc` gave up with `Box::into_raw`.
    unsafe { Box::from_raw(slice) }
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

fn dispatch(
    registry: &mut Registry,
    object: bool,
    target: i64,
    mut call: Call,
) -> Result<Answer, Refusal> {
    let handler = if object {
        registry.objects.get_mut(&target).ok_or_else(|| {
            Refusal::new(
                Status::GUATIAO_ERR_GONE,
                alloc::format!("no object {target}"),
            )
        })?
    } else {
        usize::try_from(target)
            .ok()
            .and_then(|i| registry.providers.get_mut(i))
            .ok_or_else(|| Refusal::not_found(alloc::format!("no provider {target}")))?
    };
    let answer = handler.call(&mut call)?;
    call.settle()?;
    Ok(answer)
}

/// `guatiao_guest_call`.
///
/// # Safety
///
/// `request` and `len` are an address [`alloc`] answered for `len`, which
/// the host filled and hands back here, once.
pub unsafe fn call(
    make: fn() -> Library,
    object: i32,
    target: i64,
    request: usize,
    len: usize,
) -> u32 {
    // SAFETY: the caller's contract is `reclaim`'s.
    let request = unsafe { reclaim(request, len) };
    with(make, |registry| {
        let outcome = read(&request).and_then(|call| dispatch(registry, object != 0, target, call));
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

/// Reads a number the host answered, for the native tests.
#[cfg(test)]
pub(crate) fn number(bytes: &[u8]) -> Option<i64> {
    use guatiao::value::convert::FromValue;
    i64::from_value(&wire::decode(bytes).ok()?).ok()
}
