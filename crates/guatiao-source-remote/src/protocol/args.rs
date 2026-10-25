// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A call's arguments and a reply's answer, typed by the slot's
//! description: nothing on the wire repeats what both sides know.
//!
//! ```text
//! bool            1 byte, 0 or 1
//! i8..i64, isize  int (zigzag LEB128)
//! u8..u64, usize  uint (LEB128)
//! f32, f64        4 or 8 bytes, little-endian IEEE 754
//! str, text       text           bytes  bytes
//! bytes_mut       uint: its length; the reply writes back bytes
//! value/map/list  value (a nullable value that is absent: ABSENT)
//! object          uint: id << 1 | 1 when the receiver's own, else 0
//!
//! reply body := answer? writeback*   one writeback per lent tree (value)
//!                                     and per bytes_mut (bytes), in order
//! ```

use guatiao::library::kind::{
    Answer, AnswerType, Arg, ArgType, Pass, RawObject, Scalar, SlotDescription, Tree,
};
use guatiao::{Alloc, List, Map, Value};

use super::ProtocolError;
use super::codec::{Reader, put_bytes, put_int, put_text, put_uint, put_value, put_walk};

/// An object argument or answer as it crosses: whose numbering its id is
/// in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRef {
    /// An object the sender holds: the receiver calls it through a proxy.
    Senders(u64),
    /// An object of the receiver's own, passed back.
    Receivers(u64),
}

impl ObjectRef {
    pub(crate) fn code(self) -> u64 {
        match self {
            ObjectRef::Senders(id) => id << 1,
            ObjectRef::Receivers(id) => (id << 1) | 1,
        }
    }

    pub(crate) fn from_code(code: u64) -> ObjectRef {
        if code & 1 == 0 {
            ObjectRef::Senders(code >> 1)
        } else {
            ObjectRef::Receivers(code >> 1)
        }
    }
}

