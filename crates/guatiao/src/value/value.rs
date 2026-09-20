// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The node: a tag and the union that tag selects.
//!
//! [`Tag`] is here rather than in `types/` because it is not a container
//! -- it is what uses them.

// The tag variants keep the C header's SCREAMING_CASE spelling, and the
// obvious ones stay undocumented: cbindgen copies every doc comment here
// into the header.
#![allow(non_camel_case_types)]
#![allow(missing_docs)]

use std::fmt;
use std::mem::ManuallyDrop;

use crate::value::alloc::Alloc;
use crate::value::convert::{MapError, TryAsMut, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::raw::dangling;

use super::types::{Buffer, Entry, List, Map, Number, Text};

/// The kind of a stored value.
///
/// **Not used as a field type.** A value's `tag` field is a plain
/// `uint32_t`: a `repr(u32)` enum has a restricted set of valid values,
/// so a ninth bit pattern from a foreign caller would be undefined the
/// instant the struct is read, before any `match` could reject it. As an
/// integer it is merely out of range, and a reader skips it. The layout
/// is identical either way.
///
/// **New kinds are APPENDED**, which is the whole forward-compatibility
/// story and works only if numbers never move.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// No value: the answer to a lookup that found nothing, and the tag of
    /// an optional slot that is not filled in. Never stored in a list.
    GUATIAO_ABSENT = 0,
    /// The stored nothing. Distinct from absent, the same way JSON
    /// distinguishes a missing member from a null one.
    GUATIAO_NULL = 1,
    GUATIAO_BOOL = 2,
    /// A number, carried as the exact text that declared it. Read it with
    /// `guatiao_value_number`, not `guatiao_value_str`.
    GUATIAO_NUMBER = 3,
    GUATIAO_STRING = 4,
    GUATIAO_BYTES = 5,
    GUATIAO_LIST = 6,
    GUATIAO_MAP = 7,
}

impl TryFrom<u32> for Tag {
    type Error = ValueError;

    /// The tag for a raw `u32`, or [`ValueError::UnknownTag`] carrying the
    /// integer this build does not know.
    ///
    /// **The only way in from an integer.** Never `transmute`: a
    /// `repr(u32)` enum has a restricted set of valid values, so that is
    /// undefined behaviour at the moment of construction rather than at
    /// the `match` that would have rejected it.
    fn try_from(raw: u32) -> Result<Tag, ValueError> {
        match raw {
            0 => Ok(Tag::GUATIAO_ABSENT),
            1 => Ok(Tag::GUATIAO_NULL),
            2 => Ok(Tag::GUATIAO_BOOL),
            3 => Ok(Tag::GUATIAO_NUMBER),
            4 => Ok(Tag::GUATIAO_STRING),
            5 => Ok(Tag::GUATIAO_BYTES),
            6 => Ok(Tag::GUATIAO_LIST),
            7 => Ok(Tag::GUATIAO_MAP),
            _ => Err(ValueError::UnknownTag(raw)),
        }
    }
}

impl From<Tag> for u32 {
    /// The integer that actually crosses the boundary.
    fn from(tag: Tag) -> u32 {
        tag as u32
    }
}
/// The payload of a value. The tag decides which arm is live, and
/// nothing else does.
///
/// Reading a 32-byte arm off the wrong tag is garbage but defined; all
/// four are structs of pointers and integers. Reading the `b` arm off the
/// wrong tag is **undefined**, and so is reading any wide arm off a
/// payload written as `{ b: true }` alone. One rule prevents both:
/// **every node is born with all 40 bytes initialised**.
///
/// The arms are `ManuallyDrop` because a union field must be `Copy` or
/// wrapped, and an allocator-carrying container that was `Copy` would
/// invite a silent double free. A header generator erases the wrapper.
#[repr(C)]
pub union Payload {
    /// Live when the tag is `GUATIAO_BOOL`. Zero is false, **any**
    /// non-zero byte is true.
    ///
    /// A byte, not a `bool`: a `bool` must be 0 or 1, so any other byte
    /// would be undefined to read at that type, at the read, before a
    /// check could reject it — and a producer can write one without
    /// trying. `u8` has no invalid bit patterns, so the hazard does not
    /// exist rather than being defended against. `.b = true` still
    /// stores 1.
    pub(crate) b: u8,
    /// Live when the tag is `GUATIAO_STRING` **or** `GUATIAO_NUMBER`.
    pub(crate) text: ManuallyDrop<Text>,
    /// Live when the tag is `GUATIAO_BYTES`.
    pub(crate) bytes: ManuallyDrop<Buffer>,
    /// Live when the tag is `GUATIAO_LIST`.
    pub(crate) list: ManuallyDrop<List>,
    /// Live when the tag is `GUATIAO_MAP`.
    pub(crate) map: ManuallyDrop<Map>,
}

