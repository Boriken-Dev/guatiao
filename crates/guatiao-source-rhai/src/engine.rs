// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai as a guatiao-bridge engine: a library is a script whose last
//! expression is a map of providers, its manifest a
//! `*.guatiao` of type `rhai`, each provider a map whose function members are
//! the slots, served through tables the bridge builds from the kinds'
//! descriptions.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::path::PathBuf;

use guatiao::library::ProviderError;
use guatiao::library::kind::{AnswerType, SlotDescription, Tree};
use guatiao::{Status, Value};
use guatiao_bridge::{
    BridgeError, Call, CallArg, Config, Engine, Manifest, ManifestFormat, Opened, Provided, Reply,
    Target, ThreadModel,
};
use rhai::{AST, Array, Dynamic, FnPtr, ImmutableString, Map as RhaiMap, Scope};

use crate::calls::{self, Handle, Known, provider_error};
use crate::helpers;
use crate::limits::Meter;
use crate::values;

/// A provider's or an object's state: the map its slots are members of,
/// which is `this` in each of them.
type State = Rc<RefCell<Dynamic>>;

/// An instance a provider's `create` built: its state, which is `this` in
/// its slots, and the provider's functions as they were when it was built,
/// which are its slots.
struct Instance {
    state: State,
    slots: RhaiMap,
}

/// One library opened: its script, its providers by id, and the objects its
/// slots answered and the instances its providers built, by the id each was
/// given. Every provider shares the one compiled script.
struct Library {
    ast: AST,
    providers: BTreeMap<String, State>,
    objects: RefCell<BTreeMap<u64, State>>,
    instances: RefCell<BTreeMap<u64, Rc<Instance>>>,
}

/// The provider-map keys that say how it is built and asked, not slots.
const CREATE: &str = "create";
const AVAILABLE: &str = "available";

/// What a script value is, in the words a refusal uses.
fn what(d: &Dynamic) -> String {
    if d.is_unit() {
        "()".into()
    } else if d.is_bool() {
        "boolean".into()
    } else if d.is_int() || d.is_float() {
        "number".into()
    } else if d.is_string() || d.is_char() {
        "string".into()
    } else if d.is_map() {
        "map".into()
    } else if d.is_array() {
        "array".into()
    } else if d.is::<FnPtr>() {
        "function".into()
    } else {
        values::type_name(d)
    }
}

/// A provider's or an instance's state, borrowed for one call: refused
/// `BUSY` while a call into it is running.
fn running(state: &State) -> Result<core::cell::RefMut<'_, Dynamic>, ProviderError> {
    state.try_borrow_mut().map_err(|_| {
        ProviderError::new(
            Status::GUATIAO_ERR_BUSY,
            "the provider is already running: a call re-entered it",
        )
    })
}

/// The engine: one Rhai engine, on the bridge's thread for the life of the
/// source. Not `Send`.
pub struct RhaiEngine {
    engine: rhai::Engine,
    meter: Meter,
    known: Rc<Known>,
    libraries: RefCell<BTreeMap<u64, Rc<Library>>>,
    next: Cell<u64>,
}

fn engine_error(id: &str, e: &rhai::EvalAltResult, meter: &Meter) -> BridgeError {
    BridgeError::engine(format!(
        "{id}: {}",
        provider_error(e, &meter.limits).message()
    ))
}