/// An argument as the receiver read it: owned where it must be, borrowed
/// from the frame where it can be.
#[derive(Debug)]
#[non_exhaustive]
pub enum Decoded<'a> {
    /// A flag.
    Bool(bool),
    /// Any signed integer, widened.
    Int(i64),
    /// Any unsigned integer, widened.
    Uint(u64),
    /// Either float, widened.
    Float(f64),
    /// Text, borrowed from the frame.
    Str(&'a str),
    /// Bytes, borrowed from the frame.
    Bytes(&'a [u8]),
    /// The caller's buffer to write into, as long as it was.
    Buffer(Vec<u8>),
    /// A value; `None` for a nullable one that was absent.
    Value(Option<Value>),
    /// A map.
    Map(Map),
    /// A list.
    List(List),
    /// An object, by its reference.
    Object(ObjectRef),
}

fn mismatch(i: usize, slot: &SlotDescription, what: &str) -> ProtocolError {
    let _ = slot;
    ProtocolError::Value(format!("argument {i} is not {what}, which the slot takes"))
}

fn put_scalar_int(out: &mut Vec<u8>, s: Scalar, n: i128) -> bool {
    match s {
        Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64 | Scalar::Isize => {
            match i64::try_from(n) {
                Ok(n) => put_int(out, n),
                Err(_) => return false,
            }
        }
        Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize => {
            match u64::try_from(n) {
                Ok(n) => put_uint(out, n),
                Err(_) => return false,
            }
        }
        _ => return false,
    }
    true
}

fn put_scalar_float(out: &mut Vec<u8>, s: Scalar, f: f64) -> bool {
    match s {
        Scalar::F32 => out.extend_from_slice(&(f as f32).to_le_bytes()),
        Scalar::F64 => out.extend_from_slice(&f.to_le_bytes()),
        _ => return false,
    }
    true
}

fn get_scalar(r: &mut Reader<'_>, s: Scalar) -> Result<Decoded<'static>, ProtocolError> {
    Ok(match s {
        Scalar::Bool => {
            let at = r.at();
            match r.byte()? {
                0 => Decoded::Bool(false),
                1 => Decoded::Bool(true),
                _ => return Err(ProtocolError::Field { at, why: "a bool" }),
            }
        }
        Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::I64 | Scalar::Isize => {
            Decoded::Int(r.int()?)
        }
        Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize => {
            Decoded::Uint(r.uint()?)
        }
        Scalar::F32 => Decoded::Float(f32::from_le_bytes(r.fixed::<4>()?).into()),
        Scalar::F64 => Decoded::Float(f64::from_le_bytes(r.fixed::<8>()?)),
        _ => {
            return Err(ProtocolError::Field {
                at: r.at(),
                why: "a scalar these tables carry",
            });
        }
    })
}

fn put_tree_value(out: &mut Vec<u8>, v: &Value) -> Result<(), ProtocolError> {
    put_value(out, v)
}

/// Appends `args` as `slot` types them. `refs[i]` is the reference the
/// caller gave the object at position `i`, which the link registered.
pub fn encode_args(
    slot: &SlotDescription,
    args: &[Arg<'_>],
    refs: &[Option<ObjectRef>],
    out: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    if args.len() != slot.args().len() {
        return Err(ProtocolError::Value(format!(
            "{} arguments for a slot that takes {}",
            args.len(),
            slot.args().len()
        )));
    }
    for (i, (arg, desc)) in args.iter().zip(slot.args()).enumerate() {
        match (desc.ty(), arg) {
            (ArgType::Scalar(Scalar::Bool), Arg::Bool(b)) => out.push(u8::from(*b)),
            (ArgType::Scalar(s), Arg::Int(n)) if put_scalar_int(out, *s, i128::from(*n)) => {}
            (ArgType::Scalar(s), Arg::Uint(n)) if put_scalar_int(out, *s, i128::from(*n)) => {}
            (ArgType::Scalar(s), Arg::Float(f)) if put_scalar_float(out, *s, *f) => {}
            (ArgType::Str, Arg::Str(s)) => put_text(out, s),
            (ArgType::Bytes, Arg::Bytes(b)) => put_bytes(out, b),
            (ArgType::BytesMut, Arg::Buffer(b)) => put_uint(out, b.len() as u64),
            (ArgType::Tree { .. }, Arg::Value(None)) => put_tree_value(out, &Value::absent())?,
            (ArgType::Tree { .. }, Arg::Value(Some(v))) => put_tree_value(out, v)?,
            (ArgType::Tree { .. }, Arg::TakenValue(v)) => put_tree_value(out, v)?,
            (ArgType::Tree { .. }, Arg::LentValue(v)) => put_tree_value(out, v)?,
            (ArgType::Tree { .. }, Arg::Map(m)) => put_walk(out, *m)?,
            (ArgType::Tree { .. }, Arg::TakenMap(m)) => put_walk(out, m)?,
            (ArgType::Tree { .. }, Arg::LentMap(m)) => put_walk(out, &**m)?,
            (ArgType::Tree { .. }, Arg::List(l)) => put_walk(out, *l)?,
            (ArgType::Tree { .. }, Arg::TakenList(l)) => put_walk(out, l)?,
            (ArgType::Tree { .. }, Arg::LentList(l)) => put_walk(out, &**l)?,
            (ArgType::Object { .. }, _) => match refs.get(i).copied().flatten() {
                Some(r) => put_uint(out, r.code()),
                None => return Err(mismatch(i, slot, "an object")),
            },
            (ty, _) => return Err(mismatch(i, slot, type_name(ty))),
        }
    }
    Ok(())
}

fn type_name(ty: &ArgType) -> &'static str {
    match ty {
        ArgType::Scalar(_) => "a scalar of the slot's type",
        ArgType::Str => "text",
        ArgType::Bytes => "bytes",
        ArgType::BytesMut => "a buffer",
        ArgType::Tree { .. } => "a tree",
        ArgType::Object { .. } => "an object",
        _ => "of a type these tables carry",
    }
}

/// Reads a CALL's arguments as `slot` types them. Trees are decoded into
/// `alloc`; text and bytes are borrowed from `bytes`.
pub fn decode_args<'a>(
    slot: &SlotDescription,
    bytes: &'a [u8],
    alloc: Alloc,
) -> Result<Vec<Decoded<'a>>, ProtocolError> {
    let mut r = Reader::new(bytes);
    let mut out = Vec::with_capacity(slot.args().len());
    for desc in slot.args() {
        out.push(match desc.ty() {
            ArgType::Scalar(s) => get_scalar(&mut r, *s)?,
            ArgType::Str => Decoded::Str(r.text()?),
            ArgType::Bytes => Decoded::Bytes(r.bytes()?),
            ArgType::BytesMut => {
                let at = r.at();
                let n = usize::try_from(r.uint()?)
                    .ok()
                    .filter(|&n| n <= MAX_BUFFER)
                    .ok_or(ProtocolError::Field {
                        at,
                        why: "a buffer length within the link's limit",
                    })?;
                Decoded::Buffer(vec![0; n])
            }
            ArgType::Tree { tree, nullable, .. } => {
                let at = r.at();
                let v = r.value(alloc)?;
                tree_of(*tree, *nullable, v, at)?
            }
            ArgType::Object { .. } => Decoded::Object(ObjectRef::from_code(r.uint()?)),
            _ => {
                return Err(ProtocolError::Field {
                    at: r.at(),
                    why: "an argument these tables carry",
                });
            }
        });
    }
    r.end()?;
    Ok(out)
}