/// One value: a tag and its payload.
///
/// Read the tag first, then the arm it selects. A tag this build does not
/// recognise means skip this one value and carry on; it never means stop.
#[repr(C)]
pub struct Value {
    /// One of the `GUATIAO_*` tag constants.
    pub(crate) tag: u32,
    /// Reserved. Always zero, so a node is byte-comparable and a C
    /// caller has a named field to initialise.
    pub(crate) _pad: u32,
    /// The payload the tag selects.
    pub(crate) payload: Payload,
}

impl fmt::Debug for Payload {
    /// Prints nothing about the contents: the tag that decides which arm
    /// is live is not on this type, so any choice here would be a guess,
    /// and the wrong guess on the `b` arm is undefined behaviour.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Payload { .. }")
    }
}

impl fmt::Debug for Value {
    /// The tag, and nothing that follows a pointer: a node may have been
    /// built by a foreign caller, and a `Debug` that walked into its
    /// payload would fault in the debugger it is called from.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Value");
        match Tag::try_from(self.tag) {
            Ok(tag) => s.field("tag", &tag),
            Err(_) => s.field("tag", &format_args!("unknown({})", self.tag)),
        };
        s.finish_non_exhaustive()
    }
}

impl Payload {
    /// A payload holding text.
    pub fn text(text: Text) -> Payload {
        Payload {
            text: ManuallyDrop::new(text),
        }
    }

    /// A payload holding a number: the text arm, since a number is
    /// stored as the text that declared it.
    pub fn number(number: Number) -> Payload {
        Payload::text(number.into_text())
    }

    /// A payload holding bytes.
    pub fn bytes(buffer: Buffer) -> Payload {
        Payload {
            bytes: ManuallyDrop::new(buffer),
        }
    }

    /// A payload holding a list.
    pub fn list(list: List) -> Payload {
        Payload {
            list: ManuallyDrop::new(list),
        }
    }

    /// A payload holding a map.
    pub fn map(map: Map) -> Payload {
        Payload {
            map: ManuallyDrop::new(map),
        }
    }

    /// A payload holding a boolean byte: any byte, since a producer may
    /// write any. The other arms are initialised too, so a node built
    /// over this is whole whatever its tag.
    pub fn bool(byte: u8) -> Payload {
        let mut payload = Value::null().into_raw_parts().1;
        payload.b = byte;
        payload
    }
}

/// The live arm as `T`, for a dispatch that has already read the tag.
fn arm<T: ?Sized>(v: &Value) -> Result<&T, ValueError>
where
    Value: TryAsRef<T>,
{
    v.try_as_ref().ok_or(ValueError::WrongKind)
}

impl Value {
    /// A node from a tag and a payload described by hand: the one door
    /// for a literal another language declared.
    ///
    /// # Safety
    ///
    /// `tag` selects the arm `payload` was built with, so every read of
    /// the node reads the live one. It is the raw `u32` so a producer's
    /// unknown tag can be declared; such a node owns nothing this build
    /// can see. A boolean, a null or an absent node takes
    /// [`Payload::bool`].
    pub unsafe fn from_raw_parts(tag: u32, payload: Payload) -> Value {
        Value {
            tag,
            _pad: 0,
            payload,
        }
    }