fn refused(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// The file a library's script is in on `paths`: `a.b` is `a/b.rhai`, then
/// `a/b/init.rhai`.
fn script_file(module: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    let rel = module.replace('.', "/");
    paths.iter().find_map(|dir| {
        [
            dir.join(format!("{rel}.rhai")),
            dir.join(&rel).join("init.rhai"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    })
}

/// Where a manifest says its code is.
fn paths_of(manifest: &Manifest) -> Vec<PathBuf> {
    manifest.paths().iter().map(PathBuf::from).collect()
}

/// The text of the script a manifest names.
fn script_of(manifest: &Manifest) -> Result<(PathBuf, String), BridgeError> {
    let file = script_file(manifest.id(), &paths_of(manifest)).ok_or_else(|| {
        BridgeError::engine(format!("{} is not on the manifest's paths", manifest.id()))
    })?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
    Ok((file, text))
}

/// The `VERSION` a script declares as a literal at its top, which parsing
/// finds without running anything.
fn declared_version(ast: &AST) -> Option<String> {
    ast.iter_literal_variables(true, true)
        .find(|(name, ..)| *name == "VERSION")
        .and_then(|(.., value)| value.into_string().ok())
}

/// What a slot's answer is, and the lent arguments the script changed.
enum Outcome {
    Nothing,
    Value(Value),
    /// A map of an object kind's slots: the object it answers.
    Object(String, Dynamic),
    /// An object handed back as it came.
    Handed(guatiao::library::kind::RawObject),
}

/// What a lent argument is read back from after the call.
enum Back {
    None,
    Tree(Dynamic),
    Buffer(Dynamic),
}

impl RhaiEngine {
    fn library(&self, handle: u64) -> Result<Rc<Library>, ProviderError> {
        self.libraries
            .borrow()
            .get(&handle)
            .cloned()
            .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_GONE, "the library was closed"))
    }

    /// The function member `slot` of a provider's or object's map.
    fn method(state: &Dynamic, slot: &str) -> Result<FnPtr, ProviderError> {
        state
            .read_lock::<RhaiMap>()
            .and_then(|m| Self::member(&m, slot))
            .ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    &format!("no method `{slot}`"),
                )
            })
    }

    /// The function `name` holds in `map`, if it is one.
    fn member(map: &RhaiMap, name: &str) -> Option<FnPtr> {
        map.get(name).and_then(|f| f.clone().try_cast::<FnPtr>())
    }

    /// Checks a value the host passes against the size limits.
    fn within_size(&self, v: &Dynamic) -> Result<(), ProviderError> {
        if self.engine.ensure_data_size_within_limits(v).is_err() {
            return Err(BridgeError::Limit {
                limit: guatiao_bridge::Limit::Memory,
                at: self.meter.limits.memory.unwrap_or(0),
            }
            .into());
        }
        Ok(())
    }

    /// A call's arguments as the script takes them, and what each lent one
    /// is read back from.
    fn arguments(
        &self,
        call: &mut Call,
        slot: &SlotDescription,
    ) -> Result<(Vec<Dynamic>, Vec<Back>), ProviderError> {
        let bad = |why: String| refused(why);
        let mut values = Vec::with_capacity(call.args.len());
        let mut back = Vec::with_capacity(call.args.len());
        for (arg, d) in call.args.iter_mut().zip(slot.args()) {
            let (v, b) = match arg {
                CallArg::Value(v) => (values::to_dynamic(v).map_err(bad)?, Back::None),
                CallArg::Lent(v) => {
                    // Shared, so what the script does to its argument is
                    // seen from here.
                    let shared = values::to_dynamic(v).map_err(bad)?.into_shared();
                    (shared.clone(), Back::Tree(shared))
                }
                CallArg::Buffer(b) => {
                    let shared = Dynamic::from_blob(core::mem::take(b)).into_shared();
                    (shared.clone(), Back::Buffer(shared))
                }
                CallArg::Object(_) => {
                    let CallArg::Object(o) = core::mem::replace(arg, CallArg::Value(Value::null()))
                    else {
                        unreachable!("matched just above")
                    };
                    (
                        self.object_handle(d.ty(), |c| Handle::object(c, o))?,
                        Back::None,
                    )
                }
                CallArg::LentObject(h) => {
                    let h = h.clone();
                    (
                        self.object_handle(d.ty(), |c| Handle::lent(c, h))?,
                        Back::None,
                    )
                }
                _ => {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "an argument this engine does not carry",
                    ));
                }
            };
            self.within_size(&v)?;
            values.push(v);
            back.push(b);
        }
        Ok((values, back))
    }

    /// A handle on an object argument of the kind `ty` names.
    fn object_handle(
        &self,
        ty: &guatiao::library::kind::ArgType,
        make: impl FnOnce(alloc::sync::Arc<guatiao::library::kind::Caller>) -> Handle,
    ) -> Result<Dynamic, ProviderError> {
        let guatiao::library::kind::ArgType::Object { kind, .. } = ty else {
            return Err(refused("an object was passed where the slot takes none"));
        };
        let caller = self.known.caller(kind).map_err(refused)?;
        Ok(Dynamic::from(make(caller)))
    }

    /// What the script answered to `slot`.
    fn outcome(slot: &SlotDescription, result: Dynamic) -> Result<Outcome, ProviderError> {
        let wrong = |why: String| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why);
        Ok(match slot.answer() {
            None => Outcome::Nothing,
            Some(AnswerType::Object { kind }) => {
                if result.is::<Handle>() {
                    let handle = result.cast::<Handle>();
                    Outcome::Handed(handle.take_object(kind)?)
                } else if result.is_map() {
                    Outcome::Object(kind.clone(), result)
                } else {
                    return Err(wrong(format!(
                        "`{}` answers an object of `{kind}` (a map of its methods), not a {}",
                        slot.name(),
                        values::type_name(&result)
                    )));
                }
            }
            Some(AnswerType::Text) => {
                if result.is_string() || result.is_char() {
                    Outcome::Value(values::to_value(&result).map_err(refused)?)
                } else {
                    return Err(wrong(format!(
                        "`{}` answers text, not a {}",
                        slot.name(),
                        values::type_name(&result)
                    )));
                }
            }
            Some(AnswerType::Tree(Tree::Map)) | Some(AnswerType::Tree(Tree::List)) | Some(_) => {
                Outcome::Value(values::to_value(&result).map_err(refused)?)
            }
        })
    }

    /// Converts a call's arguments for the slot, runs it, writes the lent
    /// ones back and answers the reply. The slot is `slots`' function when
    /// given (an instance's), else `state`'s own member.
    fn answer(
        &self,
        lib: &Library,
        state: &State,
        slots: Option<&RhaiMap>,
        call: &mut Call,
    ) -> Result<Reply, ProviderError> {
        let slot = call.slot_description().clone();
        let (values, back) = self.arguments(call, &slot)?;
        let mut this = running(state)?;
        // Everything the script made is read and dropped within the call,
        // the method's curried values included: Rhai copies and drops nested
        // values by recursion, in the stack the call keeps free for them.
        let outcome = self.meter.run(|| {
            let method = match slots {
                Some(slots) => Self::member(slots, slot.name()).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        &format!("no method `{}`", slot.name()),
                    )
                })?,
                None => Self::method(&this, slot.name())?,
            };
            let result = method
                .call_as_method::<Dynamic>(&self.engine, &lib.ast, &mut this, values)
                .map_err(|e| provider_error(&e, &self.meter.limits))?;
            for (arg, b) in call.args.iter_mut().zip(back) {
                match b {
                    Back::None => {}
                    Back::Tree(shared) => {
                        *arg = CallArg::Lent(values::to_value(&shared).map_err(refused)?);
                    }
                    Back::Buffer(shared) => {
                        let blob = shared.flatten_clone().into_blob().unwrap_or_default();
                        *arg = CallArg::Buffer(blob);
                    }
                }
            }
            Self::outcome(&slot, result)
        });
        drop(this);
        Ok(match outcome? {
            Outcome::Nothing => Reply::Nothing,
            Outcome::Value(v) => Reply::Value(v),
            Outcome::Handed(o) => Reply::Handed(o),
            Outcome::Object(kind, map) => {
                let id = self.next.get() + 1;
                self.next.set(id);
                lib.objects
                    .borrow_mut()
                    .insert(id, Rc::new(RefCell::new(map)));
                Reply::Object { kind, id }
            }
        })
    }

    /// What a provider's map says about building it and asking it:
    /// `config` (a JSON Schema as a map), `create` (a function of the
    /// configuration), `instances_only` (a boolean), `available` (a
    /// function). Each is optional; one of another type refuses the library.
    fn built_and_asked(
        id: &str,
        key: &str,
        members: &RhaiMap,
        mut declared: Provided,
    ) -> Result<Provided, BridgeError> {
        let wrong = |member: &str, value: &Dynamic, wanted: &str| {
            BridgeError::engine(format!(
                "{id}: the provider `{key}` has `{member}` a {}, not {wanted}",
                what(value)
            ))
        };
        let config = members.get("config");
        if let Some(schema) = config {
            if !schema.is_map() {
                return Err(wrong("config", schema, "a map (a JSON Schema)"));
            }
            let schema = values::to_value(schema)
                .map_err(|why| BridgeError::engine(format!("{id}: `{key}`'s `config`: {why}")))?;
            declared = declared.config(schema);
        }
        if let Some(create) = members.get(CREATE) {
            if !create.is::<FnPtr>() {
                return Err(wrong(CREATE, create, "a function"));
            }
            if config.is_none() {
                return Err(BridgeError::engine(format!(
                    "{id}: the provider `{key}` has `create` and no `config` to build from"
                )));
            }
        }
        if let Some(only) = members.get("instances_only") {
            match only.as_bool() {
                Ok(true) => declared = declared.instances_only(),
                Ok(false) => {}
                Err(_) => return Err(wrong("instances_only", only, "a boolean")),
            }
        }
        if let Some(available) = members.get(AVAILABLE) {
            if !available.is::<FnPtr>() {
                return Err(wrong(AVAILABLE, available, "a function"));
            }
            declared = declared.may_be_unavailable();
        }
        Ok(declared)
    }

    /// The providers a script's result names, each serving the kinds it
    /// lists with the slots it has functions for.
    fn declare(
        &self,
        manifest: &Manifest,
        result: Dynamic,
    ) -> Result<(Vec<Provided>, BTreeMap<String, State>), BridgeError> {
        let id = manifest.id();
        let Some(listed) = result
            .read_lock::<RhaiMap>()
            .and_then(|top| top.get("providers").cloned())
            .and_then(|p| p.try_cast::<RhaiMap>())
        else {
            return Err(BridgeError::engine(format!(
                "{id} ends with no `providers` map: a library's script ends with `#{{ providers: #{{ name: provider }} }}`"
            )));
        };
        let mut offered = Vec::new();
        let mut providers = BTreeMap::new();
        for (key, provider) in listed {
            let key = key.to_string();
            let Some(members) = provider.read_lock::<RhaiMap>().map(|m| m.clone()) else {
                return Err(BridgeError::engine(format!(
                    "{id}: `providers` holds a {} for `{key}`, not a provider map",
                    values::type_name(&provider)
                )));
            };
            let pid = format!("{}_{key}", id.replace('.', "_"));
            let kinds: Vec<String> = match members
                .get("kinds")
                .and_then(|k| k.read_lock::<Array>().map(|a| a.clone()))
            {
                Some(list) => list
                    .into_iter()
                    .map(|k| {
                        k.into_string().map_err(|_| {
                            BridgeError::engine(format!(
                                "{id}: the provider `{key}` lists a kind that is not text"
                            ))
                        })
                    })
                    .collect::<Result<_, _>>()?,
                None => {
                    return Err(BridgeError::engine(format!(
                        "{id}: the provider `{key}` names no `kinds`"
                    )));
                }
            };
            let name = members
                .get("name")
                .and_then(|n| n.read_lock::<ImmutableString>().map(|s| s.to_string()))
                .unwrap_or_else(|| key.clone());
            let mut declared = Provided::new(&pid).name(&name);
            for kind in &kinds {
                if !manifest.kinds().iter().any(|k| k == kind) {
                    return Err(BridgeError::engine(format!(
                        "{id}: the provider `{key}` serves `{kind}`, which the manifest does not list"
                    )));
                }
                let described = self
                    .known
                    .kind(kind)
                    .ok_or_else(|| BridgeError::UnknownKind(kind.clone()))?;
                let implements: Vec<String> = described
                    .slots()
                    .iter()
                    .filter(|slot| members.get(slot.name()).is_some_and(|f| f.is::<FnPtr>()))
                    .map(|slot| slot.name().to_string())
                    .collect();
                declared = declared.serves(kind, implements);
            }
            declared = Self::built_and_asked(id, &key, &members, declared)?;
            offered.push(declared);
            providers.insert(pid, Rc::new(RefCell::new(provider)));
        }
        Ok((offered, providers))
    }
}

