// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Providers an engine declares, served through kind tables built from
//! their kinds' descriptions: every slot a [`Call`] into the engine.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use std::sync::OnceLock;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, AnswerType, Arg, KindDescription, LentHandle, LentObject, LibraryParts, Mechanism,
    ProviderParts, Serve, ServedTable, TableError,
};
use guatiao::value::types::{Buffer, Number};
use guatiao::{Alloc, List, Map, Status, Value};

use crate::engine::{Call, CallArg, Engine, Provided, Reply, Target};
use crate::error::BridgeError;
use crate::kinds::Kinds;
use crate::thread::Runner;

/// Every slot of one library's tables lands here.
struct Dispatch<E: Engine> {
    runner: Runner<E>,
    library: E::Library,
    kinds: Arc<Kinds>,
    /// Provider ids, by `ctx - 1`.
    providers: Vec<String>,
    /// The tables of the object kinds its slots answer. The library's parts
    /// keep them; a table holds this dispatch, so these are weak.
    objects: OnceLock<BTreeMap<String, Weak<ServedTable>>>,
}

/// What a lent argument is written back into once the call returns.
enum Back<'a> {
    None,
    Value(&'a mut Value),
    Map(&'a mut Map),
    List(&'a mut List),
    Buffer(&'a mut [u8]),
}

/// A call's arguments as the engine takes them, where each lent one is
/// written back, and the lent objects to hand over for the call.
type Converted<'a> = (Vec<CallArg>, Vec<Back<'a>>, Vec<(usize, LentObject<'a>)>);

fn refused(status: Status, why: &str) -> ProviderError {
    ProviderError::new(status, why)
}

/// The engine's copy of a caller's tree, built in the caller's allocator.
fn home_of(alloc: Result<Alloc, guatiao::ValueError>) -> Alloc {
    alloc.unwrap_or_else(|_| Alloc::rust())
}

impl<E: Engine> Dispatch<E> {
    fn target(&self, ctx: usize, kind: &KindDescription) -> Result<Target, ProviderError> {
        if kind.object() {
            return Ok(Target::Object(ctx as u64));
        }
        ctx.checked_sub(1)
            .and_then(|i| self.providers.get(i))
            .map(|id| Target::Provider(id.clone()))
            .ok_or_else(|| refused(Status::GUATIAO_ERR_NULL, "no such provider"))
    }

    /// The engine's call, its arguments converted, and where each lent one
    /// is written back.
    fn convert(args: Vec<Arg<'_>>) -> Result<Converted<'_>, ProviderError> {
        let mut out = Vec::with_capacity(args.len());
        let mut backs = Vec::with_capacity(args.len());
        let mut lent = Vec::new();
        for (i, arg) in args.into_iter().enumerate() {
            let (one, back) = match arg {
                Arg::Bool(b) => (CallArg::Value(Value::from(b)), Back::None),
                Arg::Int(n) => (CallArg::Value(Value::from(n)), Back::None),
                Arg::Uint(n) => (CallArg::Value(Value::from(n)), Back::None),
                Arg::Float(f) => (
                    CallArg::Value(Value::from(Number::try_from(f).map_err(|_| {
                        refused(
                            Status::GUATIAO_ERR_BAD_VALUE,
                            "a number that is not finite has no value to cross as",
                        )
                    })?)),
                    Back::None,
                ),
                Arg::Str(s) => (CallArg::Value(Value::from(s)), Back::None),
                Arg::Bytes(b) => (CallArg::Value(Value::from(Buffer::from(b))), Back::None),
                Arg::Buffer(b) => (CallArg::Buffer(b.to_vec()), Back::Buffer(b)),
                Arg::Value(v) => (
                    CallArg::Value(v.map_or_else(Value::null, Clone::clone)),
                    Back::None,
                ),
                Arg::Map(m) => (CallArg::Value(Value::from(m.clone())), Back::None),
                Arg::List(l) => (CallArg::Value(Value::from(l.clone())), Back::None),
                Arg::TakenValue(v) => (CallArg::Value(v), Back::None),
                Arg::TakenMap(m) => (CallArg::Value(Value::from(m)), Back::None),
                Arg::TakenList(l) => (CallArg::Value(Value::from(l)), Back::None),
                Arg::LentValue(v) => (CallArg::Lent(v.clone()), Back::Value(v)),
                Arg::LentMap(m) => (CallArg::Lent(Value::from(m.clone())), Back::Map(m)),
                Arg::LentList(l) => (CallArg::Lent(Value::from(l.clone())), Back::List(l)),
                Arg::Object(o) => (CallArg::Object(o), Back::None),
                Arg::LentObject(o) => {
                    lent.push((i, o));
                    // Replaced by the handle before the call.
                    (CallArg::Value(Value::null()), Back::None)
                }
                _ => {
                    return Err(refused(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "an argument this bridge does not carry",
                    ));
                }
            };
            out.push(one);
            backs.push(back);
        }
        Ok((out, backs, lent))
    }

    /// Writes each lent argument the engine changed back over the caller's.
    fn write_back(call: Call, backs: Vec<Back<'_>>) -> Result<(), ProviderError> {
        for (arg, back) in call.args.into_iter().zip(backs) {
            match (back, arg) {
                (Back::Value(v), CallArg::Lent(new)) => {
                    let home = home_of(v.alloc());
                    *v = new.into_alloc(home)?;
                }
                (Back::Map(m), CallArg::Lent(new)) => {
                    let home = home_of(m.alloc());
                    *m = Map::try_from(new)
                        .map_err(|_| {
                            refused(
                                Status::GUATIAO_ERR_WRONG_KIND,
                                "the engine left the map it was lent as something else",
                            )
                        })?
                        .into_alloc(home)?;
                }
                (Back::List(l), CallArg::Lent(new)) => {
                    let home = home_of(l.alloc());
                    *l = List::try_from(new)
                        .map_err(|_| {
                            refused(
                                Status::GUATIAO_ERR_WRONG_KIND,
                                "the engine left the list it was lent as something else",
                            )
                        })?
                        .into_alloc(home)?;
                }
                (Back::Buffer(b), CallArg::Buffer(new)) => {
                    let n = b.len().min(new.len());
                    b[..n].copy_from_slice(&new[..n]);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn answer(&self, reply: Reply) -> Result<Answer, ProviderError> {
        Ok(match reply {
            Reply::Nothing => Answer::Nothing,
            Reply::Value(v) => Answer::Value(v),
            Reply::Handed(object) => Answer::Object(object),
            Reply::Object { kind, id } => {
                let table = self
                    .objects
                    .get()
                    .and_then(|objects| objects.get(&kind))
                    .and_then(Weak::upgrade)
                    .ok_or_else(|| {
                        refused(
                            Status::GUATIAO_ERR_NOT_FOUND,
                            &format!("the engine answered an object of `{kind}`, which none of this library's slots answers"),
                        )
                    })?;
                Answer::Object(table.object(id as usize).ok_or_else(|| {
                    refused(
                        Status::GUATIAO_ERR_WRONG_KIND,
                        &format!("`{kind}` is not an object kind"),
                    )
                })?)
            }
        })
    }

    fn run(&self, mut call: Call) -> Result<(Call, Reply), ProviderError> {
        let library = self.library.clone();
        let (call, reply) = self.runner.run(move |e| {
            let reply = e.call(&library, &mut call);
            (call, reply)
        })?;
        Ok((call, reply?))
    }
}

/// Runs `f` with a handle on each lent object, each lend ending when `f`
/// returns.
fn sharing<R>(
    mut lent: Vec<(usize, LentObject<'_>)>,
    mut handles: Vec<(usize, LentHandle)>,
    f: impl FnOnce(Vec<(usize, LentHandle)>) -> R,
) -> R {
    match lent.pop() {
        None => f(handles),
        Some((i, object)) => object.share(move |handle| {
            handles.push((i, handle));
            sharing(lent, handles, f)
        }),
    }
}

impl<E: Engine> Serve for Dispatch<E> {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let target = self.target(ctx, kind)?;
        let described = self
            .kinds
            .get(kind.name())
            .cloned()
            .ok_or_else(|| ProviderError::from(BridgeError::UnknownKind(kind.name().into())))?;
        let (call_args, backs, lent) = Self::convert(args)?;
        let (call, reply) = sharing(lent, Vec::new(), |handles| {
            let mut call_args = call_args;
            for (i, handle) in handles {
                call_args[i] = CallArg::LentObject(handle);
            }
            self.run(Call::new(target, described, slot, call_args))
        })?;
        Self::write_back(call, backs)?;
        self.answer(reply)
    }

    fn destroy(&self, ctx: usize) {
        let library = self.library.clone();
        let _ = self.runner.run(move |e| e.release(&library, ctx as u64));
    }
}

/// The object kinds `kind`'s slots answer, and theirs in turn.
fn answered_objects(kinds: &Kinds, kind: &KindDescription, into: &mut Vec<String>) {
    for slot in kind.slots() {
        if let Some(AnswerType::Object { kind: object }) = slot.answer()
            && !into.contains(object)
        {
            into.push(object.clone());
            if let Some(described) = kinds.get(object) {
                answered_objects(kinds, &described.clone(), into);
            }
        }
    }
}

fn table_error(provider: &str, kind: &str, e: TableError) -> BridgeError {
    match e {
        TableError::MissingRequired(slot) => BridgeError::Table(format!(
            "`{provider}` does not implement `{slot}`, which the `{kind}` kind requires"
        )),
        other => BridgeError::Table(format!("`{provider}` cannot serve `{kind}`: {other}")),
    }
}

/// The library an engine opened as described providers, its tables built
/// from `kinds` by `mechanism`, every slot a call through `runner`.
pub(crate) fn library<E: Engine>(
    runner: Runner<E>,
    kinds: &Arc<Kinds>,
    mechanism: &dyn Mechanism,
    id: &str,
    version: &str,
    library: E::Library,
    providers: Vec<Provided>,
) -> Result<LibraryParts, BridgeError> {
    let dispatch = Arc::new(Dispatch {
        runner,
        library,
        kinds: Arc::clone(kinds),
        providers: providers.iter().map(|p| p.id().to_string()).collect(),
        objects: OnceLock::new(),
    });
    let serve: Arc<dyn Serve> = dispatch.clone();

    let mut objects = Vec::new();
    for provider in &providers {
        for (kind, _) in provider.kinds() {
            let described = kinds
                .get(kind)
                .ok_or_else(|| BridgeError::UnknownKind(kind.clone()))?;
            answered_objects(kinds, described, &mut objects);
        }
    }
    let mut object_tables = Vec::new();
    let mut weak = BTreeMap::new();
    for object in objects {
        let Some(described) = kinds.get(&object) else {
            continue;
        };
        let table = Arc::new(
            ServedTable::new(
                Arc::clone(described),
                |_| true,
                Arc::clone(&serve),
                mechanism,
            )
            .map_err(|e| table_error(id, &object, e))?,
        );
        weak.insert(object, Arc::downgrade(&table));
        object_tables.push(table);
    }
    let _ = dispatch.objects.set(weak);

    let mut parts = Vec::new();
    for (at, provider) in providers.iter().enumerate() {
        let mut builder = ProviderParts::builder(provider.id()).name(provider.display_name());
        let mut tables = Vec::new();
        for (kind, implements) in provider.kinds() {
            let described = kinds
                .get(kind)
                .ok_or_else(|| BridgeError::UnknownKind(kind.clone()))?;
            if described.object() {
                return Err(BridgeError::Table(format!(
                    "`{}` declares the object kind `{kind}`, which no provider serves",
                    provider.id()
                )));
            }
            let table = ServedTable::new(
                Arc::clone(described),
                |s| implements.iter().any(|m| m == s.name()),
                Arc::clone(&serve),
                mechanism,
            )
            .map_err(|e| table_error(provider.id(), kind, e))?;
            builder = builder.kinds(&[kind.as_str()]).table(table.kind_table());
            tables.push(table);
        }
        builder = builder.ctx(core::ptr::without_provenance_mut(at + 1));
        if let Some(meta) = provider.declared_meta() {
            builder = builder.meta(meta.clone());
        }
        for table in tables {
            builder = builder.keep(table);
        }
        if at == 0 {
            // The object tables live as long as the library's first
            // provider's parts, which is as long as the library's.
            for table in object_tables.drain(..) {
                builder = builder.keep(table);
            }
        }
        parts.push(builder.build());
    }
    Ok(LibraryParts::new(id, version, parts))
}
