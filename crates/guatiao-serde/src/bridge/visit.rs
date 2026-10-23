// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Any serde type walked into a guatiao visitor: a serializer whose
//! calls are the visitor's events.
//!
//! A struct is a map and a sequence a list, each element handed to the
//! visitor as something that walks itself through its own `Serialize`.
//! A variant with content is a map of one key, its name; a tuple or
//! struct variant's content is collected into a value first, since serde
//! hands its fields one call at a time and a visitor takes a map entry's
//! value whole. A float is its shortest text, keeping a fraction (`1.0`
//! stays `1.0`); a number serde_json carries as its own text, and a TOML
//! date or time, arrive as that text.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use core::fmt;

use serde::ser::{self, Serialize};

use guatiao::value::visit::{Flat, ListVisitor, MapVisitor, TreeSink, Visitor, Walk};
use guatiao::{MAX_DEPTH, Value, ValueError};

use super::number::Text;
use crate::de::{RAW_NUMBER, TOML_DATETIME};

/// A serde value that walks itself, `depth` containers down.
pub(crate) struct SerdeWalk<'a, T: ?Sized> {
    value: &'a T,
    depth: u32,
}

impl<'a, T: ?Sized> SerdeWalk<'a, T> {
    pub(crate) fn new(value: &'a T) -> SerdeWalk<'a, T> {
        SerdeWalk { value, depth: 0 }
    }
}

impl<T: Serialize + ?Sized> Walk for SerdeWalk<'_, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        self.value
            .serialize(ToVisitor {
                visitor,
                depth: self.depth,
            })
            .map_err(|e| e.0)
    }
}

/// A visitor's refusal, carried through serde. A serde type's own error
/// has no message to carry into a visitor, so it is
/// [`ValueError::Unrepresentable`].
pub(crate) struct Refused<E>(pub(crate) E);

impl<E> fmt::Debug for Refused<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Refused")
    }
}

impl<E> fmt::Display for Refused<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the visitor refused a value")
    }
}

impl<E> core::error::Error for Refused<E> {}

impl<E: From<ValueError>> ser::Error for Refused<E> {
    fn custom<M: fmt::Display>(_: M) -> Refused<E> {
        Refused(E::from(ValueError::Unrepresentable))
    }
}

fn refused<E: From<ValueError>>(e: ValueError) -> Refused<E> {
    Refused(E::from(e))
}

/// Refuses a container opened past [`MAX_DEPTH`].
fn inside<E: From<ValueError>>(depth: u32) -> Result<u32, Refused<E>> {
    if depth >= MAX_DEPTH {
        return Err(refused(ValueError::TooDeep));
    }
    Ok(depth + 1)
}

/// A serializer whose calls are `visitor`'s events.
struct ToVisitor<V> {
    visitor: V,
    depth: u32,
}

impl<V: Visitor> ToVisitor<V> {
    fn shortest(self, text: Option<Text>) -> Result<V::Ok, Refused<V::Error>> {
        let text = text.ok_or_else(|| refused(ValueError::NotANumber))?;
        self.visitor.number(text.as_str()).map_err(Refused)
    }

    /// `{variant: content}`, serde's external tagging.
    fn tagged<W: Walk + ?Sized>(
        self,
        variant: &str,
        content: &W,
    ) -> Result<V::Ok, Refused<V::Error>> {
        let mut map = self.visitor.map(Some(1)).map_err(Refused)?;
        map.entry(variant, content).map_err(Refused)?;
        map.end().map_err(Refused)
    }
}

impl<V: Visitor> ser::Serializer for ToVisitor<V> {
    type Ok = V::Ok;
    type Error = Refused<V::Error>;
    type SerializeSeq = Items<V::List>;
    type SerializeTuple = Items<V::List>;
    type SerializeTupleStruct = Items<V::List>;
    type SerializeTupleVariant = Content<V>;
    type SerializeMap = Fields<V>;
    type SerializeStruct = Fields<V>;
    type SerializeStructVariant = Content<V>;

    fn serialize_bool(self, v: bool) -> Result<V::Ok, Self::Error> {
        self.visitor.bool(v).map_err(Refused)
    }

    fn serialize_i8(self, v: i8) -> Result<V::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    fn serialize_i16(self, v: i16) -> Result<V::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    fn serialize_i32(self, v: i32) -> Result<V::Ok, Self::Error> {
        self.serialize_i64(v.into())
    }

    fn serialize_i64(self, v: i64) -> Result<V::Ok, Self::Error> {
        self.visitor.i64(v).map_err(Refused)
    }

