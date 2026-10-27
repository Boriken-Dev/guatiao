// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library's code calling a host's object: what its handle holds, and
//! its values as the slot's arguments. An engine reads its own values
//! (into a [`Reading`], a [`Value`], text or bytes) and builds the rest
//! from here.

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, Arg, Caller, DescribedRemote, LentHandle, LentObject, Pass, RawObject, Scalar, Tree,
};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status, Value};

/// What a script's handle on a host's object calls.
#[derive(Debug)]
pub enum Held {
    /// A provider's table, held loaded.
    Provider(DescribedRemote),
    /// An object handed over, until it is handed on or closed.
    Object(Option<RawObject>),
    /// An object lent for a call that may have returned.
    Lent(LentHandle),
}

impl Held {
    /// Calls slot `slot` through `caller`, the held object's kind's.
    pub fn call(
        &self,
        caller: &Caller,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        match self {
            Held::Provider(remote) => remote.call(slot, args),
            Held::Object(Some(object)) => object.lend().call(caller, slot, args),
            Held::Object(None) => Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the object was handed over or closed",
            )),
            Held::Lent(handle) => handle.call(caller, slot, args),
        }
    }

    /// The object, taken out to answer a slot that answers one of `kind`.
    pub fn hand_back(&mut self, caller: &Caller, kind: &str) -> Result<RawObject, ProviderError> {
        let held = caller.kind().name();
        if held != kind {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                &format!("an object of `{kind}` is answered, not of `{held}`"),
            ));
        }
        match self {
            Held::Object(object) => object.take().ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_GONE,
                    "the object was already handed over",
                )
            }),
            Held::Lent(_) => Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                "an object lent to this call cannot be handed back",
            )),
            Held::Provider(_) => Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                "a provider is not an object to hand back",
            )),
        }
    }

    /// This handle as an argument of an object of `kind` crossing as
    /// `pass`: its object taken out (handed over, or lent and put back
    /// after the call), or a lent object passed on. `home` is the handle
    /// it came out of, as the engine holds it.
    pub fn argument<T, B, O>(
        &mut self,
        caller: &Caller,
        kind: &str,
        pass: Pass,
        home: O,
        what: &str,
    ) -> Result<PreparedArg<T, B, O>, String> {
        let held = caller.kind().name();
        if held != kind {
            return Err(format!("{what} is a `{kind}`, not a `{held}`"));
        }
        match (self, pass) {
            (Held::Object(object), pass) => {
                let taken = object
                    .take()
                    .ok_or_else(|| format!("{what} was already handed over"))?;
                Ok(if pass == Pass::Taken {
                    PreparedArg::Object(Some(taken), home)
                } else {
                    PreparedArg::LentOwned(Some(taken), home)
                })
            }
            (Held::Lent(lent), Pass::Lent) => Ok(PreparedArg::LentPassed(lent.clone())),
            (Held::Lent(_), _) => Err(format!(
                "{what} was lent to this call, and cannot be handed over"
            )),
            (Held::Provider(_), _) => Err(format!("{what} is a provider, not an object")),
        }
    }

    /// Puts back an object lent for a call. Dropped (destroyed) when the
    /// handle no longer holds an object.
    pub fn put_back(&mut self, object: RawObject) {
        if let Held::Object(slot) = self {
            *slot = Some(object);
        }
    }
}

/// One argument of a slot a library calls, held while the call runs.
///
/// `T` is what a lent tree is written back into after the call, `B` what
/// a buffer is, `O` the handle an object came out of; an engine that has
/// none of them leaves them `()`.
#[derive(Debug)]
pub enum PreparedArg<T = (), B = (), O = ()> {
    /// `bool`.
    Bool(bool),
    /// A signed whole number.
    Int(i64),
    /// An unsigned whole number.
    Uint(u64),
    /// A float.
    Float(f64),
    /// Text.
    Str(String),
    /// Bytes, read.
    Bytes(Vec<u8>),
    /// Bytes the callee writes into, and where they go after the call.
    Buffer(Vec<u8>, Option<B>),
    /// A borrowed value, `None` for null.
    Value(Option<Value>),
    /// A borrowed map.
    Map(Map),
    /// A borrowed list.
    List(List),
    /// A value handed over.
    TakenValue(Option<Value>),
    /// A map handed over.
    TakenMap(Option<Map>),
    /// A list handed over.
    TakenList(Option<List>),
    /// A value lent, and where it is written back.
    LentValue(Value, Option<T>),
    /// A map lent, and where it is written back.
    LentMap(Map, Option<T>),
    /// A list lent, and where it is written back.
    LentList(List, Option<T>),
    /// An object handed over, with the handle it came out of.
    Object(Option<RawObject>, O),
    /// An object this side owns, lent for the call and put back after.
    LentOwned(Option<RawObject>, O),
    /// A lent object passed on.
    LentPassed(LentHandle),
}

