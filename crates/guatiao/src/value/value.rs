// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The node: a tag and the union that tag selects.
//!
//! [`Tag`] is here rather than in `types/` because it is not a
//! container -- it is the node's discriminant, and the field it
//! describes is on [`Value`].
//!
//! Not in `types/` with the containers, because it is not one of them --
//! it is what uses them. `crate::value::Value` is its public path.

// The tag variants are the C header's spelling, so they stay
// SCREAMING_CASE; cbindgen copies a doc comment here verbatim, so the
// obvious ones stay undocumented rather than adding noise to the header.
#![allow(non_camel_case_types)]
#![allow(missing_docs)]

use std::fmt;
use std::mem::ManuallyDrop;

// The raw layer this crate keeps to itself: the free walk, the
// allocator-taking mutators and the private helpers. Imported whole
// because the impl below calls into it at almost every line.
use crate::value::alloc::{Alloc, Allocator};
use crate::value::convert::{MapError, TryAsMut, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::raw::{dangling, release_buffer};

use super::types::{Buffer, Entry, List, Map, Number, Text};

/// The kind of a stored value.
///
/// **This type is deliberately NOT used as a field type.** It exists to
/// give C an enum it can switch on and a debugger can print by name;
/// A value's `tag` field is a plain `uint32_t`.
///
/// The distinction is a soundness one and the layout is identical either
/// way (measured). A Rust `#[repr(u32)]` enum has a *restricted set of
/// valid values*, so a ninth bit pattern arriving from a foreign caller
/// would be undefined behaviour the instant the struct is read — before
/// any `match`, before anything could reject it. As a `uint32_t` it is
/// merely an integer out of range, which a reader skips.
///
/// **New kinds are APPENDED.** A reader meeting a tag it does not know
/// must skip that one value and render the rest; that is the whole
/// forward-compatibility story, and it works only if numbers never move.
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
/// The payload of a value. Which arm is live is decided by the
/// tag, and by nothing else.
///
/// # Reading an arm
///
/// Reading a 32-byte arm off the wrong tag is garbage but defined: all
/// four are structs of pointers and integers, which have no validity
/// constraint beyond being initialised. Reading the `b` arm off the wrong
/// tag is **undefined behaviour** — `bool` is the one arm with a
/// restricted value set.
///
/// Both are prevented by one rule the constructors obey without
/// exception: **every node is born with all 40 bytes initialised**. A
/// payload written as `{ b: true }` alone initialises a single byte and
/// leaves thirty-one uninitialised, and reading any wide arm off that is
/// undefined too. Nothing in the toolchain warns about either.
///
/// The arms are `ManuallyDrop` because a union field must be `Copy` or
/// wrapped, and making an allocator-carrying container `Copy` would invite
/// a silent double free. It costs nothing at the boundary: a header
/// generator erases the wrapper entirely.
#[repr(C)]
pub union Payload {
    /// Live when the tag is `GUATIAO_BOOL`. Zero is false, **any**
    /// non-zero byte is true.
    ///
    /// A byte rather than a `bool`, and that is the one place this design
    /// deliberately refuses a nicer-looking type. A `bool` has a
    /// *restricted set of valid values* — it must be 0 or 1 — so a byte
    /// that is neither would be undefined behaviour to read at that type,
    /// at the moment of the read, before any check could reject it. A
    /// producer can write one without trying: a cast, a union, an
    /// uninitialised local.
    ///
    /// `u8` has no invalid bit patterns, so the hazard does not exist
    /// rather than being defended against at every read. It is the same
    /// reason the tag is a `u32` and not an enum, and it costs a C caller
    /// nothing: `.b = true` still stores 1.
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
    /// Reserved. Always written as zero, so the whole node is
    /// byte-comparable and a C caller has a named field to initialise.
    pub(crate) _pad: u32,
    /// The payload the tag selects.
    pub(crate) payload: Payload,
}

impl fmt::Debug for Payload {
    /// Prints nothing about the contents, deliberately.
    ///
    /// Which arm is live is decided by a tag this type does not carry, so
    /// any choice made here would be a guess — and the wrong guess on the
    /// `b` arm is undefined behaviour rather than a wrong line of output.
    /// [`Value`]'s own `Debug` is the one that has the tag.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Payload { .. }")
    }
}