    /// The raw tag and the payload, with ownership: this node no longer
    /// frees them.
    pub fn into_raw_parts(self) -> (u32, Payload) {
        let this = ManuallyDrop::new(self);
        // SAFETY: a bitwise copy out of a node that is never dropped, so
        // the arms have exactly one owner.
        let payload = unsafe { std::ptr::read(&this.payload) };
        (this.tag, payload)
    }

    /// A node with every one of its bytes initialised: the only way one
    /// is created here.
    pub(crate) fn blank(tag: Tag) -> Value {
        Value {
            tag: u32::from(tag),
            _pad: 0,
            // The widest arm, fully written: every arm is the same 32
            // bytes, so this initialises all of them at once.
            payload: Payload::map(Map {
                ptr: dangling::<Entry>(),
                len: 0,
                cap: 0,
                alloc: std::ptr::null(),
            }),
        }
    }

    /// The allocator this tree grows through, which an owned container
    /// recorded when it was made.
    ///
    /// **Two different refusals.** A scalar has no container to have
    /// recorded one and answers [`ValueError::WrongKind`]. A container
    /// whose `alloc` field is null — a literal another language wrote —
    /// answers
    /// [`ValueError::Alloc(AllocError::Null)`](crate::value::AllocError),
    /// and growing it means naming the allocator it adopts: the `_in`
    /// form of any operation.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        match self.tag()? {
            Tag::GUATIAO_MAP => arm::<Map>(self)?.alloc(),
            Tag::GUATIAO_LIST => arm::<List>(self)?.alloc(),
            Tag::GUATIAO_STRING => arm::<Text>(self)?.alloc(),
            Tag::GUATIAO_NUMBER => arm::<Number>(self)?.alloc(),
            Tag::GUATIAO_BYTES => arm::<Buffer>(self)?.alloc(),
            _ => Err(ValueError::WrongKind),
        }
    }
}

impl Value {
    /// The stored nothing. Owns nothing, so it needs no allocator.
    pub fn null() -> Value {
        Value::blank(Tag::GUATIAO_NULL)
    }

    /// The absent sentinel: a value that says there is no value, which is
    /// a different statement from null.
    pub fn absent() -> Value {
        Value::blank(Tag::GUATIAO_ABSENT)
    }
}

impl Value {
    /// The kind this value is, or an error naming the raw integer when
    /// this build does not know it. An unknown tag means skip this value,
    /// never stop.
    pub fn tag(&self) -> Result<Tag, ValueError> {
        Tag::try_from(self.tag)
    }

