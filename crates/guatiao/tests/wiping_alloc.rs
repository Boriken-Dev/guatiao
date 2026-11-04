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

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use guatiao::value::{Allocator, Dump, rust_alloc, wiping_alloc};
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

/// A slot its kind marks `#[sensitive]`: what crosses it is built in, or
/// moved into, the wiping allocator, so the call leaves nothing behind
/// but what the provider kept of its own.
#[cfg(feature = "provider")]
mod slots {
    use core::ffi::c_void;

    use guatiao::library::{ProviderError, Remote};
    use guatiao::{Alloc, Map, Secret, Text, Value};

    use super::{SECRET, secret, seen};

    #[guatiao::kind(name = "wiped_vault")]
    pub trait Marked: Send + Sync {
        #[sensitive]
        fn read(&self) -> Result<Value, ProviderError>;
        #[sensitive]
        fn token(&self) -> Result<Secret<String>, ProviderError>;
        #[sensitive]
        fn store(&self, value: Secret<String>) -> Result<(), ProviderError>;
    }

    #[guatiao::kind(name = "plain_vault")]
    pub trait Plain: Send + Sync {
        fn read(&self) -> Result<Value, ProviderError>;
        fn token(&self) -> Result<Secret<String>, ProviderError>;
        fn store(&self, value: Secret<String>) -> Result<(), ProviderError>;
        /// Lent a map, and changes nothing.
        fn touch(&self, map: &mut Map) -> Result<(), ProviderError>;
    }

    /// Builds its answers in Rust's allocator, ignoring the carrier: the
    /// move into the caller's is what has to wipe.
    struct Careless;

    impl Marked for Careless {
        fn read(&self) -> Result<Value, ProviderError> {
            Ok(Text::new(secret()).into())
        }
        fn token(&self) -> Result<Secret<String>, ProviderError> {
            Ok(Secret::new(String::from(secret())))
        }
        fn store(&self, value: Secret<String>) -> Result<(), ProviderError> {
            assert_eq!(value.expose().as_bytes(), SECRET);
            Ok(())
        }
    }

    impl Plain for Careless {
        fn read(&self) -> Result<Value, ProviderError> {
            Ok(Text::new(secret()).into())
        }
        fn token(&self) -> Result<Secret<String>, ProviderError> {
            Ok(Secret::new(String::from(secret())))
        }
        fn store(&self, value: Secret<String>) -> Result<(), ProviderError> {
            assert_eq!(value.expose().as_bytes(), SECRET);
            Ok(())
        }
        fn touch(&self, _map: &mut Map) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    static CARELESS: Careless = Careless;
    static MARKED: MarkedVtable = MarkedVtable::of::<Careless>();
    static PLAIN: PlainVtable = PlainVtable::of::<Careless>();

    fn marked() -> Remote<dyn Marked> {
        // SAFETY: a table `of::<Careless>()` with the `&Careless` it was
        // built for, both `'static`.
        unsafe {
            Remote::<dyn Marked>::from_raw(
                &MARKED as *const MarkedVtable as *const c_void,
                size_of::<MarkedVtable>(),
                &CARELESS as *const Careless as *mut c_void,
            )
        }
        .expect("a full table validates")
    }

    fn plain() -> Remote<dyn Plain> {
        // SAFETY: as for `marked`.
        unsafe {
            Remote::<dyn Plain>::from_raw(
                &PLAIN as *const PlainVtable as *const c_void,
                size_of::<PlainVtable>(),
                &CARELESS as *const Careless as *mut c_void,
            )
        }
        .expect("a full table validates")
    }

    #[test]
    fn a_sensitive_tree_answer_built_elsewhere_is_moved_and_wiped() {
        let (m, p) = (marked(), plain());
        assert!(seen(|| drop(p.read().unwrap())) >= 1, "the control");
        assert_eq!(
            seen(|| {
                let v = m.read().unwrap();
                assert!(v.alloc().unwrap().is_wiping());
                drop(v);
            }),
            0
        );
    }

    #[test]
    fn a_sensitive_converted_answer_and_argument_leave_nothing() {
        let (m, p) = (marked(), plain());
        assert!(seen(|| drop(p.token().unwrap())) >= 1, "the control");
        assert_eq!(seen(|| drop(m.token().unwrap())), 0);
        assert!(
            seen(|| p.store(Secret::new(String::from(secret()))).unwrap()) >= 1,
            "the control"
        );
        assert_eq!(
            seen(|| m.store(Secret::new(String::from(secret()))).unwrap()),
            0
        );
    }