impl fmt::Debug for Value {
    /// The tag, and nothing that requires following a pointer.
    ///
    /// A node may have been built by a foreign caller, so a `Debug` that
    /// walked into its payload would fault in a debugger — which is
    /// exactly where this gets called and exactly where a fault is least
    /// welcome.
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
    /// A payload holding text. Safe: the arm is a whole owned container,
    /// and every other arm is the same bytes.
    pub fn text(text: Text) -> Payload {
        Payload {
            text: ManuallyDrop::new(text),
        }
    }

    /// A payload holding a number: the same arm as text, since a number
    /// is stored as the text that declared it.
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
    /// write any and a reader treats non-zero as true. The other arms are
    /// initialised too, so a node built over this is whole whatever its
    /// tag.
    pub fn bool(byte: u8) -> Payload {
        let mut payload = Value::null().into_raw_parts().1;
        payload.b = byte;
        payload
    }
}

impl Value {
    /// A node from a tag and a payload described by hand: the one door for
    /// a literal another language declared. Everything else builds through
    /// the constructors on the kind.
    ///
    /// # Safety
    ///
    /// `tag` selects the arm `payload` was built with (a string or a
    /// number over [`Payload::text`], bytes over [`Payload::bytes`], a
    /// list over [`Payload::list`], a map over [`Payload::map`]), so every
    /// read of the node reads the arm that is live. The raw `u32`, so a
    /// producer's tag this build does not know can be declared: such a
    /// node owns nothing as far as this build can tell, and is passed
    /// through and never freed into. A boolean, a null or an absent node
    /// takes [`Payload::bool`].
    pub unsafe fn from_raw_parts(tag: u32, payload: Payload) -> Value {
        Value {
            tag,
            _pad: 0,
            payload,
        }
    }

    /// The raw tag and the payload, with ownership: this node no longer
    /// frees them. The tag is the integer, which may be one this build
    /// does not know.
    pub fn into_raw_parts(self) -> (u32, Payload) {
        let this = ManuallyDrop::new(self);
        // SAFETY: a bitwise copy out of a node that is never dropped, so
        // the arms have exactly one owner.
        let payload = unsafe { std::ptr::read(&this.payload) };
        (this.tag, payload)
    }

    /// A node with every one of its bytes initialised.
    ///
    /// The only way a node is created here. Writing a single union arm
    /// leaves the rest of the payload uninitialised, and reading a wide
    /// arm off that is undefined behaviour rather than garbage; nothing in
    /// the toolchain warns about it.
    pub(crate) fn blank(tag: Tag) -> Value {
        Value {
            tag: u32::from(tag),
            _pad: 0,
            // The widest arm, fully written. Every other arm is the same
            // 32 bytes, so this initialises all of them at once.
            payload: Payload::map(Map {
                ptr: dangling::<Entry>(),
                len: 0,
                cap: 0,
                alloc: std::ptr::null(),
            }),
        }
    }

