// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lua as a guatiao-bridge engine: a library is a module returning a
//! table, its manifest a `*.guatiao.toml` of type `lua`, its providers the tables
//! of methods under its `providers`, served through tables the bridge
//! builds from the kinds' descriptions.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::path::PathBuf;

use guatiao::library::ProviderError;
use guatiao::library::kind::ArgType;
use guatiao::{Status, Value};
use guatiao_bridge::{
    BridgeError, Call, CallArg, Config, Engine, Held, Manifest, ManifestFormat, Opened, Provided,
    Reply, Target, ThreadModel,
};
use mlua::prelude::*;

use crate::calls::{self, Handle, Outcome, ReadBack, Received};
use crate::limits::Meter;
use crate::{host, served};

/// One library opened: its environment, its providers by id, and the
/// objects its slots answered, by the id each was given.
struct Library {
    providers: BTreeMap<String, LuaTable>,
    objects: RefCell<BTreeMap<u64, LuaTable>>,
}

/// The engine: one Lua state, on the bridge's thread for the life of the
/// source. Not `Send`.
pub struct LuaEngine {
    lua: Lua,
    meter: Meter,
    helpers: LuaTable,
    libraries: RefCell<BTreeMap<u64, Rc<Library>>>,
    next: Cell<u64>,
}

fn engine_error(e: LuaError) -> BridgeError {
    BridgeError::engine(calls::provider_error(&e).message().to_string())
}

fn refused(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// The file `require` finds `module` in on `paths`: `a.b` is `a/b.lua`,
/// then `a/b/init.lua`.
fn module_file(module: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    let rel = module.replace('.', "/");
    paths.iter().find_map(|dir| {
        [
            dir.join(format!("{rel}.lua")),
            dir.join(&rel).join("init.lua"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    })
}

/// Where a manifest says its code is.
fn paths_of(manifest: &Manifest) -> Vec<PathBuf> {
    manifest.paths().iter().map(PathBuf::from).collect()
}

/// The first `_VERSION = "..."` outside a comment, as a literal string.
fn literal_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let code = line.find("--").map_or(line, |at| &line[..at]);
        let mut rest = code;
        while let Some(at) = rest.find("_VERSION") {
            let before = rest[..at].chars().last();
            let after = &rest[at + "_VERSION".len()..];
            rest = after;
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let Some(after) = after.trim_start().strip_prefix('=') else {
                continue;
            };
            if after.starts_with('=') {
                continue;
            }
            let after = after.trim_start();
            let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                continue;
            };
            let body = &after[1..];
            return body.find(quote).map(|end| body[..end].to_string());
        }
    }
    None
}

