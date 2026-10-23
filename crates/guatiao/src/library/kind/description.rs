// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A kind as data: the description `#[guatiao::kind]` writes
//! (`<Trait>Vtable::description()`) and a kind crate commits as
//! `<crate>.kinds.json`, read from a [`Value`].
//!
//! The format is stated in `docs/encodings.md`, "A kind, described". This
//! module reads a value, never text: a caller holding JSON parses it first
//! (`guatiao-serde`), which keeps this crate free of a format dependency.

#![forbid(unsafe_code)]

use crate::value::convert::{FromValue, TryAsRef};
use crate::value::types::{List, Map, Value};

/// A scalar argument or answer, under its Rust name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Scalar {
    /// `bool`
    Bool,
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `i64`
    I64,
    /// `i128`
    I128,
    /// `isize`
    Isize,
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `u64`
    U64,
    /// `u128`
    U128,
    /// `usize`
    Usize,
    /// `f32`
    F32,
    /// `f64`
    F64,
}

impl Scalar {
    /// The scalar a description names, or `None`.
    pub fn from_name(name: &str) -> Option<Scalar> {
        Some(match name {
            "bool" => Scalar::Bool,
            "i8" => Scalar::I8,
            "i16" => Scalar::I16,
            "i32" => Scalar::I32,
            "i64" => Scalar::I64,
            "i128" => Scalar::I128,
            "isize" => Scalar::Isize,
            "u8" => Scalar::U8,
            "u16" => Scalar::U16,
            "u32" => Scalar::U32,
            "u64" => Scalar::U64,
            "u128" => Scalar::U128,
            "usize" => Scalar::Usize,
            "f32" => Scalar::F32,
            "f64" => Scalar::F64,
            _ => return None,
        })
    }

    /// Its name in a description.
    pub fn name(self) -> &'static str {
        match self {
            Scalar::Bool => "bool",
            Scalar::I8 => "i8",
            Scalar::I16 => "i16",
            Scalar::I32 => "i32",
            Scalar::I64 => "i64",
            Scalar::I128 => "i128",
            Scalar::Isize => "isize",
            Scalar::U8 => "u8",
            Scalar::U16 => "u16",
            Scalar::U32 => "u32",
            Scalar::U64 => "u64",
            Scalar::U128 => "u128",
            Scalar::Usize => "usize",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
        }
    }
}

/// Which tree an argument or answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Tree {
    /// A `guatiao_value`.
    Value,
    /// A `guatiao_map`.
    Map,
    /// A `guatiao_list`.
    List,
}

/// How a tree or an object argument crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Pass {
    /// Read for the call, the caller keeps it (`const` in C).
    Borrowed,
    /// Handed over: the callee owns it from the call on.
    Taken,
    /// The caller's, lent for the call: the callee may change it, never
    /// free or keep it.
    Lent,
}

/// What one argument is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArgType {
    /// A number or a flag, by value.
    Scalar(Scalar),
    /// Borrowed UTF-8 text (`guatiao_str`).
    Str,
    /// Borrowed bytes (`guatiao_bytes`).
    Bytes,
    /// The caller's bytes to write into (`guatiao_bytes_mut`).
    BytesMut,
    /// A tree, and how it crosses; `nullable` only for a borrowed value.
    Tree {
        /// Which tree.
        tree: Tree,
        /// How it crosses.
        pass: Pass,
        /// Whether a borrowed value may be null.
        nullable: bool,
    },
    /// An object of the kind named, taken (`guatiao_object` by value) or
    /// lent (`const guatiao_object *`).
    Object {
        /// The object kind's name.
        kind: String,
        /// [`Pass::Taken`] or [`Pass::Lent`].
        pass: Pass,
    },
}

/// What a slot answers through its out slot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnswerType {
    /// A scalar.
    Scalar(Scalar),
    /// Text the callee builds (`guatiao_string`).
    Text,
    /// A tree, built in the caller's carrier.
    Tree(Tree),
    /// An object of the kind named, handed to the caller.
    Object {
        /// The object kind's name.
        kind: String,
    },
}

/// One argument: its name and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgDescription {
    name: String,
    ty: ArgType,
}