/// The largest out-buffer a caller may ask a callee to fill: 64 MiB, the
/// largest frame a link carries by default.
pub const MAX_BUFFER: usize = 64 << 20;

fn tree_of(
    tree: Tree,
    nullable: bool,
    v: Value,
    at: usize,
) -> Result<Decoded<'static>, ProtocolError> {
    Ok(match tree {
        Tree::Map => {
            Decoded::Map(Map::try_from(v).map_err(|_| ProtocolError::Field { at, why: "a map" })?)
        }
        Tree::List => Decoded::List(
            List::try_from(v).map_err(|_| ProtocolError::Field { at, why: "a list" })?,
        ),
        _ if nullable && v.tag() == Ok(guatiao::Tag::GUATIAO_ABSENT) => Decoded::Value(None),
        _ => Decoded::Value(Some(v)),
    })
}

/// Appends what a call wrote into its lent trees and buffers, in order:
/// the end of a REPLY's body.
pub fn encode_writebacks(
    slot: &SlotDescription,
    args: &[Decoded<'_>],
    out: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    for (desc, arg) in slot.args().iter().zip(args) {
        match (desc.ty(), arg) {
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Decoded::Value(Some(v)),
            ) => put_value(out, v)?,
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Decoded::Map(m),
            ) => put_walk(out, m)?,
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Decoded::List(l),
            ) => put_walk(out, l)?,
            (ArgType::BytesMut, Decoded::Buffer(b)) => put_bytes(out, b),
            _ => {}
        }
    }
    Ok(())
}