impl LuaEngine {
    fn library(&self, handle: u64) -> Result<Rc<Library>, ProviderError> {
        self.libraries
            .borrow()
            .get(&handle)
            .cloned()
            .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_GONE, "the library was closed"))
    }

    /// A `require` for one library: `guatiao` is the helpers; anything
    /// else is found on the library's paths, run in its environment, and
    /// kept in its own loaded table.
    fn require(&self, env: &LuaTable, paths: Vec<PathBuf>) -> LuaResult<LuaFunction> {
        let loaded = self.lua.create_table()?;
        let helpers = self.helpers.clone();
        let env = env.clone();
        self.lua
            .create_function(move |lua, name: String| -> LuaResult<LuaValue> {
                if name == "guatiao" {
                    return Ok(LuaValue::Table(helpers.clone()));
                }
                let known: LuaValue = loaded.raw_get(name.as_str())?;
                if !known.is_nil() {
                    return Ok(known);
                }
                let file = module_file(&name, &paths).ok_or_else(|| {
                    LuaError::runtime(format!("module '{name}' not found on the library's paths"))
                })?;
                let text = std::fs::read(&file)
                    .map_err(|e| LuaError::runtime(format!("{}: {e}", file.display())))?;
                let result: LuaValue = lua
                    .load(&text[..])
                    .set_name(format!("@{}", file.display()))
                    .set_mode(mlua::chunk::ChunkMode::Text)
                    .set_environment(env.clone())
                    .call((name.as_str(), file.display().to_string()))?;
                let result = if result.is_nil() {
                    LuaValue::Boolean(true)
                } else {
                    result
                };
                loaded.raw_set(name.as_str(), result.clone())?;
                Ok(result)
            })
    }

    /// Converts a call's arguments for the slot, runs the method, writes
    /// lent ones back and answers the reply.
    fn answer(
        &self,
        lib: &Library,
        table: &LuaTable,
        call: &mut Call,
    ) -> Result<Reply, ProviderError> {
        let lua = &self.lua;
        let bad = |e: LuaError| calls::provider_error(&e);
        let slot = call.slot_description().clone();
        let mut values = Vec::with_capacity(call.args.len());
        let mut received = Vec::with_capacity(call.args.len());
        for (arg, d) in call.args.iter_mut().zip(slot.args()) {
            let object_kind = match d.ty() {
                ArgType::Object { kind, .. } => kind.clone(),
                _ => String::new(),
            };
            let (v, r) = match arg {
                CallArg::Value(v) | CallArg::Lent(v) => {
                    calls::received(lua, d.ty(), v).map_err(bad)?
                }
                CallArg::Buffer(b) => calls::buffer(lua, core::mem::take(b)).map_err(bad)?,
                CallArg::Object(_) => {
                    let CallArg::Object(o) = core::mem::replace(arg, CallArg::Value(Value::null()))
                    else {
                        unreachable!("matched just above")
                    };
                    (
                        calls::object_handle(lua, &object_kind, o).map_err(bad)?,
                        Received::Plain,
                    )
                }
                CallArg::LentObject(h) => {
                    let caller = calls::caller(lua, &object_kind).map_err(bad)?;
                    let ud = lua
                        .create_userdata(Handle {
                            held: Held::Lent(h.clone()),
                            caller,
                        })
                        .map_err(bad)?;
                    (LuaValue::UserData(ud), Received::Plain)
                }
                _ => {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "an argument this engine does not carry",
                    ));
                }
            };
            values.push(v);
            received.push(r);
        }
        let results = self.meter.run(lua, || {
            served::protected_method(lua, table, slot.name(), values, self.meter.memory())
        })?;
        for ((arg, d), r) in call.args.iter_mut().zip(slot.args()).zip(&received) {
            match calls::read_back(lua, d.ty(), r).map_err(bad)? {
                Some(ReadBack::Tree(v)) => *arg = CallArg::Lent(v),
                Some(ReadBack::Bytes(b)) => *arg = CallArg::Buffer(b),
                None => {}
            }
        }
        Ok(match calls::outcome(lua, &slot, results)? {
            Outcome::Nothing => Reply::Nothing,
            Outcome::Value(v) => Reply::Value(v),
            Outcome::Handed(o) => Reply::Handed(o),
            Outcome::Table(kind, t) => {
                let id = self.next.get() + 1;
                self.next.set(id);
                lib.objects.borrow_mut().insert(id, t);
                Reply::Object { kind, id }
            }
        })
    }

    /// Lets go of what nothing reaches any more: finalizers run here,
    /// under the limits.
    fn collect(&self) {
        let _ = self.meter.run(&self.lua, || self.lua.gc_collect());
    }
}

impl Engine for LuaEngine {
    type Library = u64;

    /// One Lua state, which is not `Send`: on the bridge's thread.
    const THREADS: ThreadModel = ThreadModel::Pinned;

    const MANIFESTS: ManifestFormat = ManifestFormat {
        serves: "lua",
        id_key: "module",
        code_extensions: &["lua"],
        skip_dirs: &[],
    };