    /// The boolean, or `None` for any other kind. Any non-zero byte is
    /// `true`. There is no `TryAsRef<bool>`, because no `&bool` over that
    /// byte would be sound; `bool::try_from(&value)` is the public door.
    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self.tag() {
            // SAFETY: the tag says the `b` arm is live, and a node is born
            // with all 40 of its bytes initialised.
            Ok(Tag::GUATIAO_BOOL) => Some(unsafe { self.payload.b } != 0),
            _ => None,
        }
    }

    /// A deep copy, built through `alloc`. Explicit, because an owned
    /// tree carries its allocator and copying one into another heap costs
    /// a walk.
    ///
    /// Bounded by [`MAX_DEPTH`]. The copy is complete or it does not
    /// exist: a failure part-way frees everything already built.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.clone_at(alloc, 0)
    }

    /// Hands off to the live container's own copy; `depth` bounds the walk
    /// at [`MAX_DEPTH`].
    pub(crate) fn clone_at(&self, alloc: Alloc, depth: u32) -> Result<Value, ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        Ok(match self.tag()? {
            Tag::GUATIAO_ABSENT => Value::absent(),
            Tag::GUATIAO_NULL => Value::null(),
            Tag::GUATIAO_BOOL => Value::from(self.as_bool().unwrap_or(false)),
            Tag::GUATIAO_STRING => arm::<Text>(self)?.clone_in(alloc)?.into(),
            Tag::GUATIAO_NUMBER => arm::<Number>(self)?.clone_in(alloc)?.into(),
            Tag::GUATIAO_BYTES => arm::<Buffer>(self)?.clone_in(alloc)?.into(),
            Tag::GUATIAO_LIST => arm::<List>(self)?.clone_at(alloc, depth)?.into(),
            Tag::GUATIAO_MAP => arm::<Map>(self)?.clone_at(alloc, depth)?.into(),
        })
    }

    /// Whether two trees hold the same thing, following no deeper than
    /// [`MAX_DEPTH`]. Two nested deeper compare **unequal** rather than
    /// overflowing a stack: a caller using this for uniqueness keeps an
    /// item it might have discarded.
    pub(crate) fn eq_at(&self, other: &Value, depth: u32) -> bool {
        /// Both sides as `T`, compared by `T`'s own rule.
        fn same<T: ?Sized>(a: &Value, b: &Value, eq: impl Fn(&T, &T) -> bool) -> bool
        where
            Value: TryAsRef<T>,
        {
            matches!((a.try_as_ref(), b.try_as_ref()), (Some(x), Some(y)) if eq(x, y))
        }

        if self.tag != other.tag || depth >= MAX_DEPTH {
            return false;
        }
        match self.tag() {
            Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL) => true,
            Ok(Tag::GUATIAO_BOOL) => self.as_bool() == other.as_bool(),
            Ok(Tag::GUATIAO_STRING) => same::<Text>(self, other, Text::eq),
            Ok(Tag::GUATIAO_NUMBER) => same::<Number>(self, other, Number::eq),
            Ok(Tag::GUATIAO_BYTES) => same::<Buffer>(self, other, Buffer::eq),
            Ok(Tag::GUATIAO_LIST) => same::<List>(self, other, |x, y| x.eq_at(y, depth)),
            Ok(Tag::GUATIAO_MAP) => same::<Map>(self, other, |x, y| x.eq_at(y, depth)),
            // Unknown to this build: equal exactly when the tags are.
            Err(_) => true,
        }
    }

    /// Frees everything this value owns and leaves it null-tagged. Rust
    /// code does not call this; it is here for the boundary, where a
    /// caller holding only a pointer has no scope to end.
    ///
    /// # Safety
    ///
    /// The node's containers describe their own storage, and nothing else
    /// refers to what they own.
    pub unsafe fn free(&mut self) {
        drop(std::mem::take(self));
    }
}

// --- the value seen as the kind it holds --------------------------------

/// One pair per arm, guarded by the tags that select it. `Number` and
/// `Text` share the `text` arm and are listed separately: sharing
/// storage is not sharing a type.
macro_rules! arm_as {
    ($ty:ty, $field:ident, $($tag:pat_param)|+) => {
        impl TryAsRef<$ty> for Value {
            fn try_as_ref(&self) -> Option<&$ty> {
                match Tag::try_from(self.tag) {
                    // SAFETY: the tag says this arm is live, and a node is
                    // born with all 40 bytes initialised.
                    $(Ok($tag))|+ => Some(unsafe { &self.payload.$field }),
                    _ => None,
                }
            }
        }

        impl TryAsMut<$ty> for Value {
            fn try_as_mut(&mut self) -> Option<&mut $ty> {
                match Tag::try_from(self.tag) {
                    // SAFETY: as above.
                    $(Ok($tag))|+ => Some(unsafe { &mut self.payload.$field }),
                    _ => None,
                }
            }
        }
    };
}

arm_as!(Map, map, Tag::GUATIAO_MAP);
arm_as!(List, list, Tag::GUATIAO_LIST);
arm_as!(Buffer, bytes, Tag::GUATIAO_BYTES);
arm_as!(Text, text, Tag::GUATIAO_STRING);

impl TryAsRef<Number> for Value {
    fn try_as_ref(&self) -> Option<&Number> {
        let text: &Text = match Tag::try_from(self.tag) {
            // SAFETY: the tag says the text arm is live.
            Ok(Tag::GUATIAO_NUMBER) => unsafe { &self.payload.text },
            _ => return None,
        };
        // SAFETY: `Number` is `repr(transparent)` over `Text`, so this
        // reinterprets the reference rather than dereferencing it twice.
        Some(unsafe { &*std::ptr::from_ref(text).cast::<Number>() })
    }
}