    fn serialize_i128(self, v: i128) -> Result<V::Ok, Self::Error> {
        self.visitor
            .number(Text::of(format_args!("{v}")).as_str())
            .map_err(Refused)
    }

    fn serialize_u8(self, v: u8) -> Result<V::Ok, Self::Error> {
        self.serialize_u64(v.into())
    }

    fn serialize_u16(self, v: u16) -> Result<V::Ok, Self::Error> {
        self.serialize_u64(v.into())
    }

    fn serialize_u32(self, v: u32) -> Result<V::Ok, Self::Error> {
        self.serialize_u64(v.into())
    }

    fn serialize_u64(self, v: u64) -> Result<V::Ok, Self::Error> {
        self.visitor.u64(v).map_err(Refused)
    }

    fn serialize_u128(self, v: u128) -> Result<V::Ok, Self::Error> {
        self.visitor
            .number(Text::of(format_args!("{v}")).as_str())
            .map_err(Refused)
    }

    // The shortest text that reads back the same float, keeping `1.0` a
    // float where its `Display` would write the integer `1`. A float that
    // is not finite is no number.
    fn serialize_f32(self, v: f32) -> Result<V::Ok, Self::Error> {
        self.shortest(v.is_finite().then(|| Text::of(format_args!("{v:?}"))))
    }

    fn serialize_f64(self, v: f64) -> Result<V::Ok, Self::Error> {
        self.shortest(v.is_finite().then(|| Text::of(format_args!("{v:?}"))))
    }

    fn serialize_char(self, v: char) -> Result<V::Ok, Self::Error> {
        self.visitor
            .string(v.encode_utf8(&mut [0; 4]))
            .map_err(Refused)
    }

