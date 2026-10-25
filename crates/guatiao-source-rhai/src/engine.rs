// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai as a guatiao-bridge engine: a library is a script whose last
//! expression is a map of providers, its manifest a
//! `*.guatiao-rhai.toml`, each provider a map whose function members are
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

/// One library opened: its script, its providers by id, and the objects its
/// slots answered, by the id each was given. Every provider shares the one
/// compiled script.
struct Library {
    ast: AST,
    providers: BTreeMap<String, State>,
    objects: RefCell<BTreeMap<u64, State>>,
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
            .and_then(|m| m.get(slot).and_then(|f| f.clone().try_cast::<FnPtr>()))
            .ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    &format!("no method `{slot}`"),
                )
            })
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
            if self.engine.ensure_data_size_within_limits(&v).is_err() {
                return Err(BridgeError::Limit {
                    limit: guatiao_bridge::Limit::Memory,
                    at: self.meter.limits.memory.unwrap_or(0),
                }
                .into());
            }
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
    /// ones back and answers the reply.
    fn answer(
        &self,
        lib: &Library,
        state: &State,
        call: &mut Call,
    ) -> Result<Reply, ProviderError> {
        let slot = call.slot_description().clone();
        let (values, back) = self.arguments(call, &slot)?;
        let mut this = state.try_borrow_mut().map_err(|_| {
            ProviderError::new(
                Status::GUATIAO_ERR_BUSY,
                "the provider is already running: a call re-entered it",
            )
        })?;
        // Everything the script made is read and dropped within the call,
        // the method's curried values included: Rhai copies and drops nested
        // values by recursion, in the stack the call keeps free for them.
        let outcome = self.meter.run(|| {
            let method = Self::method(&this, slot.name())?;
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
        suffix: ".guatiao-rhai.toml",
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
            }),
        );
        Ok(Opened::Described {
            library: handle,
            providers: offered,
        })
    }

    fn call(&self, handle: &u64, call: &mut Call) -> Result<Reply, ProviderError> {
        let lib = self.library(*handle)?;
        let state = match call.target() {
            Target::Provider(id) => lib.providers.get(id).cloned(),
            Target::Object(id) => lib.objects.borrow().get(id).cloned(),
            _ => None,
        }
        .ok_or_else(|| refused(format!("nothing answers `{}`", call.method())))?;
        self.answer(&lib, &state, call)
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
