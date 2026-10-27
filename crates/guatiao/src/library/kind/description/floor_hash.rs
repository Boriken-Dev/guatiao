// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The floor hash of a kind born outside Rust: its slots spelled as
//! `#[guatiao::kind]` spells a trait's methods, so a declaration in any
//! language repeating a trait's slots hashes as the trait does. The rule
//! is stated in `docs/encodings.md` ("A kind, described") and checked
//! against `crates/guatiao-derive/tests/vectors/floor_hash.json`.

#![forbid(unsafe_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::super::parts::fnv1a;
use super::{
    AnswerType, ArgType, DescriptionError, KindDescription, Pass, SlotDescription, Tree, flag,
    json, map_at, slots_of,
};
use crate::value::alloc::Alloc;

impl KindDescription {
    /// The floor hash of the kind whose description `text` holds as JSON:
    /// its `object` and `slots` are read as a document's are, every other
    /// key (`name`, `floor_hash`, `doc`) is not.
    pub(crate) fn floor_hash_json(text: &str) -> Result<u32, DescriptionError> {
        let value = json::read(text, Alloc::rust()).map_err(|(at, why)| {
            DescriptionError::new(format!("byte {at}"), format!("is not {why}"))
        })?;
        let map = map_at(&value, "")?;
        let object = flag(map, "", "object", None)?;
        Ok(floor_hash_of(object, &slots_of(map, "", object)?))
    }
}

/// FNV-1a (32-bit) over `provider` or `object`, then every slot in table
/// order as `name(argument types)->answer`, joined by `;`.
fn floor_hash_of(object: bool, slots: &[SlotDescription]) -> u32 {
    let mut input = String::from(if object { "object" } else { "provider" });
    for slot in slots {
        input.push(';');
        input.push_str(&signature(slot));
    }
    fnv1a(&input)
}

fn signature(slot: &SlotDescription) -> String {
    let args: Vec<String> = slot.args().iter().map(|a| arg(a.ty())).collect();
    let answer = slot.answer().map_or_else(|| "()".to_string(), answer);
    let answer = if slot.fallible() {
        format!("Result<{answer},ProviderError>")
    } else {
        answer
    };
    format!("{}({})->{answer}", slot.name(), args.join(","))
}

fn tree(tree: Tree) -> &'static str {
    match tree {
        Tree::Value => "Value",
        Tree::Map => "Map",
        Tree::List => "List",
    }
}

// A lent object is spelled `&mut dyn K` whether the trait lends it shared
// or not: a description does not tell the two apart. A kind declaring
// `&dyn K` pins its hash instead.
fn arg(ty: &ArgType) -> String {
    match ty {
        ArgType::Scalar(scalar) => scalar.name().to_string(),
        ArgType::Str => "&str".to_string(),
        ArgType::Bytes => "&[u8]".to_string(),
        ArgType::BytesMut => "&mut[u8]".to_string(),
        ArgType::Tree {
            tree: t,
            pass,
            nullable,
        } => match pass {
            Pass::Taken => tree(*t).to_string(),
            Pass::Lent => format!("&mut{}", tree(*t)),
            Pass::Borrowed if *nullable => format!("Option<&{}>", tree(*t)),
            Pass::Borrowed => format!("&{}", tree(*t)),
        },
        ArgType::Object { kind, pass } => match pass {
            Pass::Taken => format!("Object<dyn{}>", pascal(kind)),
            Pass::Lent | Pass::Borrowed => format!("&mutdyn{}", pascal(kind)),
        },
    }
}

fn answer(answer: &AnswerType) -> String {
    match answer {
        AnswerType::Scalar(scalar) => scalar.name().to_string(),
        AnswerType::Text => "String".to_string(),
        AnswerType::Tree(t) => tree(*t).to_string(),
        AnswerType::Object { kind } => format!("Object<dyn{}>", pascal(kind)),
    }
}

/// A kind's name as its trait would be named: split at every character
/// that is not an ASCII letter or digit, each part's first letter raised
/// (`codec.v2` is `CodecV2`).
fn pascal(kind: &str) -> String {
    let mut out = String::with_capacity(kind.len());
    for part in kind
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
    {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_name_is_spelled_as_its_trait() {
        assert_eq!(pascal("codec.v2"), "CodecV2");
        assert_eq!(pascal("sink_x"), "SinkX");
        assert_eq!(pascal("__a__b"), "AB");
    }

    #[test]
    fn every_slot_is_hashed_in_table_order() {
        let text = r#"{"object": false, "slots": [
            {"name": "greet", "required": true, "fallible": true,
             "args": [{"name": "name", "type": "str"}], "answer": {"type": "map"}},
            {"name": "shout", "required": false, "fallible": false,
             "args": [{"name": "name", "type": "str"}], "answer": {"type": "text"}}]}"#;
        assert_eq!(
            KindDescription::floor_hash_json(text),
            Ok(fnv1a(
                "provider;greet(&str)->Result<Map,ProviderError>;shout(&str)->String"
            ))
        );
    }

    #[test]
    fn a_kind_with_no_slots_hashes_its_shape() {
        let text = r#"{"object": true, "slots": []}"#;
        assert_eq!(KindDescription::floor_hash_json(text), Ok(fnv1a("object")));
    }

    #[test]
    fn what_is_not_a_kind_is_refused_where_it_is_wrong() {
        let e = KindDescription::floor_hash_json(r#"{"slots": []}"#).unwrap_err();
        assert_eq!((e.at(), e.why()), ("object", "is missing"));
        let e = KindDescription::floor_hash_json("{").unwrap_err();
        assert!(e.at().starts_with("byte "), "{e}");
    }
}