impl ArgDescription {
    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its type.
    pub fn ty(&self) -> &ArgType {
        &self.ty
    }
}

/// One slot, in table order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotDescription {
    name: String,
    required: bool,
    fallible: bool,
    args: Vec<ArgDescription>,
    answer: Option<AnswerType>,
    doc: Option<String>,
}

impl SlotDescription {
    /// The method's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether a table must fill it.
    pub fn required(&self) -> bool {
        self.required
    }

    /// Whether it takes a `guatiao_provider_error *err` last.
    pub fn fallible(&self) -> bool {
        self.fallible
    }

    /// Its arguments, after `ctx`.
    pub fn args(&self) -> &[ArgDescription] {
        &self.args
    }

    /// What it answers through `out`, or `None` for nothing.
    pub fn answer(&self) -> Option<&AnswerType> {
        self.answer.as_ref()
    }

    /// Its documentation, or `None`.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }
}

/// A kind, described: what its table holds and how each slot is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindDescription {
    name: String,
    floor_hash: u32,
    object: bool,
    doc: Option<String>,
    slots: Vec<SlotDescription>,
}

/// Why a value is not a description: where, and what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptionError {
    at: String,
    why: String,
}

impl DescriptionError {
    fn new(at: impl Into<String>, why: impl Into<String>) -> DescriptionError {
        DescriptionError {
            at: at.into(),
            why: why.into(),
        }
    }

    /// Where in the description, as a path (`kinds[0].slots[2].args[1]`).
    pub fn at(&self) -> &str {
        &self.at
    }

    /// What was wrong.
    pub fn why(&self) -> &str {
        &self.why
    }
}

impl core::fmt::Display for DescriptionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.at.is_empty() {
            write!(f, "the kind description {}", self.why)
        } else {
            write!(f, "the kind description's `{}` {}", self.at, self.why)
        }
    }
}

impl core::error::Error for DescriptionError {}

fn under(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

fn map_at<'a>(value: &'a Value, at: &str) -> Result<&'a Map, DescriptionError> {
    TryAsRef::<Map>::try_as_ref(value).ok_or_else(|| DescriptionError::new(at, "is not a map"))
}

fn text<'a>(map: &'a Map, at: &str, key: &str) -> Result<&'a str, DescriptionError> {
    let at = under(at, key);
    let value = map
        .get(key)
        .ok_or_else(|| DescriptionError::new(&at, "is missing"))?;
    let text = TryAsRef::<str>::try_as_ref(value)
        .ok_or_else(|| DescriptionError::new(&at, "is not text"))?;
    if text.is_empty() {
        return Err(DescriptionError::new(at, "is empty"));
    }
    Ok(text)
}

fn flag(map: &Map, at: &str, key: &str, default: Option<bool>) -> Result<bool, DescriptionError> {
    let at = under(at, key);
    match (map.get(key), default) {
        (None, Some(default)) => Ok(default),
        (None, None) => Err(DescriptionError::new(at, "is missing")),
        (Some(value), _) => {
            bool::from_value(value).map_err(|_| DescriptionError::new(at, "is not a bool"))
        }
    }
}

fn doc(map: &Map, at: &str) -> Result<Option<String>, DescriptionError> {
    match map.get("doc") {
        None => Ok(None),
        Some(value) => TryAsRef::<str>::try_as_ref(value)
            .map(|d| Some(d.to_string()))
            .ok_or_else(|| DescriptionError::new(under(at, "doc"), "is not text")),
    }
}

fn list_at<'a>(map: &'a Map, at: &str, key: &str) -> Result<&'a List, DescriptionError> {
    let at = under(at, key);
    let value = map
        .get(key)
        .ok_or_else(|| DescriptionError::new(&at, "is missing"))?;
    TryAsRef::<List>::try_as_ref(value).ok_or_else(|| DescriptionError::new(at, "is not a list"))
}

fn tree_of(name: &str) -> Option<Tree> {
    Some(match name {
        "value" => Tree::Value,
        "map" => Tree::Map,
        "list" => Tree::List,
        _ => return None,
    })
}