    /// A map lent `&mut` to a slot is settled after the call; a secret in it
    /// stays where it lives, in the wiping allocator, whatever the map is.
    #[test]
    fn a_secret_in_a_lent_map_stays_in_the_wiping_allocator() {
        let p = plain();
        let n = seen(|| {
            let mut map = Map::new();
            map.set("user", Text::new("alice"));
            map.set(
                "token",
                Value::from(Text::new_in(Alloc::wiping(), secret()).unwrap()),
            );
            p.touch(&mut map).unwrap();
            let token = map.get("token").unwrap();
            assert!(token.alloc().unwrap().is_wiping());
            let tree = Value::from(map);
            assert!(tree.is_sensitive());
            assert!(!format!("{:?}", super::Dump(&tree)).contains("hunter2"));
            drop(tree);
        });
        assert_eq!(n, 0);
    }
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

/// An allocator this build does not know that wipes on its own: what
/// another copy's wiping allocator, or a C author's, looks like from here.
mod foreign {
    use core::alloc::Layout;
    use core::ffi::c_void;

    use guatiao::value::Allocator;

    unsafe extern "C" fn alloc(_ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
        let Ok(layout) = Layout::from_size_align(size, align) else {
            return core::ptr::null_mut();
        };
        // SAFETY: a non-zero size, which guatiao never asks below.
        unsafe { alloc::alloc::alloc(layout).cast() }
    }

    unsafe extern "C" fn free(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
        for i in 0..size {
            // SAFETY: `p` is a live block of `size` bytes about to go.
            unsafe { p.cast::<u8>().add(i).write_volatile(0) };
        }
        // SAFETY: the block and its layout, as `alloc` made it.
        unsafe { alloc::alloc::dealloc(p.cast(), Layout::from_size_align_unchecked(size, align)) };
    }

    pub fn wiping() -> Allocator {
        Allocator {
            struct_size: size_of::<Allocator>() as u32,
            ctx: core::ptr::null_mut(),
            alloc: Some(alloc),
            free: Some(free),
            release: None,
        }
    }

    unsafe extern "C" fn plain_free(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
        // SAFETY: the block and its layout, as `alloc` made it.
        unsafe { alloc::alloc::dealloc(p.cast(), Layout::from_size_align_unchecked(size, align)) };
    }

