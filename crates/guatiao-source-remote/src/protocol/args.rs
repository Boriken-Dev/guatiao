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
use super::codec::{
    Reader, behind_length, put_bytes, put_int, put_text, put_uint, put_value, put_value_bare,
    put_walk, put_walk_bare,
};

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
    same_count(slot, args)?;
    put_args(slot, args, refs, out)
}

/// The size from which a last text or bytes argument is sent from where
/// it lies, behind the frame's head, rather than copied into the frame.
pub(crate) const TAIL: usize = 64 << 10;

/// As [`encode_args`], but a last text or bytes argument of [`TAIL`] bytes
/// or more is left out after its length: the frame's last bytes, for the
/// caller to send from where they lie.
pub(crate) fn encode_args_tail<'s>(
    slot: &SlotDescription,
    args: &'s [Arg<'_>],
    refs: &[Option<ObjectRef>],
    out: &mut Vec<u8>,
) -> Result<Option<&'s [u8]>, ProtocolError> {
    same_count(slot, args)?;
    let tail = match (slot.args().last().map(|d| d.ty()), args.last()) {
        (Some(ArgType::Str), Some(Arg::Str(s))) if s.len() >= TAIL => s.as_bytes(),
        (Some(ArgType::Bytes), Some(Arg::Bytes(b))) if b.len() >= TAIL => *b,
        _ => return put_args(slot, args, refs, out).map(|()| None),
    };
    put_args(slot, &args[..args.len() - 1], refs, out)?;
    put_uint(out, tail.len() as u64);
    Ok(Some(tail))
}