impl Engine for RhaiEngine {
    type Library = u64;

    /// Rhai's engine and the values it hands out are not `Send` (and its
    /// `sync` build would only make them locks): one thread.
    const THREADS: ThreadModel = ThreadModel::Pinned;

    const MANIFESTS: ManifestFormat = ManifestFormat {
        serves: "rhai",
        id_key: "module",
        code_extensions: &["rhai"],
        skip_dirs: &[],
    };

    /// A Rhai engine with the kinds the configuration names, its limits,
    /// and no way to reach the file system or import a module.
    fn start(config: &Config) -> Result<RhaiEngine, BridgeError> {
        let mut kinds = guatiao_bridge::Kinds::new();
        for path in config.kinds() {
            kinds.load(path)?;
        }
        let known = Rc::new(Known::new(kinds));
        let mut engine = rhai::Engine::new();
        let meter = Meter::install(&mut engine, *config.limits())?;
        helpers::register(&mut engine);
        calls::register(&mut engine, &known);
        Ok(RhaiEngine {
            engine,
            meter,
            known,
            libraries: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
        })
    }

    /// Every call level the limits allow and, below the deepest, the
    /// values their operations can nest: reserved, and used only as deep
    /// as a script goes.
    fn stack_size(config: &Config) -> Option<usize> {
        crate::limits::thread_stack(config.limits())
    }