    fn serialize_str(self, v: &str) -> Result<V::Ok, Self::Error> {
        self.visitor.string(v).map_err(Refused)
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<V::Ok, Self::Error> {
        self.visitor.bytes(v).map_err(Refused)
    }

    fn serialize_none(self) -> Result<V::Ok, Self::Error> {
        self.visitor.null().map_err(Refused)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<V::Ok, Self::Error> {
        v.serialize(self)
    }

    fn serialize_unit(self) -> Result<V::Ok, Self::Error> {
        self.visitor.null().map_err(Refused)
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<V::Ok, Self::Error> {
        self.visitor.null().map_err(Refused)
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<V::Ok, Self::Error> {
        self.visitor.string(variant).map_err(Refused)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<V::Ok, Self::Error> {
        v.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<V::Ok, Self::Error> {
        let depth = inside(inside(self.depth)?)?;
        self.tagged(variant, &SerdeWalk { value: v, depth })
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Items<V::List>, Self::Error> {
        let depth = inside(self.depth)?;
        Ok(Items {
            list: self.visitor.list(len).map_err(Refused)?,
            depth,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Items<V::List>, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<Items<V::List>, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Content<V>, Self::Error> {
        Content::open(self, variant, false, len)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Fields<V>, Self::Error> {
        Ok(Fields {
            depth: inside(self.depth)?,
            state: State::Waiting(self.visitor),
            len,
            key: String::new(),
        })
    }

    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Fields<V>, Self::Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Content<V>, Self::Error> {
        Content::open(self, variant, true, len)
    }
}

/// A list being written.
pub(crate) struct Items<L> {
    list: L,
    depth: u32,
}

impl<L: ListVisitor> Items<L>
where
    L::Error: From<ValueError>,
{
    fn push<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Refused<L::Error>> {
        self.list
            .item(&SerdeWalk {
                value: v,
                depth: self.depth,
            })
            .map_err(Refused)
    }

    fn finish(self) -> Result<L::Ok, Refused<L::Error>> {
        self.list.end().map_err(Refused)
    }
}

macro_rules! items {
    ($($trait:ident :: $method:ident),*) => {$(
        impl<L: ListVisitor> ser::$trait for Items<L>
        where
            L::Error: From<ValueError>,
        {
            type Ok = L::Ok;
            type Error = Refused<L::Error>;

            fn $method<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
                self.push(v)
            }

            fn end(self) -> Result<L::Ok, Self::Error> {
                self.finish()
            }
        }
    )*};
}

items!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field
);

/// A map or a struct being written. The visitor's map opens at the first
/// entry, so a one-field struct carrying a number's or a date's text
/// under a reserved key can be that number or that text instead.
pub(crate) struct Fields<V: Visitor> {
    depth: u32,
    state: State<V>,
    len: Option<usize>,
    /// A map's key, between `serialize_key` and `serialize_value`.
    key: String,
}

enum State<V: Visitor> {
    Waiting(V),
    Open(V::Map),
    /// A reserved field was the value itself.
    Done(V::Ok),
    /// Between states, never seen outside a call.
    Gone,
}

impl<V: Visitor> Fields<V> {
    fn put<T: Serialize + ?Sized>(&mut self, key: &str, v: &T) -> Result<(), Refused<V::Error>> {
        if let State::Waiting(_) = self.state
            && self.len == Some(1)
            && (key == RAW_NUMBER || key == TOML_DATETIME)
        {
            let State::Waiting(visitor) = core::mem::replace(&mut self.state, State::Gone) else {
                unreachable!("matched just above");
            };
            let text = v.serialize(TextOnly).map_err(|e| refused(e.0))?;
            let done = if key == RAW_NUMBER {
                visitor.number(&text)
            } else {
                visitor.string(&text)
            };
            self.state = State::Done(done.map_err(Refused)?);
            return Ok(());
        }
        if let State::Waiting(_) = self.state {
            let State::Waiting(visitor) = core::mem::replace(&mut self.state, State::Gone) else {
                unreachable!("matched just above");
            };
            self.state = State::Open(visitor.map(self.len).map_err(Refused)?);
        }
        let depth = self.depth;
        match &mut self.state {
            State::Open(map) => map
                .entry(key, &SerdeWalk { value: v, depth })
                .map_err(Refused),
            _ => Err(refused(ValueError::WrongKind)),
        }
    }

    fn finish(self) -> Result<V::Ok, Refused<V::Error>> {
        match self.state {
            State::Waiting(visitor) => visitor
                .map(self.len)
                .and_then(MapVisitor::end)
                .map_err(Refused),
            State::Open(map) => map.end().map_err(Refused),
            State::Done(done) => Ok(done),
            State::Gone => Err(refused(ValueError::WrongKind)),
        }
    }
}

impl<V: Visitor> ser::SerializeMap for Fields<V> {
    type Ok = V::Ok;
    type Error = Refused<V::Error>;

    // A key is text; a number or a character key crosses as its text, as
    // `serde_json` writes one.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.key = key.serialize(KeyText).map_err(|e| refused(e.0))?;
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        let key = core::mem::take(&mut self.key);
        self.put(&key, v)
    }

    fn end(self) -> Result<V::Ok, Self::Error> {
        self.finish()
    }
}

impl<V: Visitor> ser::SerializeStruct for Fields<V> {
    type Ok = V::Ok;
    type Error = Refused<V::Error>;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        self.put(key, v)
    }

    fn end(self) -> Result<V::Ok, Self::Error> {
        self.finish()
    }
}

/// A tuple or struct variant's content, collected into a value, then
/// written under the variant's name.
pub(crate) struct Content<V> {
    outer: ToVisitor<V>,
    variant: &'static str,
    sink: TreeSink,
    depth: u32,
    map: bool,
}

impl<V: Visitor> Content<V> {
    fn open(
        outer: ToVisitor<V>,
        variant: &'static str,
        map: bool,
        len: usize,
    ) -> Result<Content<V>, Refused<V::Error>> {
        let depth = inside(inside(outer.depth)?)?;
        let mut sink = TreeSink::new();
        let opened = if map {
            guatiao::value::visit::Sink::map_begin(&mut sink, Some(len))
        } else {
            guatiao::value::visit::Sink::list_begin(&mut sink, Some(len))
        };
        opened.map_err(refused)?;
        Ok(Content {
            outer,
            variant,
            sink,
            depth,
            map,
        })
    }

    fn put<T: Serialize + ?Sized>(
        &mut self,
        key: Option<&str>,
        v: &T,
    ) -> Result<(), Refused<V::Error>> {
        if let Some(key) = key {
            guatiao::value::visit::Sink::key(&mut self.sink, key).map_err(refused)?;
        }
        let walk = SerdeWalk {
            value: v,
            depth: self.depth,
        };
        walk.walk(Flat(&mut self.sink)).map_err(refused)
    }

    fn finish(mut self) -> Result<V::Ok, Refused<V::Error>> {
        let closed = if self.map {
            guatiao::value::visit::Sink::map_end(&mut self.sink)
        } else {
            guatiao::value::visit::Sink::list_end(&mut self.sink)
        };
        closed.map_err(refused)?;
        let content: Value = self.sink.into_value().map_err(refused)?;
        // The content was built inside; its depth is already counted.
        self.outer.tagged(self.variant, &content)
    }
}

impl<V: Visitor> ser::SerializeTupleVariant for Content<V> {
    type Ok = V::Ok;
    type Error = Refused<V::Error>;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.put(None, v)
    }

    fn end(self) -> Result<V::Ok, Self::Error> {
        self.finish()
    }
}

impl<V: Visitor> ser::SerializeStructVariant for Content<V> {
    type Ok = V::Ok;
    type Error = Refused<V::Error>;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        self.put(Some(key), v)
    }

    fn end(self) -> Result<V::Ok, Self::Error> {
        self.finish()
    }
}

/// What a map key is: text, or a number or a character as its text.
struct KeyText;

/// The text under a reserved key: a string, and only that.
struct TextOnly;

/// A serializer that answers text and refuses every other shape with
/// `ValueError::WrongKind`.
macro_rules! text_serializer {
    ($ty:ident, $number:expr) => {
        impl ser::Serializer for $ty {
            type Ok = String;
            type Error = Refused<ValueError>;
            type SerializeSeq = ser::Impossible<String, Refused<ValueError>>;
            type SerializeTuple = ser::Impossible<String, Refused<ValueError>>;
            type SerializeTupleStruct = ser::Impossible<String, Refused<ValueError>>;
            type SerializeTupleVariant = ser::Impossible<String, Refused<ValueError>>;
            type SerializeMap = ser::Impossible<String, Refused<ValueError>>;
            type SerializeStruct = ser::Impossible<String, Refused<ValueError>>;
            type SerializeStructVariant = ser::Impossible<String, Refused<ValueError>>;

            fn serialize_str(self, v: &str) -> Result<String, Self::Error> {
                Ok(v.to_string())
            }

            fn serialize_char(self, v: char) -> Result<String, Self::Error> {
                if $number {
                    Ok(v.to_string())
                } else {
                    Err(wrong())
                }
            }

            fn serialize_i64(self, v: i64) -> Result<String, Self::Error> {
                if $number {
                    Ok(v.to_string())
                } else {
                    Err(wrong())
                }
            }

            fn serialize_u64(self, v: u64) -> Result<String, Self::Error> {
                if $number {
                    Ok(v.to_string())
                } else {
                    Err(wrong())
                }
            }

            fn serialize_i128(self, v: i128) -> Result<String, Self::Error> {
                if $number {
                    Ok(v.to_string())
                } else {
                    Err(wrong())
                }
            }

            fn serialize_u128(self, v: u128) -> Result<String, Self::Error> {
                if $number {
                    Ok(v.to_string())
                } else {
                    Err(wrong())
                }
            }

            fn serialize_i8(self, v: i8) -> Result<String, Self::Error> {
                self.serialize_i64(v.into())
            }

            fn serialize_i16(self, v: i16) -> Result<String, Self::Error> {
                self.serialize_i64(v.into())
            }

            fn serialize_i32(self, v: i32) -> Result<String, Self::Error> {
                self.serialize_i64(v.into())
            }

            fn serialize_u8(self, v: u8) -> Result<String, Self::Error> {
                self.serialize_u64(v.into())
            }

            fn serialize_u16(self, v: u16) -> Result<String, Self::Error> {
                self.serialize_u64(v.into())
            }

            fn serialize_u32(self, v: u32) -> Result<String, Self::Error> {
                self.serialize_u64(v.into())
            }

            fn serialize_bool(self, _: bool) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_f32(self, _: f32) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_f64(self, _: f64) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_bytes(self, _: &[u8]) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_none(self) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<String, Self::Error> {
                v.serialize(self)
            }

            fn serialize_unit(self) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_unit_struct(self, _: &'static str) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_unit_variant(
                self,
                _: &'static str,
                _: u32,
                variant: &'static str,
            ) -> Result<String, Self::Error> {
                Ok(variant.to_string())
            }

            fn serialize_newtype_struct<T: Serialize + ?Sized>(
                self,
                _: &'static str,
                v: &T,
            ) -> Result<String, Self::Error> {
                v.serialize(self)
            }

            fn serialize_newtype_variant<T: Serialize + ?Sized>(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: &T,
            ) -> Result<String, Self::Error> {
                Err(wrong())
            }

            fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
                Err(wrong())
            }

            fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
                Err(wrong())
            }

            fn serialize_tuple_struct(
                self,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleStruct, Self::Error> {
                Err(wrong())
            }

            fn serialize_tuple_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleVariant, Self::Error> {
                Err(wrong())
            }

            fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
                Err(wrong())
            }

            fn serialize_struct(
                self,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeStruct, Self::Error> {
                Err(wrong())
            }

            fn serialize_struct_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeStructVariant, Self::Error> {
                Err(wrong())
            }
        }
    };
}

fn wrong() -> Refused<ValueError> {
    Refused(ValueError::WrongKind)
}

text_serializer!(KeyText, true);
text_serializer!(TextOnly, false);
