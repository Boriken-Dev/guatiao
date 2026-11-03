// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The wiping allocator, and `Wipe`, seen from the global allocator: this
//! binary's global allocator looks at every block as it is freed and
//! counts those still holding a planted secret. A secret built in
//! `Alloc::wiping` reaches it as zeros, the block a growing text left
//! behind included; the same secret in `Alloc::rust` does not, which is
//! what shows the watcher sees anything at all.
//!
//! Every block is allocated zeroed, so reading a freed block's spare
//! capacity reads initialised bytes (Miri checks that). The tests share
//! one counter and take a lock around each count.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use guatiao::value::{Allocator, Dump, rust_alloc};
use guatiao::{Alloc, Buffer, List, Map, Secret, Text, Value, Wipe};

const SECRET: &[u8] = b"hunter2-SECRET";

struct Watching;

/// Blocks freed while still holding [`SECRET`].
static SEEN: AtomicUsize = AtomicUsize::new(0);

/// The memory behind the watcher: `System`, or under Miri the
/// interpreter's own allocator, since `System`'s platform code is not
/// what is being checked here.
mod backing {
    use core::alloc::Layout;

    #[cfg(miri)]
    unsafe extern "Rust" {
        fn miri_alloc(size: usize, align: usize) -> *mut u8;
        fn miri_dealloc(ptr: *mut u8, size: usize, align: usize);
    }

    /// # Safety
    ///
    /// `layout` has a non-zero size.
    pub unsafe fn alloc(layout: Layout) -> *mut u8 {
        #[cfg(miri)]
        // SAFETY: the caller's contract.
        return unsafe { miri_alloc(layout.size(), layout.align()) };
        #[cfg(not(miri))]
        // SAFETY: the caller's contract.
        return unsafe { core::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) };
    }

    /// # Safety
    ///
    /// `ptr` came from [`alloc`] with `layout`.
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
        #[cfg(miri)]
        // SAFETY: the caller's contract.
        unsafe {
            miri_dealloc(ptr, layout.size(), layout.align())
        };
        #[cfg(not(miri))]
        // SAFETY: the caller's contract.
        unsafe {
            core::alloc::GlobalAlloc::dealloc(&std::alloc::System, ptr, layout)
        };
    }
}