    /// The script's `const VERSION = "...";`, read from its parse: nothing
    /// is run.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        let (file, text) = script_of(manifest)?;
        let ast = self
            .engine
            .compile(&text)
            .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
        declared_version(&ast).ok_or_else(|| {
            BridgeError::engine(format!(
                "{} states no `const VERSION = \"...\";` at its top: give the manifest a `version`",
                file.display()
            ))
        })
    }

    /// Compiles the script afresh, runs it once and reads the providers it
    /// ends with.
    fn open(&self, manifest: &Manifest, version: &str) -> Result<Opened<u64>, BridgeError> {
        let id = manifest.id();
        let (file, text) = script_of(manifest)?;
        let mut ast = self
            .engine
            .compile(&text)
            .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
        ast.set_source(id);
        if let Some(stated) = declared_version(&ast)
            && stated != version
        {
            return Err(BridgeError::engine(format!(
                "{id} states `const VERSION = \"{stated}\"`, and its manifest {version}"
            )));
        }
        // What the script ends with is read, and what is not kept dropped,
        // within the call.
        let (offered, providers) = self.meter.run(|| {
            let result: Dynamic = self
                .engine
                .eval_ast_with_scope(&mut Scope::new(), &ast)
                .map_err(|e| engine_error(id, &e, &self.meter))?;
            self.declare(manifest, result)
        })?;
        let handle = self.next.get() + 1;
        self.next.set(handle);
        self.libraries.borrow_mut().insert(
            handle,
            Rc::new(Library {
                ast,
                providers,
                objects: RefCell::new(BTreeMap::new()),
                instances: RefCell::new(BTreeMap::new()),
            }),
        );
        Ok(Opened::Described {
            library: handle,
            providers: offered,
        })
    }

    fn call(&self, handle: &u64, call: &mut Call) -> Result<Reply, ProviderError> {
        let lib = self.library(*handle)?;
        let nothing = |call: &Call| refused(format!("nothing answers `{}`", call.method()));
        if let Target::Instance(id) = call.target() {
            let instance = lib.instances.borrow().get(id).cloned();
            let instance = instance.ok_or_else(|| nothing(call))?;
            return self.answer(&lib, &instance.state, Some(&instance.slots), call);
        }
        let state = match call.target() {
            Target::Provider(id) => lib.providers.get(id).cloned(),
            Target::Object(id) => lib.objects.borrow().get(id).cloned(),
            _ => None,
        }
        .ok_or_else(|| nothing(call))?;
        self.answer(&lib, &state, None, call)
    }

    /// Runs the provider's `create` with the configuration, under the
    /// limits, its map as `this`; without one, the configuration is the
    /// state. The provider's functions are its instance's slots, as they
    /// are now.
    fn create(&self, handle: &u64, provider: &str, config: &Value) -> Result<u64, ProviderError> {
        let lib = self.library(*handle)?;
        let state = lib
            .providers
            .get(provider)
            .cloned()
            .ok_or_else(|| refused(format!("no provider `{provider}`")))?;
        let config = values::to_dynamic(config).map_err(refused)?;
        self.within_size(&config)?;
        let mut this = running(&state)?;
        let built = self.meter.run(|| {
            let made = match Self::method(&this, CREATE) {
                Ok(create) => create
                    .call_as_method::<Dynamic>(&self.engine, &lib.ast, &mut this, (config,))
                    .map_err(|e| provider_error(&e, &self.meter.limits))?,
                Err(_) => config,
            };
            let slots: RhaiMap = this
                .read_lock::<RhaiMap>()
                .map(|m| {
                    m.iter()
                        .filter(|(_, f)| f.is::<FnPtr>())
                        .map(|(k, f)| (k.clone(), f.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Ok::<_, ProviderError>(Instance {
                state: Rc::new(RefCell::new(made)),
                slots,
            })
        });
        drop(this);
        let id = self.next.get() + 1;
        self.next.set(id);
        lib.instances.borrow_mut().insert(id, Rc::new(built?));
        Ok(id)
    }

    /// The instance's state goes, as an object's does.
    fn destroy(&self, handle: &u64, instance: u64) {
        if let Ok(lib) = self.library(*handle) {
            let gone = lib.instances.borrow_mut().remove(&instance);
            drop(gone);
        }
    }

    /// The provider's `available`, under the limits, its map as `this`:
    /// `()` is available, text the reason it is not; an error it raises is
    /// unavailable in the error's words.
    fn available(&self, handle: &u64, provider: &str) -> Result<(), String> {
        let lib = self.library(*handle).map_err(|e| e.message().to_string())?;
        let state = lib
            .providers
            .get(provider)
            .cloned()
            .ok_or_else(|| format!("no provider `{provider}`"))?;
        let mut this = running(&state).map_err(|e| e.message().to_string())?;
        let answer = self.meter.run(|| {
            let ask = Self::method(&this, AVAILABLE)?;
            let answer = ask
                .call_as_method::<Dynamic>(&self.engine, &lib.ast, &mut this, ())
                .map_err(|e| provider_error(&e, &self.meter.limits))?;
            Ok::<_, ProviderError>(if answer.is_unit() {
                Ok(())
            } else if answer.is_string() {
                Err(answer.into_string().unwrap_or_default())
            } else {
                Err(format!(
                    "`available` answers () or a reason, not a {}",
                    what(&answer)
                ))
            })
        });
        drop(this);
        answer.map_err(|e| e.message().to_string())?
    }

    /// The object's map goes; what it held goes with it now. Rhai drops a
    /// nested value by recursion, which the engine thread's stack holds
    /// (see `limits`).
    fn release(&self, handle: &u64, object: u64) {
        if let Ok(lib) = self.library(*handle) {
            let gone = lib.objects.borrow_mut().remove(&object);
            drop(gone);
        }
    }

    fn close(&self, handle: u64) {
        let gone = self.libraries.borrow_mut().remove(&handle);
        drop(gone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_read_from_the_parse_without_running_anything() {
        let engine = rhai::Engine::new();
        let version = |code: &str| declared_version(&engine.compile(code).expect("parses"));
        assert_eq!(
            version("const VERSION = \"1.2.0\"; fn f() {}").as_deref(),
            Some("1.2.0")
        );
        assert_eq!(
            version("// const VERSION = \"9\";\nconst VERSION = \"1.0\";").as_deref(),
            Some("1.0")
        );
        assert_eq!(version("const VERSION = compute();"), None);
        assert_eq!(version("const MY_VERSION = \"2\";"), None);
    }

    /// On a thread with the stack the source gives its engine, which the
    /// engine checks before each operation.
    #[test]
    fn an_instance_is_kept_until_destroyed_once() {
        let config = Config::from_map(&guatiao::Map::new()).expect("a config");
        let stack = RhaiEngine::stack_size(&config).expect("a stack size");
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || instances_kept_until_destroyed_once(&config))
            .expect("spawned")
            .join()
            .expect("passed");
    }

    fn instances_kept_until_destroyed_once(config: &Config) {
        let dir = std::env::temp_dir().join(format!("guatiao-rhai-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("made");
        std::fs::write(
            dir.join("inst.guatiao"),
            "type = \"rhai\"\nmodule = \"inst\"\nkinds = []\nversion = \"1.0.0\"\n",
        )
        .expect("written");
        std::fs::write(
            dir.join("inst.rhai"),
            "#{ providers: #{ p: #{ kinds: [], config: #{}, create: |c| #{ n: c.n } } } }",
        )
        .expect("written");
        let engine = RhaiEngine::start(config).expect("starts");
        let Some(guatiao_bridge::Found::Library(manifest)) =
            guatiao_bridge::manifest::read(&RhaiEngine::MANIFESTS, &dir.join("inst.guatiao"))
        else {
            panic!("a manifest")
        };
        let Opened::Described { library, .. } = engine.open(&manifest, "1.0.0").expect("opens")
        else {
            panic!("described")
        };
        let config = |n: i64| {
            let mut m = guatiao::Map::new();
            m.set("n", n);
            Value::from(m)
        };
        let first = engine
            .create(&library, "inst_p", &config(1))
            .expect("built");
        let second = engine
            .create(&library, "inst_p", &config(2))
            .expect("built");
        assert_ne!(first, second);
        let live = || {
            engine
                .library(library)
                .expect("open")
                .instances
                .borrow()
                .len()
        };
        assert_eq!(live(), 2);
        engine.destroy(&library, first);
        assert_eq!(live(), 1);
        engine.destroy(&library, first);
        assert_eq!(live(), 1, "a second destroy of one id is nothing");
        engine.destroy(&library, second);
        assert_eq!(live(), 0);
        engine.close(library);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_script_is_found_as_a_module_is() {
        let dir = std::env::temp_dir().join(format!("guatiao-rhai-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg/inner")).expect("made");
        std::fs::write(dir.join("pkg/inner/init.rhai"), "1").expect("written");
        std::fs::write(dir.join("top.rhai"), "1").expect("written");
        let paths = [dir.clone()];
        assert_eq!(script_file("top", &paths), Some(dir.join("top.rhai")));
        assert_eq!(
            script_file("pkg.inner", &paths),
            Some(dir.join("pkg").join("inner").join("init.rhai"))
        );
        assert_eq!(script_file("missing", &paths), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