    /// The allocator this tree grows through.
    ///
    /// **An owned container records the allocator that made it**, which
    /// is why nothing else has to be told one: a write into this tree
    /// allocates storage *for this tree*.
    ///
    /// **Two different refusals.** A scalar — null, bool, absent — has no
    /// container to have recorded one, and answers
    /// [`ValueError::WrongKind`]. A container whose `alloc` field is null
    /// — a literal some other language wrote as a brace initialiser —
    /// answers
    /// [`ValueError::Alloc(AllocError::Null)`](crate::value::AllocError);
    /// growing one means naming the allocator it adopts, which is what the
    /// `_in` operations take.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        let stored = self.recorded_alloc().ok_or(ValueError::WrongKind)?;
        // SAFETY: the address a container recorded is an allocator that
        // outlives it, by the contract on `Alloc`.
        Ok(unsafe { Alloc::from_raw(stored) }?)
    }

    /// The allocator a live container recorded, if this node has one.
    fn recorded_alloc(&self) -> Option<*const Allocator> {
        // SAFETY: each arm is read only under the tag that selects it.
        unsafe {
            Some(match self.tag().ok()? {
                Tag::GUATIAO_MAP => self.payload.map.alloc,
                Tag::GUATIAO_LIST => self.payload.list.alloc,
                Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER => self.payload.text.alloc,
                Tag::GUATIAO_BYTES => self.payload.bytes.alloc,
                _ => return None,
            })
        }
    }

    /// Stores `value` under `key`, **consuming** it. See [`Map::set`].
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        self.map_mut()?.set(key, value)
    }

    /// Appends `value`, **consuming** it. See [`List::push`].
    pub fn push(&mut self, value: impl Into<Value>) -> Result<(), ValueError> {
        self.list_mut()?.push(value)
    }

    /// Appends `value` to the list under `key`. See [`Map::push_into`].
    pub fn push_into(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        self.map_mut()?.push_into(key, value)
    }

    /// The value under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_map()?.get(key)
    }

    /// The value under `key`, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_map_mut()?.get_mut(key)
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.as_map().is_some_and(|m| m.contains_key(key))
    }

    fn map_mut(&mut self) -> Result<&mut Map, ValueError> {
        self.try_as_mut().ok_or(ValueError::WrongKind)
    }

    fn list_mut(&mut self) -> Result<&mut List, ValueError> {
        self.try_as_mut().ok_or(ValueError::WrongKind)
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

    /// A boolean. Owns nothing.
    pub fn bool(b: bool) -> Value {
        Value::from(b)
    }

    /// Text, copied onto Rust's heap.
    pub fn string(text: &str) -> Value {
        Text::new(text).into()
    }

    /// Text, copied into an allocator you name.
    pub fn string_in(alloc: Alloc, text: &str) -> Result<Value, ValueError> {
        Ok(Text::new_in(alloc, text)?.into())
    }

    /// Bytes, copied onto Rust's heap.
    pub fn bytes(bytes: &[u8]) -> Value {
        Buffer::new(bytes).into()
    }

    /// Bytes, copied into an allocator you name.
    pub fn bytes_in(alloc: Alloc, bytes: &[u8]) -> Result<Value, ValueError> {
        Ok(Buffer::new_in(alloc, bytes)?.into())
    }

    /// An empty map, as a value: `Map::new().into()`, for a caller that
    /// wants the node rather than the container.
    pub fn map() -> Value {
        Map::new().into()
    }

    /// The same, through an allocator you name.
    pub fn map_in(alloc: Alloc) -> Value {
        Map::new_in(alloc).into()
    }

    /// An empty list, as a value. See [`Value::map`].
    pub fn list() -> Value {
        List::new().into()
    }

    /// The same, through an allocator you name.
    pub fn list_in(alloc: Alloc) -> Value {
        List::new_in(alloc).into()
    }
}

impl Value {
    /// The kind this value is, or the raw integer if this build does not
    /// know it.
    ///
    /// Not the same as the `tag` FIELD, which is the `u32` a C caller
    /// writes: this reads that field and answers whether it names a kind
    /// this build can act on. An unknown one means skip this value, never
    /// stop.
    pub fn tag(&self) -> Result<Tag, ValueError> {
        Tag::try_from(self.tag)
    }

    /// The boolean, or `None` for any other kind.
    ///
    /// Any non-zero byte reads as `true`, matching C's own rule. The arm
    /// is a `u8`, which has no invalid bit patterns, so every byte a
    /// producer could have written is a valid value of it.
    pub fn as_bool(&self) -> Option<bool> {
        match self.tag() {
            // SAFETY: the tag says the `b` arm is live, and a node is born
            // with all 40 of its bytes initialised.
            Ok(Tag::GUATIAO_BOOL) => Some(unsafe { self.payload.b } != 0),
            _ => None,
        }
    }

    /// The text of a string value.
    pub fn as_str(&self) -> Option<&str> {
        self.try_as_ref()
    }