// SAFETY: every block comes from `backing` with the layout it is released
// with. A block aligned to 1 is zeroed when allocated, so it holds only
// initialised bytes, and is read once, as it is released; any other block
// is never read.
unsafe impl GlobalAlloc for Watching {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, passed on.
        let p = unsafe { backing::alloc(layout) };
        if !p.is_null() && layout.align() == 1 {
            // SAFETY: a fresh block of `layout.size()` bytes.
            unsafe { p.write_bytes(0, layout.size()) };
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Bytes live in blocks aligned to 1 (a text's, a buffer's, a
        // `String`'s); a block of wider elements may hold padding, which
        // is never initialised and must not be read.
        if layout.align() == 1 {
            // SAFETY: `ptr` is a live block of `layout.size()` bytes,
            // about to be released, zeroed when allocated and written only
            // as bytes.
            let block = unsafe { core::slice::from_raw_parts(ptr, layout.size()) };
            if block.windows(SECRET.len()).any(|w| w == SECRET) {
                SEEN.fetch_add(1, Ordering::SeqCst);
            }
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { backing::dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Watching = Watching;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Runs `f` alone and answers how many freed blocks held the secret.
fn seen(f: impl FnOnce()) -> usize {
    let _alone: MutexGuard<'_, ()> = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    SEEN.store(0, Ordering::SeqCst);
    f();
    SEEN.load(Ordering::SeqCst)
}

fn secret() -> &'static str {
    core::str::from_utf8(SECRET).unwrap()
}

/// A text holding the secret, grown three times past its first block.
fn grown(alloc: Alloc) -> Text {
    let mut t = Text::new_in(alloc, secret()).unwrap();
    for _ in 0..3 {
        let more = t.len() * 2;
        t.push_str_in(&"x".repeat(more), alloc).unwrap();
    }
    t
}

#[test]
fn the_watcher_sees_a_secret_freed_unwiped() {
    let n = seen(|| drop(grown(Alloc::rust())));
    assert!(n >= 2, "the first block and every growth: {n}");
}

#[test]
fn a_text_in_the_wiping_allocator_is_wiped_when_it_grows_and_when_it_drops() {
    assert_eq!(seen(|| drop(grown(Alloc::wiping()))), 0);
}

#[test]
fn a_tree_in_the_wiping_allocator_leaves_nothing() {
    let n = seen(|| {
        let w = Alloc::wiping();
        let mut map = Map::new_in(w);
        map.set_in(secret(), Text::new_in(w, secret()).unwrap(), w)
            .unwrap();
        let mut list = List::new_in(w);
        for _ in 0..9 {
            list.push_in(Buffer::new_in(w, SECRET).unwrap(), w).unwrap();
        }
        map.set_in("list", list, w).unwrap();
        let v = Value::from(map);
        assert!(v.is_sensitive());
        drop(v);
    });
    assert_eq!(n, 0);
}

#[test]
fn a_tree_moved_into_the_wiping_allocator_leaves_nothing_behind() {
    let into = seen(|| {
        let v = Value::from(grown(Alloc::rust()));
        let moved = v.into_alloc(Alloc::wiping()).unwrap();
        assert!(moved.alloc().unwrap().is_wiping());
        drop(moved);
    });
    // The growth blocks were freed by Rust's allocator before the move:
    // what the move itself leaves is the last block, which it wipes.
    let control = seen(|| drop(Value::from(grown(Alloc::rust()))));
    assert_eq!(
        into + 1,
        control,
        "only the last block is the move's to wipe"
    );

    let settled = seen(|| {
        let mut v = Value::from(grown(Alloc::rust()));
        v.make_local_in(Alloc::wiping()).unwrap();
        drop(v);
    });
    assert_eq!(settled + 1, control);
}

#[test]
fn a_clone_of_a_secret_is_wiped_too() {
    let n = seen(|| {
        let v = Value::from(Text::new_in(Alloc::wiping(), secret()).unwrap());
        let copy = v.clone();
        assert!(copy.alloc().unwrap().is_wiping());
        assert!(copy.is_sensitive());
        drop((v, copy));
    });
    assert_eq!(n, 0);
}

#[test]
fn the_c_export_hands_out_the_same_allocator() {
    let mut raw = core::mem::MaybeUninit::<Allocator>::uninit();
    // SAFETY: `raw` is writable storage for one allocator.
    let status = unsafe { guatiao::exports::value::guatiao_alloc_wiping(raw.as_mut_ptr()) };
    assert_eq!(status, guatiao::Status::GUATIAO_OK);
    // SAFETY: written just now.
    let raw = unsafe { raw.assume_init() };
    // SAFETY: `raw` lives to the end of this test, past every tree.
    let alloc = unsafe { Alloc::from_raw(&raw) }.unwrap();
    assert!(alloc.is_wiping());
    // SAFETY: valid allocators.
    assert!(unsafe { guatiao::exports::value::guatiao_alloc_is_wiping(&raw) });
    let rust = rust_alloc();
    // SAFETY: as above.
    assert!(!unsafe { guatiao::exports::value::guatiao_alloc_is_wiping(&rust) });
    assert!(!Alloc::rust().is_wiping());
    assert_eq!(seen(|| drop(grown(alloc))), 0);
}

#[test]
fn a_dropped_secret_is_wiped() {
    let control = seen(|| drop(String::from(secret())));
    assert_eq!(control, 1);
    assert_eq!(seen(|| drop(Secret::new(String::from(secret())))), 0);
    assert_eq!(seen(|| drop(Secret::new(SECRET.to_vec()))), 0);
    let n = seen(|| {
        let v = Value::from(Text::new_in(Alloc::rust(), secret()).unwrap());
        drop(Secret::new(v));
    });
    assert_eq!(n, 0);
}

#[test]
fn wiping_a_tree_reaches_every_text_bytes_and_key_whatever_its_allocator() {
    let n = seen(|| {
        let mut map = Map::new();
        map.set(secret(), Text::new(secret()));
        map.set("bytes", Buffer::new(SECRET));
        let mut v = Value::from(map);
        v.wipe();
        assert_eq!(v.tag(), Ok(guatiao::Tag::GUATIAO_NULL));
    });
    assert_eq!(n, 0);
    let mut s = String::from(secret());
    s.wipe();
    assert!(s.is_empty());
}

#[test]
fn nothing_that_prints_a_secret_shows_it() {
    let w = Alloc::wiping();
    let mut map = Map::new_in(w);
    map.set_in("password", Text::new_in(w, secret()).unwrap(), w)
        .unwrap();
    let tree = Value::from(map);
    let leaf = Value::from(Text::new_in(w, secret()).unwrap());
    let mut outer = Map::new();
    outer.set("user", Text::new("alice"));
    outer.set("password", Value::from(Text::new_in(w, secret()).unwrap()));
    let outer = Value::from(outer);
    let shown = [
        format!("{tree:?}"),
        format!("{:?}", Dump(&tree)),
        format!("{:?}", Dump(&leaf)),
        format!("{:?}", Dump(&outer)),
        format!("{:?}", Secret::new(String::from(secret()))),
        format!("{}", Secret::new(String::from(secret()))),
    ];
    for s in &shown {
        assert!(!s.contains("hunter2"), "{s}");
    }
    assert_eq!(format!("{:?}", Dump(&leaf)), "[sensitive]");
    let outer_shown = format!("{:?}", Dump(&outer));
    assert!(
        outer_shown.contains("alice") && outer_shown.contains("[sensitive]"),
        "{outer_shown}"
    );
}

/// The wire encoding into the wiping allocator: the bytes on the way, a
/// vector of the global allocator's, are wiped once copied out.
#[test]
fn bytes_encoded_into_the_wiping_allocator_leave_nothing_on_the_way() {
    use guatiao::exports::channel::guatiao_wire_send;
    use guatiao::exports::value::guatiao_wire_encode;

    let encode_with = |alloc: Alloc| {
        seen(|| {
            let w = Alloc::wiping();
            let value = Value::from(Text::new_in(w, secret()).unwrap());
            for send in [false, true] {
                let mut out = Value::absent();
                // SAFETY: valid allocator and value; `out` holds nothing.
                let status = unsafe {
                    if send {
                        guatiao_wire_send(alloc.as_raw(), 7, &value, &mut out)
                    } else {
                        guatiao_wire_encode(alloc.as_raw(), &value, &mut out)
                    }
                };
                assert_eq!(status, guatiao::Status::GUATIAO_OK);
                drop(out);
            }
        })
    };
    assert!(encode_with(Alloc::rust()) >= 2, "the control");
    assert_eq!(encode_with(Alloc::wiping()), 0);
}

/// `guatiao_value_wipe`: a tree in an allocator that does not wipe is
/// overwritten where it stands, then freed, and left null.
#[test]
fn the_c_export_wipes_a_tree_in_any_allocator() {
    use guatiao::exports::value::guatiao_value_wipe;

    let n = seen(|| {
        let mut map = Map::new();
        map.set(secret(), Text::new(secret()));
        map.set("bytes", Buffer::new(SECRET));
        let mut v = Value::from(map);
        // SAFETY: a valid value this test owns.
        assert_eq!(
            unsafe { guatiao_value_wipe(&mut v) },
            guatiao::Status::GUATIAO_OK
        );
        assert_eq!(v.tag(), Ok(guatiao::Tag::GUATIAO_NULL));
    });
    assert_eq!(n, 0);
    // SAFETY: null is refused, not read.
    let status = unsafe { guatiao_value_wipe(core::ptr::null_mut()) };
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_NULL);
}