impl TryAsMut<Number> for Value {
    fn try_as_mut(&mut self) -> Option<&mut Number> {
        let text: &mut Text = match Tag::try_from(self.tag) {
            // SAFETY: the tag says the text arm is live.
            Ok(Tag::GUATIAO_NUMBER) => unsafe { &mut self.payload.text },
            _ => return None,
        };
        // SAFETY: as above.
        Some(unsafe { &mut *std::ptr::from_mut(text).cast::<Number>() })
    }
}

impl TryAsRef<str> for Value {
    /// A STRING's text. A number is not a string, so this answers `None`
    /// for one.
    fn try_as_ref(&self) -> Option<&str> {
        TryAsRef::<Text>::try_as_ref(self)?.as_str()
    }
}

impl TryAsRef<[u8]> for Value {
    fn try_as_ref(&self) -> Option<&[u8]> {
        Some(TryAsRef::<Buffer>::try_as_ref(self)?.as_slice())
    }
}

// --- taking the container out of the node -------------------------------

/// Consuming conversions. The error is the **value handed back
/// untouched**: a refusal that dropped it would free what the caller
/// still wanted.
///
/// ```
/// # use guatiao::{Map, Tag, Value};
/// let refused = Map::try_from(Value::null()).unwrap_err();
/// assert_eq!(refused.tag(), Ok(Tag::GUATIAO_NULL));
/// ```
macro_rules! arm_into {
    ($ty:ty, $field:ident, $tag:path) => {
        impl TryFrom<Value> for $ty {
            type Error = Value;

            fn try_from(value: Value) -> Result<$ty, Value> {
                if Tag::try_from(value.tag) != Ok($tag) {
                    return Err(value);
                }
                let (_, payload) = value.into_raw_parts();
                // SAFETY: the tag was just checked, and `into_raw_parts`
                // forgot the node, so this is the arm's only owner.
                Ok(ManuallyDrop::into_inner(unsafe { payload.$field }))
            }
        }
    };
}

arm_into!(Map, map, Tag::GUATIAO_MAP);
arm_into!(List, list, Tag::GUATIAO_LIST);
arm_into!(Buffer, bytes, Tag::GUATIAO_BYTES);
arm_into!(Text, text, Tag::GUATIAO_STRING);

impl TryFrom<Value> for Number {
    type Error = Value;

    fn try_from(value: Value) -> Result<Number, Value> {
        if Tag::try_from(value.tag) != Ok(Tag::GUATIAO_NUMBER) {
            return Err(value);
        }
        let (_, payload) = value.into_raw_parts();
        // SAFETY: as `arm_into!`; a number's digits live in the text arm.
        Ok(Number::from_text(ManuallyDrop::into_inner(unsafe {
            payload.text
        })))
    }
}

/// A borrowed container, with the error a reader wants: which kind was
/// needed and which was there.
macro_rules! arm_borrowed {
    ($ty:ty, $tag:path) => {
        impl<'a> TryFrom<&'a Value> for &'a $ty {
            type Error = MapError;

            fn try_from(value: &'a Value) -> Result<&'a $ty, MapError> {
                value
                    .try_as_ref()
                    .ok_or_else(|| MapError::wrong_type($tag, value))
            }
        }
    };
}

arm_borrowed!(Map, Tag::GUATIAO_MAP);
arm_borrowed!(List, Tag::GUATIAO_LIST);

impl Clone for Value {
    /// A deep copy through the allocator this tree recorded, so a copy
    /// lives where its source did. Panics where the short constructors
    /// do; [`clone_in`](Value::clone_in) is the fallible form.
    fn clone(&self) -> Value {
        let alloc = self.alloc().unwrap_or_else(|_| Alloc::rust());
        self.clone_in(alloc)
            .expect("a well-formed tree clones through a working allocator")
    }
}

/// Null, so `mem::take` leaves a node that owns nothing.
impl Default for Value {
    fn default() -> Value {
        Value::null()
    }
}

impl PartialEq for Value {
    /// Structural: the same kind and the same contents, whatever
    /// allocator either side lives in.
    fn eq(&self, other: &Value) -> bool {
        self.eq_at(other, 0)
    }
}