    /// The bytes of a bytes value. Any content at all, NULs included.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.try_as_ref()
    }

    /// A number's **exact text**, as it was written down.
    ///
    /// Numbers are text here, so `1.10` and `1.1` are different values and
    /// `u64::MAX` survives. Convert with `TryInto` when you want a machine
    /// width, and the refusal is then visible.
    pub fn as_number_str(&self) -> Option<&str> {
        Some(TryAsRef::<Number>::try_as_ref(self)?.as_str())
    }

    /// The map container, or `None` for any other kind.
    pub fn as_map(&self) -> Option<&Map> {
        self.try_as_ref()
    }

    /// The same, mutably.
    pub fn as_map_mut(&mut self) -> Option<&mut Map> {
        self.try_as_mut()
    }

    /// The list container, or `None` for any other kind.
    pub fn as_list(&self) -> Option<&List> {
        self.try_as_ref()
    }

    /// The same, mutably.
    pub fn as_list_mut(&mut self) -> Option<&mut List> {
        self.try_as_mut()
    }

    /// A map's entries, in insertion order, which is part of the contract.
    pub fn entries(&self) -> Option<&[Entry]> {
        Some(self.as_map()?.entries())
    }

    /// A list's elements, in order.
    pub fn items(&self) -> Option<&[Value]> {
        Some(self.as_list()?.items())
    }

    /// Removes `key` from a map and hands back its value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.as_map_mut()?.remove(key)
    }

    /// Removes `key` and frees its value. Answers whether it was there.
    pub fn discard(&mut self, key: &str) -> bool {
        self.as_map_mut().is_some_and(|m| m.discard(key))
    }

    /// Removes the element at `index` from a list, keeping the order of
    /// the rest.
    pub fn remove_at(&mut self, index: usize) -> Option<Value> {
        self.as_list_mut()?.remove(index)
    }

    /// Removes and frees the element at `index`.
    pub fn discard_at(&mut self, index: usize) -> bool {
        self.as_list_mut().is_some_and(|l| l.discard(index))
    }

    /// Frees every entry or element, **keeping the capacity** already
    /// paid for. Errors for a kind that holds neither.
    pub fn clear(&mut self) -> Result<(), ValueError> {
        match self.tag() {
            Ok(Tag::GUATIAO_MAP) => {
                self.map_mut()?.clear();
                Ok(())
            }
            Ok(Tag::GUATIAO_LIST) => {
                self.list_mut()?.clear();
                Ok(())
            }
            _ => Err(ValueError::WrongKind),
        }
    }

    /// The map this value holds, consuming it; `Err` hands the value back
    /// untouched when it is not a map.
    pub fn into_map(self) -> Result<Map, Value> {
        Map::try_from(self)
    }

    /// The list this value holds, consuming it; `Err` hands the value back
    /// untouched when it is not a list.
    pub fn into_list(self) -> Result<List, Value> {
        List::try_from(self)
    }

    /// A deep copy, built through `alloc`.
    ///
    /// Explicit, and deliberately so: an owned tree carries its allocator,
    /// so copying one into another heap costs a walk, and that cost should
    /// be a line a reader can see rather than something a setter does
    /// quietly.
    ///
    /// Bounded by [`MAX_DEPTH`], so a hostile tree is
    /// [`ValueError::TooDeep`] rather than a dead process. The copy is
    /// complete or it does not exist: a failure part-way frees everything
    /// already built.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.clone_at(alloc, 0)
    }

    pub(crate) fn clone_at(&self, alloc: Alloc, depth: u32) -> Result<Value, ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        match self.tag()? {
            Tag::GUATIAO_ABSENT => Ok(Value::absent()),
            Tag::GUATIAO_NULL => Ok(Value::null()),
            Tag::GUATIAO_BOOL => Ok(Value::from(self.as_bool().unwrap_or(false))),
            Tag::GUATIAO_STRING => {
                let text = self.text_arm().ok_or(ValueError::NotUtf8)?;
                Ok(text.clone_in(alloc)?.into())
            }
            Tag::GUATIAO_NUMBER => {
                let text = self.text_arm().ok_or(ValueError::NotUtf8)?;
                Ok(Number::from_text(text.clone_in(alloc)?).into())
            }
            Tag::GUATIAO_BYTES => {
                // SAFETY: the tag says the bytes arm is live.
                let buffer = unsafe { &*self.payload.bytes };
                Ok(buffer.clone_in(alloc)?.into())
            }
            Tag::GUATIAO_LIST => {
                // SAFETY: the tag says the list arm is live.
                let list = unsafe { &*self.payload.list };
                Ok(list.clone_at(alloc, depth)?.into())
            }
            Tag::GUATIAO_MAP => {
                // SAFETY: the tag says the map arm is live.
                let map = unsafe { &*self.payload.map };
                Ok(map.clone_at(alloc, depth)?.into())
            }
        }
    }

    /// The text arm, under either tag that selects it.
    fn text_arm(&self) -> Option<&Text> {
        match self.tag() {
            // SAFETY: the tag says the text arm is live.
            Ok(Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER) => Some(unsafe { &self.payload.text }),
            _ => None,
        }
    }

    /// Whether two trees hold the same thing, following no deeper than
    /// [`MAX_DEPTH`].
    ///
    /// Two trees nested deeper compare **unequal** without being walked
    /// further: the input can come from a foreign producer, and an
    /// unbounded recursion over one is a stack overflow, which on Windows
    /// is not catchable. A caller using this for uniqueness keeps an item
    /// it might have discarded, which is a value kept rather than lost.
    pub(crate) fn eq_at(&self, other: &Value, depth: u32) -> bool {
        if self.tag != other.tag {
            return false;
        }
        if depth >= MAX_DEPTH {
            return false;
        }
        match self.tag() {
            Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL) => true,
            Ok(Tag::GUATIAO_BOOL) => self.as_bool() == other.as_bool(),
            // Byte equality of the text, which is what makes `1.10`
            // different from `1.1`.
            Ok(Tag::GUATIAO_NUMBER | Tag::GUATIAO_STRING) => self.text_arm() == other.text_arm(),
            Ok(Tag::GUATIAO_BYTES) => self.as_bytes() == other.as_bytes(),
            Ok(Tag::GUATIAO_LIST) => match (self.as_list(), other.as_list()) {
                (Some(x), Some(y)) => x.eq_at(y, depth),
                _ => false,
            },
            Ok(Tag::GUATIAO_MAP) => match (self.as_map(), other.as_map()) {
                (Some(x), Some(y)) => x.eq_at(y, depth),
                _ => false,
            },
            // Two values this build cannot read are equal exactly when
            // their tags are, which the first line already established.
            Err(_) => true,
        }
    }

    /// Stores `value` under `key`, **moving** it and leaving the source
    /// null-tagged, adopting `alloc` for a container that has none.
    ///
    /// [`set`](Value::set) is the form to reach for: it consumes the value
    /// and takes the allocator off the tree. This one exists for a caller
    /// that has a `&mut Value` it must not consume — which is what a
    /// boundary hands over.
    ///
    /// # Safety
    ///
    /// Both nodes are well formed and `value`'s buffers came from an
    /// allocator that outlives this tree.
    pub unsafe fn set_in(
        &mut self,
        key: &str,
        value: &mut Value,
        alloc: Alloc,
    ) -> Result<(), ValueError> {
        let map = self.map_mut()?;
        map.set_in(key, std::mem::take(value), alloc)
    }

    /// Appends `value`, **moving** it. See [`set_in`](Value::set_in).
    ///
    /// # Safety
    ///
    /// As for [`set_in`](Value::set_in).
    pub unsafe fn push_in(&mut self, value: &mut Value, alloc: Alloc) -> Result<(), ValueError> {
        let list = self.list_mut()?;
        list.push_in(std::mem::take(value), alloc)
    }

    /// Copies every entry of `src` into this map, replacing keys that
    /// collide and appending the rest. Answers how many were copied.
    ///
    /// # Safety
    ///
    /// Both nodes are well formed, and `src` does not address this node or
    /// one inside it.
    pub unsafe fn copy_from(&mut self, src: &Value, alloc: Alloc) -> Result<usize, ValueError> {
        let copy = src.as_map().ok_or(ValueError::WrongKind)?.clone_in(alloc)?;
        self.map_mut()?.absorb(copy, alloc)
    }

    /// Appends to a string value in place.
    ///
    /// # Safety
    ///
    /// This value is a well-formed string, and `text` is readable for the
    /// call.
    pub unsafe fn push_str(&mut self, text: &str, alloc: Alloc) -> Result<(), ValueError> {
        let arm: &mut Text = self.try_as_mut().ok_or(ValueError::WrongKind)?;
        arm.push_str_in(text, alloc)
    }

    /// Appends to a bytes value in place.
    ///
    /// # Safety
    ///
    /// This value is a well-formed bytes value, and `bytes` is readable
    /// for the call.
    pub unsafe fn push_bytes(&mut self, bytes: &[u8], alloc: Alloc) -> Result<(), ValueError> {
        let arm: &mut Buffer = self.try_as_mut().ok_or(ValueError::WrongKind)?;
        arm.push_in(bytes, alloc)
    }

    /// Frees everything this value owns and leaves it null-tagged.
    ///
    /// Rust code does not call this — a value frees itself when it goes
    /// out of scope. It is here for the boundary, where a caller that has
    /// only a pointer has no scope to end.
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