    /// A Lua state with the safe standard libraries, the kinds the
    /// configuration names, and its limits.
    fn start(config: &Config) -> Result<LuaEngine, BridgeError> {
        let unavailable = |e: LuaError| BridgeError::Unavailable(format!("Lua did not start: {e}"));
        let lua = Lua::new();
        served::keep_pcall(&lua).map_err(unavailable)?;
        {
            let mut known = calls::known(&lua).map_err(unavailable)?;
            for path in config.kinds() {
                known.kinds.load(path)?;
            }
        }
        let helpers = host::helpers(&lua).map_err(unavailable)?;
        #[cfg(feature = "luau")]
        lua.sandbox(true).map_err(unavailable)?;
        let meter = Meter::install(&lua, *config.limits()).map_err(unavailable)?;
        Ok(LuaEngine {
            lua,
            meter,
            helpers,
            libraries: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
        })
    }

    /// The module's `_VERSION`, read as a literal from its file, which is
    /// not run.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        let file = module_file(manifest.id(), &paths_of(manifest)).ok_or_else(|| {
            BridgeError::engine(format!("{} is not on the manifest's paths", manifest.id()))
        })?;
        let text = std::fs::read_to_string(&file)
            .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
        literal_version(&text).ok_or_else(|| {
            BridgeError::engine(format!(
                "{} states no `_VERSION = \"...\"`: give the manifest a `version`",
                file.display()
            ))
        })
    }

    /// Runs the module afresh in an environment of its own and reads its
    /// `providers`.
    fn open(&self, manifest: &Manifest, version: &str) -> Result<Opened<u64>, BridgeError> {
        let lua = &self.lua;
        let id = manifest.id();
        let env = lua.create_table().map_err(engine_error)?;
        let inherit = lua.create_table().map_err(engine_error)?;
        inherit
            .raw_set("__index", lua.globals())
            .map_err(engine_error)?;
        env.set_metatable(Some(inherit)).map_err(engine_error)?;
        let require = self
            .require(&env, paths_of(manifest))
            .map_err(engine_error)?;
        env.raw_set("require", require.clone())
            .map_err(engine_error)?;

        let mut all = LuaMultiValue::new();
        all.push_back(LuaValue::Function(require));
        all.push_back(LuaValue::String(
            lua.create_string(id).map_err(engine_error)?,
        ));
        let mut results = self
            .meter
            .run(lua, || served::protected(lua, all, self.meter.memory()))
            .map_err(|e| BridgeError::engine(format!("{id}: {}", e.message())))?;
        let LuaValue::Table(module) = results.pop_front().unwrap_or(LuaValue::Nil) else {
            return Err(BridgeError::engine(format!(
                "{id} returns no table: a library is a module returning one"
            )));
        };
        if let Ok(LuaValue::String(stated)) = module.raw_get::<LuaValue>("_VERSION")
            && stated.to_string_lossy() != version
        {
            return Err(BridgeError::engine(format!(
                "{id} states `_VERSION = \"{}\"`, and its manifest {version}",
                stated.to_string_lossy()
            )));
        }
        let listed: LuaValue = module.raw_get("providers").map_err(engine_error)?;
        let LuaValue::Table(listed) = listed else {
            return Err(BridgeError::engine(format!(
                "{id} has no `providers` table: name each provider in it"
            )));
        };

        // Reading a provider can run its metatable's code: under the
        // limits too.
        let (offered, providers) = self.meter.run(lua, || self.declare(manifest, listed))?;

        let handle = self.next.get() + 1;
        self.next.set(handle);
        self.libraries.borrow_mut().insert(
            handle,
            Rc::new(Library {
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
        let table = match call.target() {
            Target::Provider(id) => lib.providers.get(id).cloned(),
            Target::Object(id) => lib.objects.borrow().get(id).cloned(),
            _ => None,
        }
        .ok_or_else(|| refused(format!("nothing answers `{}`", call.method())))?;
        self.answer(&lib, &table, call)
    }

    /// The object's table goes; what it held goes with it now.
    fn release(&self, handle: &u64, object: u64) {
        if let Ok(lib) = self.library(*handle) {
            let gone = lib.objects.borrow_mut().remove(&object);
            drop(gone);
            self.collect();
        }
    }

    fn close(&self, handle: u64) {
        let gone = self.libraries.borrow_mut().remove(&handle);
        drop(gone);
        self.collect();
    }
}

impl LuaEngine {
    /// The providers a module's `providers` table names, each serving the
    /// kinds it lists with the slots it has functions for.
    #[allow(clippy::type_complexity)]
    fn declare(
        &self,
        manifest: &Manifest,
        listed: LuaTable,
    ) -> Result<(Vec<Provided>, BTreeMap<String, LuaTable>), BridgeError> {
        let id = manifest.id();
        let mut keyed: Vec<(String, LuaTable)> = Vec::new();
        for pair in listed.pairs::<String, LuaTable>() {
            let (key, provider) = pair.map_err(|e| {
                BridgeError::engine(format!(
                    "{id}: `providers` holds something that is not a provider table named by text: {}",
                    calls::provider_error(&e).message()
                ))
            })?;
            keyed.push((key, provider));
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0));

        let mut offered = Vec::new();
        let mut providers = BTreeMap::new();
        for (key, provider) in keyed {
            let pid = format!("{}_{key}", id.replace('.', "_"));
            let kinds: Vec<String> =
                match provider.get::<LuaValue>("kinds").map_err(engine_error)? {
                    LuaValue::Table(t) => t
                        .sequence_values::<String>()
                        .collect::<LuaResult<_>>()
                        .map_err(engine_error)?,
                    _ => {
                        return Err(BridgeError::engine(format!(
                            "{id}: the provider `{key}` names no `kinds`"
                        )));
                    }
                };
            let name = match provider.get::<LuaValue>("name").map_err(engine_error)? {
                LuaValue::String(s) => s.to_string_lossy(),
                _ => key.clone(),
            };
            let mut declared = Provided::new(&pid).name(&name);
            for kind in &kinds {
                if !manifest.kinds().iter().any(|k| k == kind) {
                    return Err(BridgeError::engine(format!(
                        "{id}: the provider `{key}` serves `{kind}`, which the manifest does not list"
                    )));
                }
                let described = calls::known(&self.lua)
                    .map_err(engine_error)?
                    .kinds
                    .get(kind)
                    .cloned()
                    .ok_or_else(|| BridgeError::UnknownKind(kind.clone()))?;
                let mut implements = Vec::new();
                for slot in described.slots() {
                    if matches!(
                        provider
                            .get::<LuaValue>(slot.name())
                            .map_err(engine_error)?,
                        LuaValue::Function(_)
                    ) {
                        implements.push(slot.name().to_string());
                    }
                }
                declared = declared.serves(kind, implements);
            }
            offered.push(declared);
            providers.insert(pid, provider);
        }
        Ok((offered, providers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_read_as_a_literal() {
        assert_eq!(
            literal_version("local M = { _VERSION = \"1.2.0\" }").as_deref(),
            Some("1.2.0")
        );
        assert_eq!(
            literal_version("-- _VERSION = '0.9'\nM._VERSION = '1.0'").as_deref(),
            Some("1.0")
        );
        assert_eq!(literal_version("if M._VERSION == '1' then end"), None);
        assert_eq!(literal_version("MY_VERSION = '2'"), None);
        assert_eq!(literal_version("_VERSION = compute()"), None);
    }

    #[test]
    fn a_module_is_found_as_require_finds_one() {
        let dir = std::env::temp_dir().join(format!("guatiao-lua-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg/inner")).expect("made");
        std::fs::write(dir.join("pkg/inner/init.lua"), "return {}").expect("written");
        std::fs::write(dir.join("top.lua"), "return {}").expect("written");
        let paths = [dir.clone()];
        assert_eq!(module_file("top", &paths), Some(dir.join("top.lua")));
        assert_eq!(
            module_file("pkg.inner", &paths),
            Some(dir.join("pkg").join("inner").join("init.lua"))
        );
        assert_eq!(module_file("missing", &paths), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
