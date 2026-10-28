// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A kind as data: the description `#[guatiao::kind]` writes
//! (`<Trait>Vtable::description()`) and a kind crate commits as
//! `<crate>.kinds.json`, read from a [`Value`] and written back as one.
//!
//! The format is stated in `docs/encodings.md`, "A kind, described". A
//! description is read from a value; the JSON text a provider carries in
//! its descriptor is read by a small reader of this module's own, so the
//! crate takes no format dependency.

#![forbid(unsafe_code)]

mod floor_hash;
mod json;

use crate::value::alloc::Alloc;
use crate::value::convert::{FromValue, ToValue, TryAsRef};
use crate::value::error::ValueError;
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

/// A kind's `slots`, each name once and none `header`, an object kind's
/// never `destroy`, `bytes_mut` on an object kind only, an object only on
/// a fallible slot and a list never borrowed: what `#[guatiao::kind]`
/// refuses to declare, checked here for a kind declared anywhere else.
/// Required and optional slots come in any order.
fn slots_of(map: &Map, at: &str, object: bool) -> Result<Vec<SlotDescription>, DescriptionError> {
    let mut slots = Vec::new();
    for (i, s) in list_at(map, at, "slots")?.iter().enumerate() {
        let here = format!("{}[{i}]", under(at, "slots"));
        let slot = slot(s, &here)?;
        if let Some(j) = slot.args.iter().position(|a| a.ty == ArgType::BytesMut)
            && !object
        {
            return Err(DescriptionError::new(
                format!("{here}.args[{j}].type"),
                "is `bytes_mut`, the caller's bytes written in place, which cross on an object \
                 kind only",
            ));
        }
        if let Some(j) = slot.args.iter().position(|a| {
            matches!(
                a.ty,
                ArgType::Tree {
                    tree: Tree::List,
                    pass: Pass::Borrowed,
                    ..
                }
            )
        }) {
            return Err(DescriptionError::new(
                format!("{here}.args[{j}].pass"),
                "is borrowed: a list is not borrowed, but taken or lent",
            ));
        }
        // The object's table is checked where it arrives, and only a
        // fallible slot can answer that it was refused.
        if !slot.fallible {
            let fallible_only = "is `object`, which crosses on a fallible slot only: its table is \
                                 checked where it arrives";
            if let Some(j) = slot
                .args
                .iter()
                .position(|a| matches!(a.ty, ArgType::Object { .. }))
            {
                return Err(DescriptionError::new(
                    format!("{here}.args[{j}].type"),
                    fallible_only,
                ));
            }
            if matches!(slot.answer, Some(AnswerType::Object { .. })) {
                return Err(DescriptionError::new(
                    format!("{here}.answer.type"),
                    fallible_only,
                ));
            }
        }
        if slot.name == "header" {
            return Err(DescriptionError::new(
                here,
                "names `header`: the table's first field is its `header`",
            ));
        }
        if slots
            .iter()
            .any(|known: &SlotDescription| known.name == slot.name)
        {
            return Err(DescriptionError::new(
                here,
                format!("names `{}` a second time", slot.name),
            ));
        }
        if object && slot.name == "destroy" {
            return Err(DescriptionError::new(
                here,
                "names `destroy`, which an object kind's table holds before its slots",
            ));
        }
        slots.push(slot);
    }
    Ok(slots)
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
        let slots = slots_of(map, at, object)?;
        Ok(KindDescription {
            name,
            floor_hash,
            object,
            doc: doc(map, at)?,
            slots,
        })
    }

    /// Reads a document, `{"kinds": [..]}`, as a kind crate commits it. A
    /// key other than `kinds` is skipped.
    pub fn document(value: &Value) -> Result<Vec<KindDescription>, DescriptionError> {
        let map = map_at(value, "")?;
        let mut kinds = Vec::new();
        for (i, k) in list_at(map, "", "kinds")?.iter().enumerate() {
            kinds.push(Self::read(k, &format!("kinds[{i}]"))?);
        }
        Ok(kinds)
    }

    /// Reads a document from its JSON text, as a provider carries it in
    /// [`ProviderInfo::kind_descriptions`](crate::library::ProviderInfo::kind_descriptions)
    /// and a kind crate commits it. Text that is not JSON is refused at
    /// `byte N`, where it stops being JSON.
    pub fn document_json(text: &str) -> Result<Vec<KindDescription>, DescriptionError> {
        Self::document(&Self::document_value(text, Alloc::rust())?)
    }

    /// The document `text` holds as a value built in `alloc`, refused as
    /// [`document_json`](KindDescription::document_json) refuses it.
    pub(crate) fn document_value(text: &str, alloc: Alloc) -> Result<Value, DescriptionError> {
        let value = json::read(text, alloc).map_err(|(at, why)| {
            DescriptionError::new(format!("byte {at}"), format!("is not {why}"))
        })?;
        Self::document(&value)?;
        Ok(value)
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

    /// The smallest table a caller accepts: every slot, [`size`](Self::size).
    pub fn floor(&self) -> usize {
        self.size()
    }
}

