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
use crate::value::error::ValueError;
use crate::value::raw::dangling;

use super::types::number::validate_json_number;
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
    /// Live when the tag is `GUATIAO_BOOL`. A `bool`: 0 or 1. A producer
    /// that writes any other byte has broken the contract.
    pub(crate) b: bool,
    /// Live when the tag is `GUATIAO_STRING`.
    pub(crate) text: ManuallyDrop<Text>,
    /// Live when the tag is `GUATIAO_NUMBER`. The same 32 bytes as `text`:
    /// a number is the exact text that declared it.
    pub(crate) number: ManuallyDrop<Number>,
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

    /// A payload holding a number.
    pub fn number(number: Number) -> Payload {
        Payload {
            number: ManuallyDrop::new(number),
        }
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

    /// A payload holding a boolean. The other arms are initialised too,
    /// so a node built over this is whole whatever its tag.
    pub fn bool(b: bool) -> Payload {
        let mut payload = Value::null().into_raw_parts().1;
        payload.b = b;
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
            Tag::GUATIAO_MAP => self.map_arm().ok_or(ValueError::WrongKind)?.alloc(),
            Tag::GUATIAO_LIST => arm::<List>(self)?.alloc(),
            Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER => {
                self.text_arm().ok_or(ValueError::WrongKind)?.alloc()
            }
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

    /// A deep copy, built through `alloc`. Explicit, because an owned
    /// tree carries its allocator and copying one into another heap costs
    /// a walk.
    ///
    /// Any depth: the walk is a loop over a heap stack. The copy is
    /// complete or it does not exist: a failure part-way frees everything
    /// already built.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        crate::value::walk::copy_value(self, alloc)
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

arm_as!(bool, b, Tag::GUATIAO_BOOL);
arm_as!(List, list, Tag::GUATIAO_LIST);
arm_as!(Buffer, bytes, Tag::GUATIAO_BYTES);

/// The arms whose content a producer can get wrong, read before any
/// check: freeing, finding the allocator and comparing must reach a
/// malformed foreign node too. What they hand out is read by its stored
/// bytes only, never as a `str`.
impl Value {
    /// A STRING's or a NUMBER's storage, laid out alike.
    fn text_arm(&self) -> Option<&Text> {
        match Tag::try_from(self.tag) {
            // SAFETY: the tag says the text arm is live, and a `Number` is
            // a `Text` in layout, so its arm is the same storage.
            Ok(Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER) => Some(unsafe { &self.payload.text }),
            _ => None,
        }
    }

    fn text_arm_mut(&mut self) -> Option<&mut Text> {
        match Tag::try_from(self.tag) {
            // SAFETY: as for `text_arm`.
            Ok(Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER) => {
                Some(unsafe { &mut self.payload.text })
            }
            _ => None,
        }
    }

    /// A MAP, keys unchecked: compare them with `Entry::key_bytes`.
    pub(crate) fn map_arm(&self) -> Option<&Map> {
        match Tag::try_from(self.tag) {
            // SAFETY: the tag says the map arm is live.
            Ok(Tag::GUATIAO_MAP) => Some(unsafe { &self.payload.map }),
            _ => None,
        }
    }

    fn map_arm_mut(&mut self) -> Option<&mut Map> {
        match Tag::try_from(self.tag) {
            // SAFETY: as for `map_arm`.
            Ok(Tag::GUATIAO_MAP) => Some(unsafe { &mut self.payload.map }),
            _ => None,
        }
    }

    /// The bytes under `tag`, STRING or NUMBER, whatever they are.
    pub(crate) fn text_bytes(&self, tag: Tag) -> Option<&[u8]> {
        if self.tag() != Ok(tag) {
            return None;
        }
        self.text_arm().map(Text::bytes)
    }

    /// A STRING whose bytes are UTF-8.
    fn holds_text(&self) -> bool {
        self.text_bytes(Tag::GUATIAO_STRING)
            .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok())
    }

    /// A NUMBER whose text is one.
    fn holds_a_number(&self) -> bool {
        self.text_bytes(Tag::GUATIAO_NUMBER)
            .is_some_and(|digits| validate_json_number(digits).is_ok())
    }

    /// A MAP whose keys are UTF-8.
    fn holds_a_map(&self) -> bool {
        self.map_arm().is_some_and(Map::keys_are_text)
    }
}

/// The doors to an arm whose content is checked: the tag, and what the
/// type promises -- a `Text` and a map's keys are UTF-8, a `Number` is
/// the JSON grammar. A foreign producer may write anything under the tag,
/// and such a node holds none of them. Once through, the type is trusted
/// and never checked again.
macro_rules! checked_arm {
    ($ty:ty, $field:ident, $holds:ident) => {
        impl TryAsRef<$ty> for Value {
            fn try_as_ref(&self) -> Option<&$ty> {
                if !self.$holds() {
                    return None;
                }
                // SAFETY: the tag says this arm is live, and its content
                // was just checked.
                Some(unsafe { &self.payload.$field })
            }
        }

        impl TryAsMut<$ty> for Value {
            fn try_as_mut(&mut self) -> Option<&mut $ty> {
                if !self.$holds() {
                    return None;
                }
                // SAFETY: as for `try_as_ref`. The type offers no way to
                // break what was checked.
                Some(unsafe { &mut self.payload.$field })
            }
        }

        /// Consuming the node; one that fails the check is handed back
        /// untouched.
        impl TryFrom<Value> for $ty {
            type Error = Value;

            fn try_from(value: Value) -> Result<$ty, Value> {
                if !value.$holds() {
                    return Err(value);
                }
                let (_, payload) = value.into_raw_parts();
                // SAFETY: checked just above, and `into_raw_parts` forgot
                // the node, so this is the arm's only owner.
                Ok(ManuallyDrop::into_inner(unsafe { payload.$field }))
            }
        }
    };
}

checked_arm!(Text, text, holds_text);
checked_arm!(Number, number, holds_a_number);
checked_arm!(Map, map, holds_a_map);

impl TryAsRef<str> for Value {
    /// A STRING's text. A number is not a string, so this answers `None`
    /// for one.
    fn try_as_ref(&self) -> Option<&str> {
        TryAsRef::<Text>::try_as_ref(self).map(|text| &**text)
    }
}

impl TryAsRef<[u8]> for Value {
    fn try_as_ref(&self) -> Option<&[u8]> {
        TryAsRef::<Buffer>::try_as_ref(self).map(|buffer| &buffer[..])
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

arm_into!(List, list, Tag::GUATIAO_LIST);
arm_into!(Buffer, bytes, Tag::GUATIAO_BYTES);

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
    /// allocator either side lives in, at any depth.
    fn eq(&self, other: &Value) -> bool {
        crate::value::walk::equal(crate::value::walk::Pair::Values(self, other))
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
            } else if let Some(map) = node.map_arm_mut() {
                map.dismantle_into(&mut stack);
            } else if let Some(text) = node.text_arm_mut() {
                text.release();
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
    /// Stores a `bool`, and the arm **is** a `bool`.
    ///
    /// Reading one back is infallible once the tag says `GUATIAO_BOOL`:
    /// `TryAsRef<bool>` answers `None` for the wrong arm and never for
    /// the byte. **A `bool` is a type, not content**: a producer that
    /// writes any other byte has broken the contract the way a bad
    /// pointer does, which is the half this crate trusts rather than the
    /// half it checks. A number's grammar and a text's UTF-8 are content
    /// and are checked; `true` and `false` are all a `bool` has.
    fn from(b: bool) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_BOOL);
        v.payload.b = b;
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
