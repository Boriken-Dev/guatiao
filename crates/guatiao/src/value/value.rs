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
use crate::value::alloc::Alloc;
use crate::value::mutate::*;

use super::types::{Buffer, Entry, List, Map, Text};

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
    pub b: u8,
    /// Live when the tag is `GUATIAO_STRING` **or** `GUATIAO_NUMBER`.
    pub text: ManuallyDrop<Text>,
    /// Live when the tag is `GUATIAO_BYTES`.
    pub bytes: ManuallyDrop<Buffer>,
    /// Live when the tag is `GUATIAO_LIST`.
    pub list: ManuallyDrop<List>,
    /// Live when the tag is `GUATIAO_MAP`.
    pub map: ManuallyDrop<Map>,
}

/// One value: a tag and its payload.
///
/// Read the tag first, then the arm it selects. A tag this build does not
/// recognise means skip this one value and carry on; it never means stop.
#[repr(C)]
pub struct Value {
    /// One of the `GUATIAO_*` tag constants.
    pub tag: u32,
    /// Reserved. Always written as zero, so the whole node is
    /// byte-comparable and a C caller has a named field to initialise.
    pub _pad: u32,
    /// The payload the tag selects.
    pub payload: Payload,
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

impl Value {
    /// The allocator this tree grows through.
    ///
    /// **An owned container records the allocator that made it**, which
    /// is why nothing below has to be told one: a write into this tree
    /// allocates storage *for this tree*, and its own allocator is the
    /// only right answer.
    ///
    /// Errors for a value that carries no allocator at all: a scalar, or
    /// a literal some other language built by hand. Growing one of those
    /// means saying which allocator to adopt, which is what the raw
    /// [`map_set`] and [`list_push`] still take.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        let stored = alloc_of(self).ok_or(ValueError::WrongKind)?;
        // SAFETY: the address a container recorded is an allocator that
        // outlives it, by the contract on `Alloc`.
        Ok(unsafe { Alloc::from_raw(stored) }?)
    }

    /// Stores `value` under `key`, **consuming** it.
    ///
    /// Takes anything a value can be made from, so the common case is one
    /// call and no allocator:
    ///
    /// ```
    /// # use guatiao::{Map, Value};
    /// let mut map = Value::map();
    /// map.set("host", "10.0.0.1")?;
    /// map.set("port", 5900)?;
    /// map.set("options", Value::map())?;
    /// # Ok::<(), guatiao::ValueError>(())
    /// ```
    ///
    /// Safe, and that is the point: both sides are well-formed trees by
    /// construction and the one being stored is consumed here, so neither
    /// the malformed-input nor the double-owner case a raw call has to
    /// worry about can arise.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        let mut value = value.into();
        // SAFETY: `self` and `value` are both well-formed trees built
        // through allocators that outlive them, and `value` is consumed
        // here so nothing else refers to what it owned.
        unsafe { map_set(self, key, &mut value, alloc) }
    }

    /// Appends `value`, **consuming** it. Takes anything [`set`] does.
    ///
    /// [`set`]: Value::set
    pub fn push(&mut self, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        let mut value = value.into();
        // SAFETY: as for `set`.
        unsafe { list_push(self, &mut value, alloc) }
    }

    /// Appends `value` to the list under `key`, creating the list when
    /// there is none.
    ///
    /// The operation every nested structure needs, and the reason it is
    /// here rather than left to a caller: without it, building a list of
    /// maps means reaching into a borrowed node with the raw functions,
    /// which is exactly the `unsafe` this type exists to remove.
    pub fn push_into(&mut self, key: &str, value: impl Into<Value>) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        if !self.contains_key(key) {
            // A LIST, not a map. The key names a sequence being appended
            // to — getting this wrong builds a map that the push below
            // then refuses, one call after the mistake.
            self.set(key, Value::list_in(alloc))?;
        }
        let mut value = value.into();
        let list = self.get_mut(key).ok_or(ValueError::WrongKind)?;
        // SAFETY: `list` is the list ensured immediately above, and
        // `value` is a well-formed tree consumed here, so nothing else
        // refers to what it owned.
        unsafe { list_push(list, &mut value, alloc) }
    }

    /// The value under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        map_get(self, key)
    }

    /// The value under `key`, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        map_get_mut(self, key)
    }

    /// Whether `key` is present.
    ///
    /// Named as the standard library names it, because it answers the
    /// same question and a second name for one idea is one more thing to
    /// look up.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