/// One `TryAsRef`/`TryAsMut` pair per arm, guarded by the tags that select
/// it. `Number` and `Text` share the `text` arm and are listed separately,
/// because sharing storage is not sharing a type.
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
        // SAFETY: `Number` is `repr(transparent)` over `Text`, so the two
        // have one layout and this reinterprets the reference rather than
        // dereferencing it a second time.
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
/// untouched**: it owns a tree, so a refusal that dropped it would free
/// what the caller still wanted.
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
    /// A deep copy, grown through the allocator this tree recorded — the
    /// crate's own for a scalar, or for a literal that recorded none — so
    /// a copy lives where its source did.
    ///
    /// Panics if the allocator refuses or the tree is deeper than
    /// [`MAX_DEPTH`], the same policy as the short constructors;
    /// [`clone_in`](Value::clone_in) is the fallible form and the one
    /// that names an allocator.
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
// or `&mut self`, so no two threads share it without the borrow checker
// saying so; the allocator it recorded is a table that outlives it, by the contract on `Alloc`,
// and may be called from any thread, which is the contract on
// `Allocator` — a host handing out an arena synchronises it, as Rust's
// global allocator does.
unsafe impl Send for Value {}
// SAFETY: as above.
unsafe impl Sync for Value {}

impl Drop for Value {
    /// A value frees what it owns.
    ///
    /// **This is why there is no separate owning wrapper.** A `Value` is
    /// the thing a caller holds, and holding it is what makes it yours:
    /// it frees on drop like any other Rust value, and handing it to
    /// something else is a move, which is exactly when Rust stops
    /// dropping it. Crossing a boundary is therefore a plain
    /// `ptr::write`, with nothing to remember.
    ///
    /// A value that owns nothing — null, a boolean, the absent sentinel,
    /// a literal some other language declared with `cap == 0` — frees to
    /// nothing, so this is safe on every value however it was made.
    fn drop(&mut self) {
        // A scalar owns nothing, so leaving it null-tagged IS the whole
        // operation: no walk, and no heap for the walk's stack. Scalars
        // are most of the nodes in a tree.
        if !self.owns_storage() {
            self.tag = u32::from(Tag::GUATIAO_NULL);
            return;
        }

        // **Iterative on purpose.** A tree may have arrived from a foreign
        // caller, and recursion on adversarial depth is a stack overflow,
        // which on Windows is not catchable.
        //
        // Every node here is held in a `ManuallyDrop`: a node this loop
        // has already dismantled would otherwise be freed a second time
        // when its binding ended.
        let mut stack = vec![std::mem::take(self)];
        while let Some(node) = stack.pop() {
            let mut node = ManuallyDrop::new(node);
            match Tag::try_from(node.tag) {
                Ok(Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER) => {
                    // SAFETY: the tag says the text arm is live, and it
                    // describes its own storage.
                    unsafe { release_buffer(&mut *node.payload.text) };
                }
                Ok(Tag::GUATIAO_BYTES) => {
                    // SAFETY: as above, for the bytes arm.
                    unsafe { release_buffer(&mut *node.payload.bytes) };
                }
                Ok(Tag::GUATIAO_LIST) => {
                    // SAFETY: the tag says the list arm is live.
                    let l = unsafe { &mut *node.payload.list };
                    for i in 0..l.len {
                        // SAFETY: the first `len` elements are initialised.
                        stack.push(std::mem::take(unsafe { &mut *l.ptr.add(i) }));
                    }
                    l.len = 0;
                    // SAFETY: the elements have been moved out, so nothing
                    // reads the buffer again.
                    unsafe { release_buffer(l) };
                }
                Ok(Tag::GUATIAO_MAP) => {
                    // SAFETY: the tag says the map arm is live.
                    let m = unsafe { &mut *node.payload.map };
                    for i in 0..m.len {
                        // SAFETY: the first `len` entries are initialised.
                        let entry = unsafe { &mut *m.ptr.add(i) };
                        // SAFETY: the key is an owned text container.
                        unsafe { release_buffer(&mut entry.key) };
                        stack.push(std::mem::take(&mut entry.value));
                    }
                    m.len = 0;
                    // SAFETY: as above.
                    unsafe { release_buffer(m) };
                }
                // Absent, null, bool, and any tag this build does not
                // know: nothing is owned. An unknown tag is deliberately
                // not an error -- refusing to free a tree because one node
                // came from a newer producer would leak the whole tree to
                // punish the one node.
                _ => {}
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

/// A float converts **fallibly**, and that is not an oversight.
///
/// `NaN` and the infinities have no JSON spelling, so `f64` cannot promise
/// what `From` promises. `map.set("ratio", Number::try_from(x)?)` says out
/// loud that the value might not be one.
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
/// **The one blanket impl in this crate**, and it has to be the only one:
/// a second `From<T: Into<Something>>` overlaps with this one, because a
/// downstream type may implement both `Into`s, and the blanket `TryFrom`
/// collides with core's own. So numbers get the blanket and every other
/// kind gets a concrete impl below.
impl<T: Into<Number>> From<T> for Value {
    fn from(v: T) -> Value {
        let mut node = Value::blank(Tag::GUATIAO_NUMBER);
        node.payload = Payload::number(v.into());
        node
    }
}

/// A conversion that goes through the container that owns the operation:
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