/// A script's value as a scalar argument reads it.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// A flag.
    Bool(bool),
    /// A whole number the engine holds as one.
    Int(i64),
    /// A float.
    Float(f64),
    /// A number held as its text (a big integer, an exact decimal).
    Exact(String),
    /// Anything else.
    Other,
}

impl<T, B, O> PreparedArg<T, B, O> {
    /// `reading` as an argument of scalar type `ty`. `shown` names the
    /// script's value in a refusal (`a string`), `what` the argument.
    pub fn scalar(reading: Reading, shown: &str, ty: Scalar, what: &str) -> Result<Self, String> {
        Ok(match ty {
            Scalar::Bool => match reading {
                Reading::Bool(b) => PreparedArg::Bool(b),
                _ => return Err(format!("{what} is a boolean, not a {shown}")),
            },
            Scalar::F32 | Scalar::F64 => match reading {
                Reading::Int(i) => PreparedArg::Float(i as f64),
                Reading::Float(f) => PreparedArg::Float(f),
                Reading::Exact(text) => PreparedArg::Float(
                    text.parse()
                        .map_err(|_| format!("{what}: `{text}` is not a number"))?,
                ),
                _ => return Err(format!("{what} is a number, not a {shown}")),
            },
            Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize => {
                let n = whole(reading, shown, what)?;
                PreparedArg::Uint(
                    u64::try_from(n).map_err(|_| format!("{what}: {n} does not fit"))?,
                )
            }
            _ => {
                let n = whole(reading, shown, what)?;
                PreparedArg::Int(i64::try_from(n).map_err(|_| format!("{what}: {n} does not fit"))?)
            }
        })
    }

    /// `value` as a tree argument crossing as `pass`; `home` is where a lent
    /// one is written back. A map or a list must be one.
    pub fn tree(
        value: Value,
        tree: Tree,
        pass: Pass,
        home: Option<T>,
        what: &str,
    ) -> Result<Self, String> {
        let fits = match tree {
            Tree::Map => TryAsRef::<Map>::try_as_ref(&value).is_some(),
            Tree::List => TryAsRef::<List>::try_as_ref(&value).is_some(),
            _ => true,
        };
        if !fits {
            return Err(format!(
                "{what} is a {}",
                if tree == Tree::Map { "map" } else { "list" }
            ));
        }
        Ok(match (tree, pass) {
            (Tree::Map, Pass::Borrowed) => {
                PreparedArg::Map(Map::try_from(value).unwrap_or_default())
            }
            (Tree::List, Pass::Borrowed) => {
                PreparedArg::List(List::try_from(value).unwrap_or_default())
            }
            (_, Pass::Borrowed) => PreparedArg::Value(Some(value)),
            (Tree::Map, Pass::Taken) => PreparedArg::TakenMap(Map::try_from(value).ok()),
            (Tree::List, Pass::Taken) => PreparedArg::TakenList(List::try_from(value).ok()),
            (_, Pass::Taken) => PreparedArg::TakenValue(Some(value)),
            (Tree::Map, _) => PreparedArg::LentMap(Map::try_from(value).unwrap_or_default(), home),
            (Tree::List, _) => {
                PreparedArg::LentList(List::try_from(value).unwrap_or_default(), home)
            }
            (_, _) => PreparedArg::LentValue(value, home),
        })
    }