/// The kind as `<Trait>Vtable::description()` writes it, key for key and
/// in the same order: what [`KindDescription::from_value`] reads back.
/// How a host that holds a description, but not its text, hands it on,
/// as a provider's document or to a language that loads it.
impl ToValue for KindDescription {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut kind = Map::new_in(alloc);
        put(&mut kind, "name", self.name.as_str(), alloc)?;
        put(&mut kind, "floor_hash", self.floor_hash, alloc)?;
        put(&mut kind, "object", self.object, alloc)?;
        if let Some(doc) = &self.doc {
            put(&mut kind, "doc", doc.as_str(), alloc)?;
        }
        let mut slots = List::new_in(alloc);
        for slot in &self.slots {
            slots.push_in(slot_value(slot, alloc)?, alloc)?;
        }
        kind.set_in("slots", slots, alloc)?;
        Ok(kind.into())
    }
}

fn put(map: &mut Map, key: &str, value: impl ToValue, alloc: Alloc) -> Result<(), ValueError> {
    map.set_in(key, value.to_value_in(alloc)?, alloc)
}

fn tree_name(tree: Tree) -> &'static str {
    match tree {
        Tree::Value => "value",
        Tree::Map => "map",
        Tree::List => "list",
    }
}

fn pass_name(pass: Pass) -> &'static str {
    match pass {
        Pass::Borrowed => "borrowed",
        Pass::Taken => "taken",
        Pass::Lent => "lent",
    }
}

fn slot_value(slot: &SlotDescription, alloc: Alloc) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    put(&mut map, "name", slot.name.as_str(), alloc)?;
    put(&mut map, "required", slot.required, alloc)?;
    put(&mut map, "fallible", slot.fallible, alloc)?;
    let mut args = List::new_in(alloc);
    for arg in &slot.args {
        args.push_in(arg_value(arg, alloc)?, alloc)?;
    }
    map.set_in("args", args, alloc)?;
    if let Some(answer) = &slot.answer {
        map.set_in("answer", answer_value(answer, alloc)?, alloc)?;
    }
    if let Some(doc) = &slot.doc {
        put(&mut map, "doc", doc.as_str(), alloc)?;
    }
    Ok(map.into())
}

fn arg_value(arg: &ArgDescription, alloc: Alloc) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    put(&mut map, "name", arg.name.as_str(), alloc)?;
    match &arg.ty {
        ArgType::Scalar(scalar) => put(&mut map, "type", scalar.name(), alloc)?,
        ArgType::Str => put(&mut map, "type", "str", alloc)?,
        ArgType::Bytes => put(&mut map, "type", "bytes", alloc)?,
        ArgType::BytesMut => put(&mut map, "type", "bytes_mut", alloc)?,
        ArgType::Tree {
            tree,
            pass,
            nullable,
        } => {
            put(&mut map, "type", tree_name(*tree), alloc)?;
            put(&mut map, "pass", pass_name(*pass), alloc)?;
            if *nullable {
                put(&mut map, "nullable", true, alloc)?;
            }
        }
        ArgType::Object { kind, pass } => {
            put(&mut map, "type", "object", alloc)?;
            put(&mut map, "kind", kind.as_str(), alloc)?;
            put(&mut map, "pass", pass_name(*pass), alloc)?;
        }
    }
    Ok(map.into())
}