    /// The same, with a `free` that does not wipe.
    pub fn plain() -> Allocator {
        Allocator {
            free: Some(plain_free),
            ..wiping()
        }
    }
}

/// A move between allocators overwrites the block it leaves, whatever the
/// target: here one this build does not recognise as wiping.
#[test]
fn a_move_into_an_allocator_this_build_does_not_know_leaves_nothing_behind() {
    let raw = foreign::wiping();
    // SAFETY: `raw` outlives every tree built through it, in this test.
    let other = unsafe { Alloc::from_raw(&raw) }.unwrap();
    assert!(!other.is_wiping());
    let tree = || {
        let r = Alloc::rust();
        let mut map = Map::new_in(r);
        map.set_in(secret(), Text::new_in(r, secret()).unwrap(), r)
            .unwrap();
        Value::from(map)
    };
    assert_eq!(seen(|| drop(tree().into_alloc(other).unwrap())), 0);
    assert_eq!(
        seen(|| {
            let mut v = tree();
            v.make_local_in(other).unwrap();
            drop(v);
        }),
        0
    );
    assert!(seen(|| drop(tree())) >= 2, "the control");
}

/// A move out of an allocator that does not wipe overwrites what it
/// leaves there before that allocator frees it: into this build's own
/// (`into_local`, `into_alloc`, `make_local_in`).
#[test]
fn a_move_out_of_an_allocator_that_does_not_wipe_overwrites_what_it_leaves() {
    let raw = foreign::plain();
    // SAFETY: `raw` outlives every tree built through it, in this test.
    let plain = unsafe { Alloc::from_raw(&raw) }.unwrap();
    let tree = || {
        let mut map = Map::new_in(plain);
        map.set_in(secret(), Text::new_in(plain, secret()).unwrap(), plain)
            .unwrap();
        Value::from(map)
    };
    let rust = Alloc::rust();
    assert!(seen(|| drop(tree())) >= 2, "the control");
    assert_eq!(
        seen(|| drop(tree().into_local().into_alloc(Alloc::wiping()).unwrap())),
        0
    );
    assert_eq!(
        seen(|| {
            let moved = tree().into_alloc(rust).unwrap();
            let mut moved = moved;
            moved.wipe();
        }),
        0
    );
    assert_eq!(
        seen(|| {
            let mut v = tree();
            v.make_local_in(rust).unwrap();
            v.wipe();
        }),
        0
    );
}

/// A copy of a node that lives in the wiping allocator is made in the
/// wiping allocator, whatever allocator the copy was asked for: a secret
/// inside a tree that does not, a key included, stays a secret.
#[test]
fn a_copy_of_a_secret_inside_a_plain_tree_stays_in_the_wiping_allocator() {
    use guatiao::exports::value::guatiao_value_clone;

    let w = Alloc::wiping();
    let plain = || {
        let mut map = Map::new();
        map.set("user", Text::new("alice"));
        map.set("token", Value::from(Text::new_in(w, secret()).unwrap()));
        let mut keyed = Map::new();
        keyed.set_in(secret(), Value::null(), w).unwrap();
        assert!(
            !keyed.alloc().unwrap().is_wiping(),
            "the array is not, the key is"
        );
        map.set("keyed", keyed);
        Value::from(map)
    };
    let n = seen(|| {
        let v = plain();
        let copies = [v.clone(), v.clone_in(Alloc::rust()).unwrap(), {
            let mut out = Value::absent();
            // SAFETY: a valid allocator and value; `out` holds nothing.
            let status = unsafe { guatiao_value_clone(Alloc::rust().as_raw(), &v, &mut out) };
            assert_eq!(status, guatiao::Status::GUATIAO_OK);
            out
        }];
        for copy in &copies {
            assert!(copy.is_sensitive());
            let shown = format!("{:?}", Dump(copy));
            assert!(
                !shown.contains("hunter2") && shown.contains("alice"),
                "{shown}"
            );
        }
        let text = Text::new_in(w, secret()).unwrap();
        assert!(
            text.clone_in(Alloc::rust())
                .unwrap()
                .alloc()
                .unwrap()
                .is_wiping()
        );
        drop((v, copies));
    });
    assert_eq!(n, 0);
}

/// `into_local` and `into_alloc` take a node that lives in this build's
/// wiping allocator as home: a tree holding a secret is local after
/// `into_local`, and a second one changes nothing.
#[test]
fn a_tree_holding_a_secret_is_local_and_moves_no_more() {
    let mut map = Map::new();
    map.set("user", Text::new("alice"));
    map.set(
        "token",
        Value::from(Text::new_in(Alloc::wiping(), secret()).unwrap()),
    );
    let v = Value::from(map).into_local();
    assert!(v.is_local());
    let at = |v: &Value| {
        let map: &Map = guatiao::TryAsRef::try_as_ref(v).unwrap();
        let token: &str = guatiao::TryAsRef::try_as_ref(map.get("token").unwrap()).unwrap();
        token.as_ptr()
    };
    let before = at(&v);
    let v = v.into_local();
    assert_eq!(at(&v), before, "a second into_local copied");
    let v = v.into_alloc(Alloc::rust()).unwrap();
    assert_eq!(at(&v), before, "into_alloc copied a tree already home");
    let mut v = v;
    v.make_local_in(Alloc::rust()).unwrap();
    assert_eq!(at(&v), before, "make_local_in moved a secret out");
}

/// A secret built through a copy of the wiping vtable that goes away (a C
/// caller's `guatiao_alloc` on its stack, filled by `guatiao_alloc_wiping`)
/// and then kept through a move: the container ends up recording this
/// build's own wiping allocator, never the copy.
#[test]
fn a_secret_kept_through_a_move_records_this_builds_own_wiping_allocator() {
    let wiping = Alloc::wiping().as_raw();
    for how in ["make_local_in", "into_local", "into_alloc"] {
        let copy = Box::new(wiping_alloc());
        // SAFETY: a complete vtable, alive until the tree moves.
        let through_copy = unsafe { Alloc::from_raw(&*copy) }.unwrap();
        assert!(through_copy.is_wiping());
        let mut map = Map::new();
        map.set("user", Text::new("alice"));
        map.set(
            "token",
            Value::from(Text::new_in(through_copy, secret()).unwrap()),
        );
        let map = match how {
            "make_local_in" => {
                let mut map = map;
                map.make_local_in(Alloc::rust()).unwrap();
                map
            }
            "into_local" => map.into_local(),
            _ => map.into_alloc(Alloc::rust()).unwrap(),
        };
        let token = map.get("token").unwrap().alloc().unwrap();
        assert!(
            core::ptr::eq(token.as_raw(), wiping),
            "{how} left the secret recording the copy"
        );
        drop(copy);
        assert!(map.get("token").unwrap().is_sensitive());
        drop(map);
    }
}