    /// Runs `f` with the arguments `prepared` holds, as core takes them. What
    /// was handed over is taken out of `prepared`; the rest stays, for the
    /// engine to settle after the call.
    pub fn with_args<R>(prepared: &mut [Self], f: impl FnOnce(Vec<Arg<'_>>) -> R) -> R {
        // A lent object passed on is lent for as long as its own lend lasts:
        // nest one `lend` per such argument.
        let passed: Vec<LentHandle> = prepared
            .iter()
            .filter_map(|p| match p {
                PreparedArg::LentPassed(h) => Some(h.clone()),
                _ => None,
            })
            .collect();
        lending(&passed, Vec::new(), move |lent| {
            let mut lent = lent.into_iter();
            let mut args = Vec::with_capacity(prepared.len());
            for p in prepared.iter_mut() {
                args.push(match p {
                    PreparedArg::Bool(b) => Arg::Bool(*b),
                    PreparedArg::Int(n) => Arg::Int(*n),
                    PreparedArg::Uint(n) => Arg::Uint(*n),
                    PreparedArg::Float(x) => Arg::Float(*x),
                    PreparedArg::Str(s) => Arg::Str(s),
                    PreparedArg::Bytes(b) => Arg::Bytes(b),
                    PreparedArg::Buffer(b, _) => Arg::Buffer(b),
                    PreparedArg::Value(v) => Arg::Value(v.as_ref()),
                    PreparedArg::Map(m) => Arg::Map(m),
                    PreparedArg::List(l) => Arg::List(l),
                    PreparedArg::TakenValue(v) => {
                        Arg::TakenValue(v.take().unwrap_or_else(Value::null))
                    }
                    PreparedArg::TakenMap(m) => Arg::TakenMap(m.take().unwrap_or_default()),
                    PreparedArg::TakenList(l) => Arg::TakenList(l.take().unwrap_or_default()),
                    PreparedArg::LentValue(v, _) => Arg::LentValue(v),
                    PreparedArg::LentMap(m, _) => Arg::LentMap(m),
                    PreparedArg::LentList(l, _) => Arg::LentList(l),
                    PreparedArg::Object(o, _) => match o.take() {
                        Some(o) => Arg::Object(o),
                        None => Arg::Value(None),
                    },
                    PreparedArg::LentOwned(Some(o), _) => Arg::LentObject(o.lend()),
                    PreparedArg::LentOwned(None, _) => Arg::Value(None),
                    PreparedArg::LentPassed(_) => match lent.next().flatten() {
                        Some(o) => Arg::LentObject(o),
                        None => Arg::Value(None),
                    },
                });
            }
            f(args)
        })
    }
}

/// A whole number, from a float only when it is one and fits 64 bits
/// (2^64 is about 1.8e19).
fn whole(reading: Reading, shown: &str, what: &str) -> Result<i128, String> {
    match reading {
        Reading::Int(i) => Ok(i128::from(i)),
        Reading::Float(f) if f.fract() == 0.0 && f.abs() < 1.8e19 => Ok(f as i128),
        Reading::Float(f) => Err(format!("{what}: {f} is not a whole number")),
        Reading::Exact(text) => text
            .parse::<i128>()
            .map_err(|_| format!("{what}: `{text}` is not a whole number")),
        _ => Err(format!("{what} is a whole number, not a {shown}")),
    }
}

/// Lends each of `handles` in turn (each lend lives only inside its
/// closure), then runs `f` with all of them.
fn lending<R>(
    handles: &[LentHandle],
    lent: Vec<Option<LentObject<'_>>>,
    f: impl FnOnce(Vec<Option<LentObject<'_>>>) -> R,
) -> R {
    match handles.split_first() {
        None => f(lent),
        Some((first, rest)) => first.lend(|object| {
            let mut all = lent;
            all.push(object);
            lending(rest, all, f)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Plain = PreparedArg;

    fn scalar_of(reading: Reading, ty: Scalar) -> Result<Plain, String> {
        Plain::scalar(reading, "string", ty, "`f`'s argument `x`")
    }

    #[test]
    fn a_whole_number_is_read_from_every_spelling_of_one() {
        for reading in [
            Reading::Int(7),
            Reading::Float(7.0),
            Reading::Exact("7".into()),
        ] {
            assert!(matches!(
                scalar_of(reading.clone(), Scalar::U8),
                Ok(PreparedArg::Uint(7))
            ));
            assert!(matches!(
                scalar_of(reading, Scalar::I32),
                Ok(PreparedArg::Int(7))
            ));
        }
        let big = Reading::Exact("18446744073709551615".into());
        assert!(matches!(
            scalar_of(big, Scalar::U64),
            Ok(PreparedArg::Uint(u64::MAX))
        ));
    }

    #[test]
    fn what_is_no_whole_number_or_does_not_fit_is_refused_in_words() {
        let refused = |reading, ty| scalar_of(reading, ty).expect_err("refused");
        assert_eq!(
            refused(Reading::Float(1.5), Scalar::I64),
            "`f`'s argument `x`: 1.5 is not a whole number"
        );
        assert_eq!(
            refused(Reading::Float(1.8e19), Scalar::U64),
            "`f`'s argument `x`: 18000000000000000000 is not a whole number"
        );
        assert_eq!(
            refused(Reading::Int(-1), Scalar::U32),
            "`f`'s argument `x`: -1 does not fit"
        );
        assert_eq!(
            refused(Reading::Exact("1.5".into()), Scalar::I64),
            "`f`'s argument `x`: `1.5` is not a whole number"
        );
        assert_eq!(
            refused(Reading::Other, Scalar::I64),
            "`f`'s argument `x` is a whole number, not a string"
        );
        assert_eq!(
            refused(Reading::Int(1), Scalar::Bool),
            "`f`'s argument `x` is a boolean, not a string"
        );
        assert_eq!(
            refused(Reading::Bool(true), Scalar::F64),
            "`f`'s argument `x` is a number, not a string"
        );
    }

    #[test]
    fn a_float_is_read_from_any_number() {
        for (reading, want) in [
            (Reading::Int(2), 2.0),
            (Reading::Float(2.5), 2.5),
            (Reading::Exact("1.10".into()), 1.1),
        ] {
            match scalar_of(reading, Scalar::F64) {
                Ok(PreparedArg::Float(f)) => assert_eq!(f, want),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_tree_crosses_as_its_pass_says_and_a_map_must_be_one() {
        let map = || Value::from(Map::new());
        let at = "`f`'s argument `m`";
        assert!(matches!(
            PreparedArg::<(), (), ()>::tree(map(), Tree::Map, Pass::Borrowed, None, at),
            Ok(PreparedArg::Map(_))
        ));
        assert!(matches!(
            PreparedArg::<(), (), ()>::tree(map(), Tree::Map, Pass::Taken, None, at),
            Ok(PreparedArg::TakenMap(Some(_)))
        ));
        assert!(matches!(
            PreparedArg::<u8, (), ()>::tree(map(), Tree::Map, Pass::Lent, Some(1), at),
            Ok(PreparedArg::LentMap(_, Some(1)))
        ));
        assert!(matches!(
            PreparedArg::<(), (), ()>::tree(Value::from(1i64), Tree::Value, Pass::Taken, None, at),
            Ok(PreparedArg::TakenValue(Some(_)))
        ));
        assert_eq!(
            PreparedArg::<(), (), ()>::tree(Value::from(1i64), Tree::Map, Pass::Borrowed, None, at)
                .expect_err("not a map"),
            "`f`'s argument `m` is a map"
        );
        assert_eq!(
            PreparedArg::<(), (), ()>::tree(map(), Tree::List, Pass::Taken, None, at)
                .expect_err("not a list"),
            "`f`'s argument `m` is a list"
        );
    }

    #[test]
    fn the_arguments_cross_in_order_and_what_is_handed_over_is_taken() {
        let mut prepared: Vec<Plain> = vec![
            PreparedArg::Bool(true),
            PreparedArg::Int(-2),
            PreparedArg::Str("hi".into()),
            PreparedArg::Value(None),
            PreparedArg::TakenValue(Some(Value::from(3i64))),
            PreparedArg::Object(None, ()),
        ];
        let shapes = PreparedArg::with_args(&mut prepared, |args| {
            args.iter()
                .map(|a| match a {
                    Arg::Bool(true) => "bool",
                    Arg::Int(-2) => "int",
                    Arg::Str("hi") => "str",
                    Arg::Value(None) => "null",
                    Arg::TakenValue(_) => "taken",
                    _ => "other",
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(shapes, ["bool", "int", "str", "null", "taken", "null"]);
        assert!(matches!(prepared[4], PreparedArg::TakenValue(None)));
    }
}