fn pass_of(map: &Map, at: &str) -> Result<Pass, DescriptionError> {
    Ok(match text(map, at, "pass")? {
        "borrowed" => Pass::Borrowed,
        "taken" => Pass::Taken,
        "lent" => Pass::Lent,
        other => {
            return Err(DescriptionError::new(
                under(at, "pass"),
                format!("is `{other}`, not borrowed, taken or lent"),
            ));
        }
    })
}

fn arg(value: &Value, at: &str) -> Result<ArgDescription, DescriptionError> {
    let map = map_at(value, at)?;
    let name = text(map, at, "name")?.to_string();
    let ty = text(map, at, "type")?;
    let nullable = flag(map, at, "nullable", Some(false))?;
    let ty = if let Some(scalar) = Scalar::from_name(ty) {
        ArgType::Scalar(scalar)
    } else if let Some(tree) = tree_of(ty) {
        let pass = pass_of(map, at)?;
        if nullable && (tree != Tree::Value || pass != Pass::Borrowed) {
            return Err(DescriptionError::new(
                under(at, "nullable"),
                "is set on something other than a borrowed value",
            ));
        }
        ArgType::Tree {
            tree,
            pass,
            nullable,
        }
    } else {
        match ty {
            "str" => ArgType::Str,
            "bytes" => ArgType::Bytes,
            "bytes_mut" => ArgType::BytesMut,
            "object" => {
                let kind = text(map, at, "kind")?.to_string();
                let pass = pass_of(map, at)?;
                if pass == Pass::Borrowed {
                    return Err(DescriptionError::new(
                        under(at, "pass"),
                        "is borrowed: an object is taken or lent",
                    ));
                }
                ArgType::Object { kind, pass }
            }
            other => {
                return Err(DescriptionError::new(
                    under(at, "type"),
                    format!("is `{other}`, which no argument is"),
                ));
            }
        }
    };
    if nullable && !matches!(ty, ArgType::Tree { .. }) {
        return Err(DescriptionError::new(
            under(at, "nullable"),
            "is set on something other than a borrowed value",
        ));
    }
    Ok(ArgDescription { name, ty })
}

fn answer(value: &Value, at: &str) -> Result<AnswerType, DescriptionError> {
    let map = map_at(value, at)?;
    let ty = text(map, at, "type")?;
    if let Some(scalar) = Scalar::from_name(ty) {
        return Ok(AnswerType::Scalar(scalar));
    }
    if let Some(tree) = tree_of(ty) {
        return Ok(AnswerType::Tree(tree));
    }
    match ty {
        "text" => Ok(AnswerType::Text),
        "object" => Ok(AnswerType::Object {
            kind: text(map, at, "kind")?.to_string(),
        }),
        other => Err(DescriptionError::new(
            under(at, "type"),
            format!("is `{other}`, which no answer is"),
        )),
    }
}

fn slot(value: &Value, at: &str) -> Result<SlotDescription, DescriptionError> {
    let map = map_at(value, at)?;
    let name = text(map, at, "name")?.to_string();
    let required = flag(map, at, "required", None)?;
    let fallible = flag(map, at, "fallible", None)?;
    let mut args = Vec::new();
    for (i, a) in list_at(map, at, "args")?.iter().enumerate() {
        args.push(arg(a, &format!("{}[{i}]", under(at, "args")))?);
    }
    let answer = match map.get("answer") {
        None => None,
        Some(value) => Some(answer(value, &under(at, "answer"))?),
    };
    Ok(SlotDescription {
        name,
        required,
        fallible,
        args,
        answer,
        doc: doc(map, at)?,
    })
}

impl KindDescription {
    /// Reads one kind, as `<Trait>Vtable::description()` writes it.
    pub fn from_value(value: &Value) -> Result<KindDescription, DescriptionError> {
        Self::read(value, "")
    }