fn same_count(slot: &SlotDescription, args: &[Arg<'_>]) -> Result<(), ProtocolError> {
    if args.len() != slot.args().len() {
        return Err(ProtocolError::Value(format!(
            "{} arguments for a slot that takes {}",
            args.len(),
            slot.args().len()
        )));
    }
    Ok(())
}

/// Appends `args` as the first `args.len()` of `slot`'s arguments type
/// them.
fn put_args(
    slot: &SlotDescription,
    args: &[Arg<'_>],
    refs: &[Option<ObjectRef>],
    out: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
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
    for (i, (desc, arg)) in slot.args().iter().zip(args).enumerate() {
        if !written_back(desc.ty()) {
            continue;
        }
        match arg {
            Decoded::Value(Some(v)) => put_value(out, v)?,
            Decoded::Map(m) => put_walk(out, m)?,
            Decoded::List(l) => put_walk(out, l)?,
            Decoded::Buffer(b) => put_bytes(out, b),
            _ => return Err(mismatch(i, slot, "a tree or buffer to write back")),
        }
    }
    Ok(())
}

/// Whether an argument of type `ty` is written back after the answer: what
/// [`encode_writebacks`] writes, and what keeps a large answer inside its
/// frame. [`decode_answer`] reads the same arguments by their lent forms.
fn written_back(ty: &ArgType) -> bool {
    matches!(
        ty,
        ArgType::BytesMut
            | ArgType::Tree {
                pass: Pass::Lent,
                ..
            }
    )
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

/// Whether a call of `slot` writes anything back after its answer.
fn writes_back(slot: &SlotDescription) -> bool {
    slot.args().iter().any(|d| written_back(d.ty()))
}

/// As [`encode_answer`], but a tree answered for a slot that writes nothing
/// back, whose encoding is [`TAIL`] bytes or more, is left in `out` from
/// the position answered, without its length: the frame's last bytes,
/// behind a head the caller builds, so that nothing moves it.
pub(crate) fn encode_answer_tail(
    slot: &SlotDescription,
    answer: Answer,
    out: &mut Vec<u8>,
    export: &mut dyn FnMut(RawObject) -> ObjectRef,
) -> Result<Option<usize>, ProtocolError> {
    if !matches!(slot.answer(), Some(AnswerType::Tree(_))) || writes_back(slot) {
        return encode_answer(slot, answer, out, export).map(|()| None);
    }
    let at = out.len();
    match &answer {
        Answer::Value(v) => put_value_bare(out, v)?,
        Answer::Map(m) => put_walk_bare(out, m)?,
        Answer::List(l) => put_walk_bare(out, l)?,
        _ => return encode_answer(slot, answer, out, export).map(|()| None),
    }
    if out.len() - at >= TAIL {
        return Ok(Some(at));
    }
    behind_length(out, at);
    Ok(None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use guatiao::library::kind::KindDescription;

    const KIND: &str = r#"{"format": 1, "kinds": [{"name": "t", "floor_hash": 1, "object": false,
      "slots": [
        {"name": "text", "required": true, "fallible": false,
         "args": [{"name": "n", "type": "i64"}, {"name": "s", "type": "str"}]},
        {"name": "bytes", "required": true, "fallible": false,
         "args": [{"name": "n", "type": "i64"}, {"name": "b", "type": "bytes"}]},
        {"name": "tree", "required": true, "fallible": false, "args": [],
         "answer": {"type": "map"}}]}]}"#;

    fn kind() -> KindDescription {
        KindDescription::document_json(KIND).unwrap().remove(0)
    }

    fn none(_: RawObject) -> ObjectRef {
        unreachable!("no object here")
    }

    /// A frame's head then its tail: the bytes `encode_args` writes.
    #[test]
    fn a_large_last_argument_as_a_tail_is_the_same_bytes() {
        let kind = kind();
        let text = "t".repeat(TAIL);
        let bytes = vec![7u8; TAIL];
        let cases = [
            (0, Arg::Str(&text[..TAIL - 1]), false),
            (0, Arg::Str(&text), true),
            (1, Arg::Bytes(&bytes[..TAIL - 1]), false),
            (1, Arg::Bytes(&bytes), true),
        ];
        for (slot, last, tail_expected) in cases {
            let slot = &kind.slots()[slot];
            let args = [Arg::Int(-3), last];
            let mut whole = Vec::new();
            encode_args(slot, &args, &[None, None], &mut whole).unwrap();
            let mut head = Vec::new();
            let tail = encode_args_tail(slot, &args, &[None, None], &mut head).unwrap();
            assert_eq!(tail.is_some(), tail_expected);
            head.extend_from_slice(tail.unwrap_or_default());
            assert_eq!(head, whole);
        }
    }

    /// A slot of each argument type, each answering a map; those named
    /// `back_` write back.
    const WRITES: &str = r#"{"format": 1, "kinds": [{"name": "w", "floor_hash": 1, "object": false,
      "slots": [
        {"name": "back_lent_map", "args": [{"name": "a", "type": "map", "pass": "lent"}]},
        {"name": "back_lent_list", "args": [{"name": "a", "type": "list", "pass": "lent"}]},
        {"name": "back_lent_value", "args": [{"name": "a", "type": "value", "pass": "lent"}]},
        {"name": "back_buffer", "args": [{"name": "a", "type": "bytes_mut"}]},
        {"name": "map", "args": [{"name": "a", "type": "map", "pass": "borrowed"}]},
        {"name": "taken_list", "args": [{"name": "a", "type": "list", "pass": "taken"}]},
        {"name": "value", "args": [{"name": "a", "type": "value", "pass": "borrowed", "nullable": true}]},
        {"name": "text", "args": [{"name": "a", "type": "str"}]},
        {"name": "bytes", "args": [{"name": "a", "type": "bytes"}]},
        {"name": "n", "args": [{"name": "a", "type": "i64"}]}]}]}"#;

    /// What a call writes back is one rule: the slots whose writebacks
    /// `encode_writebacks` writes are those whose large answer
    /// `encode_answer_tail` keeps inside the frame.
    #[test]
    fn the_slots_that_write_back_are_the_slots_whose_answer_stays_whole() {
        let slot = r#""required": true, "fallible": false, "answer": {"type": "map"}, "args""#;
        let doc = WRITES.replace(r#""args""#, slot);
        let kind = KindDescription::document_json(&doc).unwrap().remove(0);
        let mut large = Map::new();
        large.set("k", "v".repeat(TAIL).as_str());
        for slot in kind.slots() {
            let arg = match slot.args()[0].ty() {
                ArgType::Tree {
                    tree: Tree::Map, ..
                } => Decoded::Map(Map::new()),
                ArgType::Tree {
                    tree: Tree::List, ..
                } => Decoded::List(List::new()),
                ArgType::Tree { .. } => Decoded::Value(Some(Value::from(true))),
                ArgType::BytesMut => Decoded::Buffer(vec![1, 2]),
                ArgType::Str => Decoded::Str("s"),
                ArgType::Bytes => Decoded::Bytes(b"b"),
                _ => Decoded::Int(1),
            };
            let back = slot.name().starts_with("back_");
            let mut written = Vec::new();
            encode_writebacks(slot, &[arg], &mut written).unwrap();
            assert_eq!(!written.is_empty(), back, "{}", slot.name());
            let mut out = Vec::new();
            let tail =
                encode_answer_tail(slot, Answer::Map(large.clone()), &mut out, &mut none).unwrap();
            assert_eq!(tail.is_none(), back, "{}", slot.name());
        }
    }

    /// The bytes before the tail, its length, then the tail: the bytes
    /// `encode_answer` writes.
    #[test]
    fn a_large_answer_as_a_tail_is_the_same_bytes() {
        let kind = kind();
        let slot = &kind.slots()[2];
        for len in [10, TAIL] {
            let mut map = Map::new();
            map.set("k", "v".repeat(len).as_str());
            let mut whole = b"head".to_vec();
            encode_answer(slot, Answer::Map(map.clone()), &mut whole, &mut none).unwrap();
            let mut out = b"head".to_vec();
            let at = encode_answer_tail(slot, Answer::Map(map), &mut out, &mut none).unwrap();
            let rebuilt = match at {
                None => out,
                Some(from) => {
                    let mut head = out[..from].to_vec();
                    put_uint(&mut head, (out.len() - from) as u64);
                    head.extend_from_slice(&out[from..]);
                    head
                }
            };
            assert_eq!(at.is_some(), len == TAIL, "{len}");
            assert_eq!(rebuilt, whole);
        }
    }
}