impl Value {
    /// The stored nothing. Owns nothing, so it needs no allocator.
    pub fn null() -> Value {
        value_null()
    }

    /// The absent sentinel: a value that says there is no value, which is
    /// a different statement from null.
    pub fn absent() -> Value {
        value_absent()
    }

    /// A boolean. Owns nothing.
    pub fn bool(b: bool) -> Value {
        value_bool(b)
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

    /// A number from its exact text, stored verbatim.
    ///
    /// Fallible even in the short form, and the refusal is about the
    /// **text**, not the memory: `"1,5"` is not a JSON number, and saying
    /// so here puts the error at the mistake.
    pub fn number(text: &str) -> Result<Value, ValueError> {
        Value::number_in(Alloc::rust(), text)
    }

    /// The same, through an allocator you name.
    pub fn number_in(alloc: Alloc, text: &str) -> Result<Value, ValueError> {
        value_number(alloc, text)
    }

    /// A number from an integer. Every `i64` has a JSON spelling, so this
    /// cannot be refused.
    pub fn int(v: i64) -> Value {
        or_abort(Value::int_in(Alloc::rust(), v))
    }

    /// The same, through an allocator you name.
    pub fn int_in(alloc: Alloc, v: i64) -> Result<Value, ValueError> {
        value_int(alloc, v)
    }

    /// A number from a float, refusing one with no JSON spelling.
    ///
    /// Fallible for the same reason [`number`](Value::number) is: `NaN`
    /// and the infinities are not numbers this format can hold, and that
    /// is a fact about the argument rather than about the heap.
    pub fn float(v: f64) -> Result<Value, ValueError> {
        Value::float_in(Alloc::rust(), v)
    }

    /// The same, through an allocator you name.
    pub fn float_in(alloc: Alloc, v: f64) -> Result<Value, ValueError> {
        value_float(alloc, v)
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
        value_tag(self)
    }

    /// The boolean, or `None` for any other kind.
    pub fn as_bool(&self) -> Option<bool> {
        as_bool(self)
    }

    /// The text of a string value.
    pub fn as_str(&self) -> Option<&str> {
        str_of(self)
    }

    /// The bytes of a bytes value. Any content at all, NULs included.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        bytes_of(self)
    }

    /// A number's **exact text**, as it was written down.
    ///
    /// Numbers are text here, so `1.10` and `1.1` are different values and
    /// `u64::MAX` survives. Convert with `TryInto` when you want a machine
    /// width, and the refusal is then visible.
    pub fn as_number_str(&self) -> Option<&str> {
        number_str(self)
    }

    /// The map container, or `None` for any other kind.
    pub fn as_map(&self) -> Option<&Map> {
        as_map(self)
    }

    /// The same, mutably.
    pub fn as_map_mut(&mut self) -> Option<&mut Map> {
        as_map_mut(self)
    }

    /// The list container, or `None` for any other kind.
    pub fn as_list(&self) -> Option<&List> {
        as_list(self)
    }

    /// The same, mutably.
    pub fn as_list_mut(&mut self) -> Option<&mut List> {
        as_list_mut(self)
    }

    /// A map's entries, in insertion order, which is part of the contract.
    pub fn entries(&self) -> Option<&[Entry]> {
        entries_of(self)
    }

    /// A list's elements, in order.
    pub fn items(&self) -> Option<&[Value]> {
        items_of(self)
    }