    fn read(value: &Value, at: &str) -> Result<KindDescription, DescriptionError> {
        let map = map_at(value, at)?;
        let name = text(map, at, "name")?.to_string();
        let floor_hash = u32::from_value(
            map.get("floor_hash")
                .ok_or_else(|| DescriptionError::new(under(at, "floor_hash"), "is missing"))?,
        )
        .map_err(|_| DescriptionError::new(under(at, "floor_hash"), "is not a u32"))?;
        let object = flag(map, at, "object", None)?;
        let mut slots = Vec::new();
        for (i, s) in list_at(map, at, "slots")?.iter().enumerate() {
            let slot = slot(s, &format!("{}[{i}]", under(at, "slots")))?;
            if slots
                .iter()
                .any(|known: &SlotDescription| known.name == slot.name)
            {
                return Err(DescriptionError::new(
                    format!("{}[{i}]", under(at, "slots")),
                    format!("names `{}` a second time", slot.name),
                ));
            }
            if object && slot.name == "destroy" {
                return Err(DescriptionError::new(
                    format!("{}[{i}]", under(at, "slots")),
                    "names `destroy`, which an object kind's table holds before its slots",
                ));
            }
            slots.push(slot);
        }
        Ok(KindDescription {
            name,
            floor_hash,
            object,
            doc: doc(map, at)?,
            slots,
        })
    }

    /// Reads a document, `{"format": 1, "kinds": [..]}`, as a kind crate
    /// commits it.
    pub fn document(value: &Value) -> Result<Vec<KindDescription>, DescriptionError> {
        let map = map_at(value, "")?;
        let format = map
            .get("format")
            .ok_or_else(|| DescriptionError::new("format", "is missing"))?;
        if u32::from_value(format).ok() != Some(1) {
            return Err(DescriptionError::new(
                "format",
                "is not 1, the only format this build reads",
            ));
        }
        let mut kinds = Vec::new();
        for (i, k) in list_at(map, "", "kinds")?.iter().enumerate() {
            kinds.push(Self::read(k, &format!("kinds[{i}]"))?);
        }
        Ok(kinds)
    }

    /// The kind's name, as a descriptor's `kinds` and `tables` name it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the table's header carries.
    pub fn floor_hash(&self) -> u32 {
        self.floor_hash
    }

    /// Whether it is an object kind: `destroy` first, never offered by a
    /// provider.
    pub fn object(&self) -> bool {
        self.object
    }

    /// Its documentation, or `None`.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Its slots, in table order (without an object kind's `destroy`).
    pub fn slots(&self) -> &[SlotDescription] {
        &self.slots
    }

    /// The slot named `name`, and its index in [`slots`](Self::slots).
    pub fn slot(&self, name: &str) -> Option<(usize, &SlotDescription)> {
        self.slots.iter().enumerate().find(|(_, s)| s.name == name)
    }

    /// Where slot `index` sits in the table, in pointer-sized words after
    /// the 8-byte header: an object kind's `destroy` is word 0.
    pub fn word_of(&self, index: usize) -> usize {
        index + usize::from(self.object)
    }

    /// `sizeof` the whole table on this target: the header and one
    /// pointer per slot.
    pub fn size(&self) -> usize {
        size_of::<super::KindHeader>()
            + (self.slots.len() + usize::from(self.object)) * size_of::<usize>()
    }