fn answer_value(answer: &AnswerType, alloc: Alloc) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    match answer {
        AnswerType::Scalar(scalar) => put(&mut map, "type", scalar.name(), alloc)?,
        AnswerType::Text => put(&mut map, "type", "text", alloc)?,
        AnswerType::Tree(tree) => put(&mut map, "type", tree_name(*tree), alloc)?,
        AnswerType::Object { kind } => {
            put(&mut map, "type", "object", alloc)?;
            put(&mut map, "kind", kind.as_str(), alloc)?;
        }
    }
    Ok(map.into())
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
        assert_eq!(kind.floor(), kind.size(), "every slot, optional ones too");
    }

    #[test]
    fn a_document_is_read_by_its_kinds() {
        let doc = map_of(vec![("kinds", list_of(vec![greeter()]))]);
        assert_eq!(KindDescription::document(&doc).expect("read").len(), 1);
        let noted = map_of(vec![
            ("x-note", text_value("a key this build does not know")),
            ("kinds", list_of(vec![greeter()])),
        ]);
        assert_eq!(
            KindDescription::document(&noted).expect("an unknown key is skipped"),
            KindDescription::document(&doc).unwrap()
        );
        let none = map_of(vec![("name", text_value("greeter"))]);
        assert_eq!(KindDescription::document(&none).unwrap_err().at(), "kinds");
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

    /// `bytes_mut` crosses on an object kind only, as `#[guatiao::kind]`
    /// says; whether a slot is required has nothing to do with its place.
    #[test]
    fn bytes_mut_is_an_object_kinds_and_slots_are_in_any_order() {
        let kind = |object: bool, required: [bool; 3]| {
            let slot = |name: &str, required: bool, ty: &str| {
                map_of(vec![
                    ("name", text_value(name)),
                    ("required", Value::from(required)),
                    ("fallible", Value::from(false)),
                    (
                        "args",
                        list_of(vec![map_of(vec![
                            ("name", text_value("dst")),
                            ("type", text_value(ty)),
                        ])]),
                    ),
                ])
            };
            map_of(vec![
                ("name", text_value("k")),
                ("floor_hash", Value::from(1u32)),
                ("object", Value::from(object)),
                (
                    "slots",
                    list_of(vec![
                        slot("a", required[0], "bytes"),
                        slot("b", required[1], "str"),
                        slot("c", required[2], "bytes_mut"),
                    ]),
                ),
            ])
        };
        let e = KindDescription::from_value(&kind(false, [true; 3])).unwrap_err();
        assert_eq!(e.at(), "slots[2].args[0].type");
        assert!(e.why().contains("on an object kind only"), "{e}");
        let read = KindDescription::from_value(&kind(true, [true, false, true])).expect("read");
        let required: Vec<bool> = read.slots().iter().map(|s| s.required()).collect();
        assert_eq!(required, [true, false, true]);
    }

    /// What `#[guatiao::kind]` cannot declare: an object on a slot that
    /// cannot fail, a borrowed list, a slot named `header`; each refused
    /// where it is said.
    #[test]
    fn what_no_trait_declares_is_refused_where_it_is_said() {
        let kind = |slot: Value| {
            map_of(vec![
                ("name", text_value("k")),
                ("floor_hash", Value::from(1u32)),
                ("object", Value::from(false)),
                ("slots", list_of(vec![slot])),
            ])
        };
        let slot = |name: &str, fallible: bool, args: Vec<Value>, answer: Option<&str>| {
            let mut entries = vec![
                ("name", text_value(name)),
                ("required", Value::from(true)),
                ("fallible", Value::from(fallible)),
                ("args", list_of(args)),
            ];
            if let Some(kind) = answer {
                entries.push((
                    "answer",
                    map_of(vec![
                        ("type", text_value("object")),
                        ("kind", text_value(kind)),
                    ]),
                ));
            }
            map_of(entries)
        };
        let object = |pass: &str| {
            map_of(vec![
                ("name", text_value("memo")),
                ("type", text_value("object")),
                ("kind", text_value("memo")),
                ("pass", text_value(pass)),
            ])
        };
        let list = |pass: &str| {
            map_of(vec![
                ("name", text_value("items")),
                ("type", text_value("list")),
                ("pass", text_value(pass)),
            ])
        };
        for (description, at, why) in [
            (
                slot("f", false, vec![text_arg(), object("lent")], None),
                "slots[0].args[1].type",
                "on a fallible slot only",
            ),
            (
                slot("f", false, vec![object("taken")], None),
                "slots[0].args[0].type",
                "on a fallible slot only",
            ),
            (
                slot("f", false, vec![], Some("memo")),
                "slots[0].answer.type",
                "on a fallible slot only",
            ),
            (
                slot("f", true, vec![list("borrowed")], None),
                "slots[0].args[0].pass",
                "a list is not borrowed",
            ),
            (
                slot("header", true, vec![], None),
                "slots[0]",
                "the table's first field",
            ),
        ] {
            let e = KindDescription::from_value(&kind(description)).unwrap_err();
            assert_eq!(e.at(), at, "{e}");
            assert!(e.why().contains(why), "{e}");
        }
        for fine in [
            slot(
                "f",
                true,
                vec![object("lent"), object("taken")],
                Some("memo"),
            ),
            slot("f", false, vec![list("lent"), list("taken")], None),
        ] {
            KindDescription::from_value(&kind(fine)).expect("a kind");
        }
    }

    fn text_arg() -> Value {
        map_of(vec![("name", text_value("s")), ("type", text_value("str"))])
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

    /// Each kind of the committed document, read and written back, is the
    /// value it was read from, key for key and in order: the writer spells
    /// a kind as the kind macro does.
    #[test]
    fn a_kind_written_back_is_the_value_it_was_read_from() {
        let document = KindDescription::document_value(
            include_str!("../../../../../examples/greeter_kind/include/greeter_kind.kinds.json"),
            Alloc::rust(),
        )
        .expect("the committed document reads");
        let kinds = TryAsRef::<Map>::try_as_ref(&document)
            .and_then(|m| m.get("kinds"))
            .and_then(TryAsRef::<List>::try_as_ref)
            .expect("a list of kinds");
        assert!(kinds.len() >= 5);
        for value in kinds.iter() {
            let kind = KindDescription::from_value(value).expect("a kind");
            let written = kind.to_value().expect("written");
            assert_eq!(written, *value, "`{}`", kind.name());
            assert_eq!(KindDescription::from_value(&written), Ok(kind));
        }
    }

    /// What the committed document has no example of: a nullable value,
    /// a scalar answer, a kind with no docs.
    #[test]
    fn a_nullable_value_and_a_scalar_answer_are_written_back() {
        let arg = |name: &str, ty: &str, extra: Vec<(&str, Value)>| {
            let mut entries = vec![("name", text_value(name)), ("type", text_value(ty))];
            entries.extend(extra);
            map_of(entries)
        };
        let kind = map_of(vec![
            ("name", text_value("probe")),
            ("floor_hash", Value::from(7u32)),
            ("object", Value::from(false)),
            (
                "slots",
                list_of(vec![map_of(vec![
                    ("name", text_value("weigh")),
                    ("required", Value::from(true)),
                    ("fallible", Value::from(true)),
                    (
                        "args",
                        list_of(vec![
                            arg(
                                "maybe",
                                "value",
                                vec![
                                    ("pass", text_value("borrowed")),
                                    ("nullable", Value::from(true)),
                                ],
                            ),
                            arg("items", "list", vec![("pass", text_value("lent"))]),
                            arg(
                                "who",
                                "object",
                                vec![
                                    ("kind", text_value("listener")),
                                    ("pass", text_value("lent")),
                                ],
                            ),
                            arg("scale", "f64", vec![]),
                        ]),
                    ),
                    ("answer", map_of(vec![("type", text_value("u64"))])),
                ])]),
            ),
        ]);
        let read = KindDescription::from_value(&kind).expect("a kind");
        assert_eq!(read.to_value().expect("written"), kind);
    }
}