    /// Removes `key` from a map and hands back its value.
    ///
    /// The value frees itself when it goes out of scope, so dropping it
    /// on the floor is a release rather than a leak.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        // SAFETY: a `&mut Value` in safe code is a well-formed node.
        unsafe { map_remove(self, key) }
    }

    /// Removes `key` and frees its value. Answers whether it was there.
    pub fn discard(&mut self, key: &str) -> bool {
        // SAFETY: as above.
        unsafe { map_discard(self, key) }
    }

    /// Removes the element at `index` from a list, keeping the order of
    /// the rest.
    pub fn remove_at(&mut self, index: usize) -> Option<Value> {
        // SAFETY: as above.
        unsafe { list_remove(self, index) }
    }

    /// Removes and frees the element at `index`.
    pub fn discard_at(&mut self, index: usize) -> bool {
        // SAFETY: as above.
        unsafe { list_discard(self, index) }
    }

    /// Frees every entry or element, **keeping the capacity** already
    /// paid for. Errors for a kind that holds neither.
    pub fn clear(&mut self) -> Result<(), ValueError> {
        // SAFETY: as above.
        match Tag::try_from(self.tag) {
            Ok(Tag::GUATIAO_MAP) => unsafe { map_clear(self) },
            Ok(Tag::GUATIAO_LIST) => unsafe { list_clear(self) },
            _ => Err(ValueError::WrongKind),
        }
    }

    /// A deep copy, built through `alloc`.
    ///
    /// Explicit, and deliberately so: an owned tree carries its allocator,
    /// so copying one into another heap costs a walk, and that cost should
    /// be a line a reader can see rather than something a setter does
    /// quietly.
    ///
    /// # Safety
    ///
    /// This value is a well-formed node.
    pub unsafe fn clone_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        // SAFETY: forwarded.
        unsafe { value_clone(alloc, self) }
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
        // SAFETY: forwarded.
        unsafe { map_set(self, key, value, alloc) }
    }

    /// Appends `value`, **moving** it. See [`set_in`](Value::set_in).
    ///
    /// # Safety
    ///
    /// As for [`set_in`](Value::set_in).
    pub unsafe fn push_in(&mut self, value: &mut Value, alloc: Alloc) -> Result<(), ValueError> {
        // SAFETY: forwarded.
        unsafe { list_push(self, value, alloc) }
    }

    /// Copies every entry of `src` into this map, replacing keys that
    /// collide and appending the rest. Answers how many were copied.
    ///
    /// # Safety
    ///
    /// Both nodes are well formed.
    pub unsafe fn copy_from(&mut self, src: &Value, alloc: Alloc) -> Result<usize, ValueError> {
        // SAFETY: forwarded.
        unsafe { map_copy_from(self, src, alloc) }
    }

    /// Appends to a string value in place.
    ///
    /// # Safety
    ///
    /// This value is a well-formed string.
    pub unsafe fn push_str(&mut self, text: &str, alloc: Alloc) -> Result<(), ValueError> {
        // SAFETY: forwarded.
        unsafe { string_push(self, text, alloc) }
    }

    /// Appends to a bytes value in place.
    ///
    /// # Safety
    ///
    /// This value is a well-formed bytes value.
    pub unsafe fn push_bytes(&mut self, bytes: &[u8], alloc: Alloc) -> Result<(), ValueError> {
        // SAFETY: forwarded.
        unsafe { buffer_push(self, bytes, alloc) }
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
        // SAFETY: forwarded.
        unsafe { value_free(self) }
    }
}

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
        // SAFETY: a `Value` reaching Rust either was built through an
        // allocator that outlives it, which is the contract on `Alloc`,
        // or owns nothing at all.
        unsafe { value_free(self) }
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Value {
        Value::string(text)
    }
}

impl From<String> for Value {
    fn from(text: String) -> Value {
        Value::string(&text)
    }
}

impl From<&String> for Value {
    fn from(text: &String) -> Value {
        Value::string(text)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::bool(b)
    }
}

/// A float converts **fallibly**, and that is not an oversight.
///
/// `NaN` and the infinities have no JSON spelling, so `f64` cannot promise
/// what `From` promises. `map.set("ratio", Value::float(x)?)` says out
/// loud that the value might not be one.
impl TryFrom<f64> for Value {
    type Error = ValueError;

    fn try_from(v: f64) -> Result<Value, ValueError> {
        Value::float(v)
    }
}

impl TryFrom<f32> for Value {
    type Error = ValueError;

    fn try_from(v: f32) -> Result<Value, ValueError> {
        Value::float(f64::from(v))
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

/// Every integer width, written as its **decimal text**.
///
/// Not squeezed through an `i64` on the way: a number is the exact text
/// that declared it, so `u64::MAX` and `i128::MIN` cross as themselves
/// rather than wrapping. The text form is what makes that free — there is
/// no machine width at the boundary to overflow.
macro_rules! integer_from {
    ($($t:ty),* $(,)?) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Value {
                or_abort(value_number(Alloc::rust(), &v.to_string()))
            }
        }
    )*};
}

integer_from!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