/// Appends a slot's answer. An object answered is registered by `export`,
/// which gives its reference.
pub fn encode_answer(
    slot: &SlotDescription,
    answer: Answer,
    out: &mut Vec<u8>,
    export: &mut dyn FnMut(RawObject) -> ObjectRef,
) -> Result<(), ProtocolError> {
    let wrong = |what: &str| ProtocolError::Value(format!("the callee answered {what}"));
    match (slot.answer(), answer) {
        (None, Answer::Nothing) => {}
        (None, _) => return Err(wrong("something for a slot that answers nothing")),
        (Some(AnswerType::Scalar(Scalar::Bool)), Answer::Bool(b)) => out.push(u8::from(b)),
        (Some(AnswerType::Scalar(s)), Answer::Int(n)) if put_scalar_int(out, *s, n.into()) => {}
        (Some(AnswerType::Scalar(s)), Answer::Uint(n)) if put_scalar_int(out, *s, n.into()) => {}
        (Some(AnswerType::Scalar(s)), Answer::Float(f)) if put_scalar_float(out, *s, f) => {}
        (Some(AnswerType::Text), Answer::Text(t)) => put_text(out, &t),
        (Some(AnswerType::Tree(_)), Answer::Value(v)) => put_value(out, &v)?,
        (Some(AnswerType::Tree(_)), Answer::Map(m)) => put_walk(out, &m)?,
        (Some(AnswerType::Tree(_)), Answer::List(l)) => put_walk(out, &l)?,
        (Some(AnswerType::Object { .. }), Answer::Object(o)) => put_uint(out, export(o).code()),
        (Some(_), _) => return Err(wrong("something its slot does not")),
    }
    Ok(())
}

/// Reads a REPLY's answer, then writes what the callee wrote back into
/// the caller's lent trees and buffers. An object answered is turned into
/// the caller's handle by `object`.
pub fn decode_answer(
    slot: &SlotDescription,
    body: &[u8],
    args: &mut [Arg<'_>],
    alloc: Alloc,
    object: &mut dyn FnMut(&str, ObjectRef) -> Result<RawObject, ProtocolError>,
) -> Result<Answer, ProtocolError> {
    let mut r = Reader::new(body);
    let answer = match slot.answer() {
        None => Answer::Nothing,
        Some(AnswerType::Scalar(s)) => match get_scalar(&mut r, *s)? {
            Decoded::Bool(b) => Answer::Bool(b),
            Decoded::Int(n) => Answer::Int(n),
            Decoded::Uint(n) => Answer::Uint(n),
            Decoded::Float(f) => Answer::Float(f),
            _ => unreachable!("a scalar reads as a scalar"),
        },
        Some(AnswerType::Text) => Answer::Text(r.text()?.to_string()),
        Some(AnswerType::Tree(tree)) => {
            let at = r.at();
            match tree_of(*tree, false, r.value(alloc)?, at)? {
                Decoded::Map(m) => Answer::Map(m),
                Decoded::List(l) => Answer::List(l),
                Decoded::Value(v) => Answer::Value(v.unwrap_or_else(Value::absent)),
                _ => unreachable!("a tree reads as a tree"),
            }
        }
        Some(AnswerType::Object { kind }) => {
            Answer::Object(object(kind, ObjectRef::from_code(r.uint()?))?)
        }
        Some(_) => {
            return Err(ProtocolError::Field {
                at: 0,
                why: "an answer these tables carry",
            });
        }
    };
    for (desc, arg) in slot.args().iter().zip(args.iter_mut()) {
        match (desc.ty(), arg) {
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Arg::LentValue(v),
            ) => {
                let home = v.alloc().unwrap_or_else(|_| Alloc::rust());
                **v = r.value(home)?;
            }
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Arg::LentMap(m),
            ) => {
                let home = m.alloc().unwrap_or_else(|_| Alloc::rust());
                let at = r.at();
                **m = Map::try_from(r.value(home)?).map_err(|_| ProtocolError::Field {
                    at,
                    why: "the lent map written back",
                })?;
            }
            (
                ArgType::Tree {
                    pass: Pass::Lent, ..
                },
                Arg::LentList(l),
            ) => {
                let home = l.alloc().unwrap_or_else(|_| Alloc::rust());
                let at = r.at();
                **l = List::try_from(r.value(home)?).map_err(|_| ProtocolError::Field {
                    at,
                    why: "the lent list written back",
                })?;
            }
            (ArgType::BytesMut, Arg::Buffer(b)) => {
                let written = r.bytes()?;
                let n = written.len().min(b.len());
                b[..n].copy_from_slice(&written[..n]);
            }
            _ => {}
        }
    }
    r.end()?;
    Ok(answer)
}