impl Eq for Value {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`; the allocator it recorded outlives it and may be
// called from any thread, which is the contract on `Allocator`.
unsafe impl Send for Value {}
// SAFETY: as above.
unsafe impl Sync for Value {}

impl Drop for Value {
    /// A value frees what it owns, which is why there is no separate
    /// owning wrapper: holding a `Value` is what makes it yours, and
    /// handing it away is a move. A value that owns nothing — a scalar,
    /// or a literal with `cap == 0` — frees to nothing.
    fn drop(&mut self) {
        // A scalar owns nothing, so null-tagging it IS the whole
        // operation: no walk, and no heap for the walk's stack.
        if !self.owns_storage() {
            self.tag = u32::from(Tag::GUATIAO_NULL);
            return;
        }

        // Iterative on purpose: a tree may have arrived from a foreign
        // caller, and recursion on adversarial depth is a stack overflow
        // that Windows cannot catch. Every node is held in a
        // `ManuallyDrop`, or one this loop has already dismantled would
        // be freed again when its binding ended.
        let mut stack = vec![std::mem::take(self)];
        while let Some(node) = stack.pop() {
            let mut node = ManuallyDrop::new(node);
            let node: &mut Value = &mut node;
            // Each container takes itself apart. An unknown tag owns
            // nothing this build can name, and is not an error.
            if let Some(list) = TryAsMut::<List>::try_as_mut(node) {
                list.dismantle_into(&mut stack);
            } else if let Some(map) = TryAsMut::<Map>::try_as_mut(node) {
                map.dismantle_into(&mut stack);
            } else if let Some(text) = TryAsMut::<Text>::try_as_mut(node) {
                text.release();
            } else if let Some(number) = TryAsMut::<Number>::try_as_mut(node) {
                number.release();
            } else if let Some(buffer) = TryAsMut::<Buffer>::try_as_mut(node) {
                buffer.release();
            }
        }
    }
}

impl Value {
    /// Whether this node's tag selects an arm that owns a buffer.
    fn owns_storage(&self) -> bool {
        matches!(
            self.tag(),
            Ok(Tag::GUATIAO_STRING
                | Tag::GUATIAO_NUMBER
                | Tag::GUATIAO_BYTES
                | Tag::GUATIAO_LIST
                | Tag::GUATIAO_MAP)
        )
    }
}

impl From<bool> for Value {
    /// Stores 1 or 0. The arm is a byte rather than a `bool` so that a
    /// value written by somebody else is still readable.
    fn from(b: bool) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_BOOL);
        v.payload.b = u8::from(b);
        v
    }
}

/// A float converts **fallibly**: `NaN` and the infinities have no JSON
/// spelling, so `f64` cannot promise what `From` promises.
impl TryFrom<f64> for Value {
    type Error = ValueError;

    fn try_from(v: f64) -> Result<Value, ValueError> {
        Ok(Number::try_from(v)?.into())
    }
}

impl TryFrom<f32> for Value {
    type Error = ValueError;

    fn try_from(v: f32) -> Result<Value, ValueError> {
        Ok(Number::try_from(v)?.into())
    }
}

/// `None` is a stored null, matching [`ToValue for Option`](crate::ToValue).
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        match v {
            Some(v) => v.into(),
            None => Value::null(),
        }
    }
}

/// Any number, whatever it was written from.
///
/// **The one blanket impl in this crate**, and necessarily the only one:
/// a second `From<T: Into<_>>` overlaps with it, because a downstream
/// type may implement both `Into`s, and a blanket `TryFrom` collides
/// with core's own.
impl<T: Into<Number>> From<T> for Value {
    fn from(v: T) -> Value {
        let mut node = Value::blank(Tag::GUATIAO_NUMBER);
        node.payload = Payload::number(v.into());
        node
    }
}

/// `Value::from("x")` is `Text::from("x").into()`, written once.
macro_rules! value_from_via {
    ($($container:ident: $($source:ty),+ );* $(;)?) => {$($(
        impl From<$source> for Value {
            fn from(v: $source) -> Value {
                <$container>::from(v).into()
            }
        }
    )+)*};
}

value_from_via!(Text: &str, String, &String; Buffer: &[u8], Vec<u8>);