    /// One past the end of the last required slot: the smallest table a
    /// caller accepts.
    pub fn floor(&self) -> usize {
        let last = self.slots.iter().rposition(|s| s.required);
        let words = match last {
            Some(i) => self.word_of(i) + 1,
            None => usize::from(self.object),
        };
        size_of::<super::KindHeader>() + words * size_of::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_value(s: &str) -> Value {
        Value::from(s)
    }

    fn map_of(entries: Vec<(&str, Value)>) -> Value {
        let mut map = Map::new();
        for (k, v) in entries {
            map.set(k, v);
        }
        Value::from(map)
    }

    fn list_of(items: Vec<Value>) -> Value {
        let mut list = List::new();
        for v in items {
            list.push(v);
        }
        Value::from(list)
    }

    fn greeter() -> Value {
        map_of(vec![
            ("name", text_value("greeter")),
            ("floor_hash", Value::from(2695856491u32)),
            ("object", Value::from(false)),
            (
                "slots",
                list_of(vec![
                    map_of(vec![
                        ("name", text_value("greet")),
                        ("required", Value::from(true)),
                        ("fallible", Value::from(true)),
                        (
                            "args",
                            list_of(vec![map_of(vec![
                                ("name", text_value("name")),
                                ("type", text_value("str")),
                            ])]),
                        ),
                        ("answer", map_of(vec![("type", text_value("map"))])),
                    ]),
                    map_of(vec![
                        ("name", text_value("check")),
                        ("required", Value::from(false)),
                        ("fallible", Value::from(true)),
                        (
                            "args",
                            list_of(vec![map_of(vec![
                                ("name", text_value("config")),
                                ("type", text_value("map")),
                                ("pass", text_value("lent")),
                            ])]),
                        ),
                        ("answer", map_of(vec![("type", text_value("bool"))])),
                    ]),
                ]),
            ),
        ])
    }

    #[test]
    fn a_kind_is_read_with_its_slots_in_table_order() {
        let kind = KindDescription::from_value(&greeter()).expect("a description");
        assert_eq!(kind.name(), "greeter");
        assert_eq!(kind.floor_hash(), 2695856491);
        assert!(!kind.object());
        assert_eq!(kind.slots().len(), 2);
        let (at, check) = kind.slot("check").expect("named");
        assert_eq!(at, 1);
        assert_eq!(
            check.args()[0].ty(),
            &ArgType::Tree {
                tree: Tree::Map,
                pass: Pass::Lent,
                nullable: false
            }
        );
        assert_eq!(check.answer(), Some(&AnswerType::Scalar(Scalar::Bool)));
        let word = size_of::<usize>();
        assert_eq!(kind.size(), 8 + 2 * word);
        assert_eq!(kind.floor(), 8 + word, "only greet is required");
    }

    #[test]
    fn a_document_is_read_and_its_format_checked() {
        let doc = map_of(vec![
            ("format", Value::from(1u32)),
            ("kinds", list_of(vec![greeter()])),
        ]);
        assert_eq!(KindDescription::document(&doc).expect("read").len(), 1);
        let later = map_of(vec![
            ("format", Value::from(2u32)),
            ("kinds", list_of(vec![])),
        ]);
        assert_eq!(
            KindDescription::document(&later).unwrap_err().at(),
            "format"
        );
    }

    #[test]
    fn what_is_wrong_is_named_with_where() {
        let wrong = map_of(vec![
            ("name", text_value("w")),
            ("floor_hash", Value::from(1u32)),
            ("object", Value::from(false)),
            (
                "slots",
                list_of(vec![map_of(vec![
                    ("name", text_value("s")),
                    ("required", Value::from(true)),
                    ("fallible", Value::from(false)),
                    (
                        "args",
                        list_of(vec![map_of(vec![
                            ("name", text_value("x")),
                            ("type", text_value("u7")),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let e = KindDescription::from_value(&wrong).unwrap_err();
        assert_eq!(e.at(), "slots[0].args[0].type");
        assert!(e.to_string().contains("u7"), "{e}");

        let lent_object_borrowed = map_of(vec![
            ("name", text_value("o")),
            ("floor_hash", Value::from(1u32)),
            ("object", Value::from(true)),
            (
                "slots",
                list_of(vec![map_of(vec![
                    ("name", text_value("destroy")),
                    ("required", Value::from(true)),
                    ("fallible", Value::from(false)),
                    ("args", list_of(vec![])),
                ])]),
            ),
        ]);
        assert!(
            KindDescription::from_value(&lent_object_borrowed)
                .unwrap_err()
                .why()
                .contains("destroy")
        );
    }

    #[test]
    fn an_object_kind_counts_its_destroy_slot() {
        let listener = map_of(vec![
            ("name", text_value("listener")),
            ("floor_hash", Value::from(1867921573u32)),
            ("object", Value::from(true)),
            (
                "slots",
                list_of(vec![map_of(vec![
                    ("name", text_value("heard")),
                    ("required", Value::from(true)),
                    ("fallible", Value::from(false)),
                    (
                        "args",
                        list_of(vec![map_of(vec![
                            ("name", text_value("what")),
                            ("type", text_value("str")),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let kind = KindDescription::from_value(&listener).expect("read");
        let word = size_of::<usize>();
        assert_eq!(kind.word_of(0), 1);
        assert_eq!(kind.size(), 8 + 2 * word);
        assert_eq!(kind.floor(), 8 + 2 * word);
    }
}
