// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a language runtime implements to become a source: the [`Engine`]
//! trait, its thread model, and what crosses a call.

use alloc::sync::Arc;

use guatiao::Value;
use guatiao::library::LocalLibrary;
use guatiao::library::ProviderError;
use guatiao::library::kind::{KindDescription, LentHandle, RawObject, SlotDescription};

use crate::config::Config;
use crate::error::BridgeError;
use crate::manifest::{Manifest, ManifestFormat};

/// Where an engine may be called from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThreadModel {
    /// From any thread, at once: the engine is thread-safe.
    Any,
    /// From any thread, one call at a time: the engine holds a lock of its
    /// own (CPython's GIL), which a call waits on.
    Locked,
    /// From one thread the bridge starts, which owns the engine: a call
    /// from another thread is queued to it and waited for; a call made on
    /// it (the engine calling a provider of its own through the host) runs
    /// there and then. Lua, QuickJS, Rhai.
    Pinned,
}

/// A language runtime serving libraries written in it.
///
/// The bridge reads manifests, keeps stamps, refreshes, and answers the
/// registry; the engine starts, opens the library a manifest describes,
/// answers calls into what it opened, and closes it. Every method is
/// called where [`THREADS`](Engine::THREADS) says, and takes `&self`: an
/// engine called while one of its own calls is on the stack (through the
/// host) is called again.
pub trait Engine: 'static {
    /// The engine's handle on a library it opened: a key into its own
    /// state, which crosses to and from its thread.
    type Library: Clone + Send + Sync + 'static;

    /// Where it may be called from.
    const THREADS: ThreadModel;

    /// Which manifests are its own (their `type`) and what its code looks like.
    const MANIFESTS: ManifestFormat;

    /// Starts the engine for a source's configuration, on the thread it
    /// will be called from. `Unavailable` when it cannot run here.
    fn start(config: &Config) -> Result<Self, BridgeError>
    where
        Self: Sized;

    /// The stack, in bytes, of the thread a [`Pinned`](ThreadModel::Pinned)
    /// engine is started on for `config`; `None` (the default) leaves the
    /// platform's. The size is reserved, and memory is used only as the
    /// stack grows into it; a size the system will not reserve refuses the
    /// source as `Unavailable`. Not asked of other thread models.
    fn stack_size(config: &Config) -> Option<usize>
    where
        Self: Sized,
    {
        let _ = config;
        None
    }

    /// The version a manifest leaves out, read from the library's code
    /// without running it. Refused by default.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        Err(BridgeError::Manifest {
            path: manifest.path().to_path_buf(),
            why: format!(
                "{}: `version` is missing, and this engine reads none from the code",
                manifest.path().display()
            ),
        })
    }

    /// Opens the library `manifest` describes, at `version`: runs its code
    /// and answers what it offers. The library's id must be the manifest's.
    fn open(
        &self,
        manifest: &Manifest,
        version: &str,
    ) -> Result<Opened<Self::Library>, BridgeError>;

    /// A call into a library [`open`](Engine::open) answered as
    /// [`Opened::Described`]: a slot of one of its providers, or of an
    /// object it answered. Lent arguments it changes are written back
    /// after the call. Refused by default.
    fn call(&self, library: &Self::Library, call: &mut Call) -> Result<Reply, ProviderError> {
        let _ = (library, call);
        Err(ProviderError::new(
            guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
            "this engine answers no calls",
        ))
    }

    /// An object the engine answered ([`Reply::Object`]) was destroyed by
    /// whoever held it last.
    fn release(&self, library: &Self::Library, object: u64) {
        let _ = (library, object);
    }

    /// An instance of the provider declared as `provider`, built from
    /// `config`, for a provider declared with
    /// [`Provided::config`]: the engine's id for it, which its slots'
    /// calls name as [`Target::Instance`]. A host's `instantiate` (Rust or
    /// C) checks `config` against the declared schema first when it can
    /// read the schema; read it as input all the same. Refused by default.
    fn create(
        &self,
        library: &Self::Library,
        provider: &str,
        config: &Value,
    ) -> Result<u64, ProviderError> {
        let _ = (library, provider, config);
        Err(ProviderError::new(
            guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
            "this engine builds no instances",
        ))
    }

    /// The instance [`create`](Engine::create) answered as `instance` was
    /// dropped by whoever held it last; called once. An instance never
    /// dropped is never destroyed, and its lease keeps the library open.
    fn destroy(&self, library: &Self::Library, instance: u64) {
        let _ = (library, instance);
    }

    /// Whether the provider declared as `provider` can run now, or why
    /// not, for a provider declared with
    /// [`Provided::may_be_unavailable`]. Asked each time a host asks,
    /// never cached; every distinct reason is kept until the library
    /// closes (past 64, a fixed sentence stands for a new one), so answer
    /// with stable words. Available by default.
    fn available(&self, library: &Self::Library, provider: &str) -> Result<(), String> {
        let _ = (library, provider);
        Ok(())
    }

    /// Lets a library go: the registry closed it, nothing holds it.
    fn close(&self, library: Self::Library);
}

/// What opening a library came to.
#[non_exhaustive]
pub enum Opened<L> {
    /// The engine built the library's descriptor itself, with tables of
    /// its own (CPython builds them with ctypes). It keeps everything the
    /// descriptor points into until [`Engine::close`].
    Built {
        /// The engine's handle on it.
        library: L,
        /// The descriptor.
        descriptor: LocalLibrary<'static>,
    },
    /// Providers whose kinds the bridge builds tables for from their
    /// descriptions, every slot a [`Call`] to [`Engine::call`]. Needs a
    /// mechanism ([`BridgeSource::with_mechanism`](crate::BridgeSource::with_mechanism)).
    Described {
        /// The engine's handle on it.
        library: L,
        /// Its providers.
        providers: Vec<Provided>,
    },
}

impl<L> core::fmt::Debug for Opened<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Opened::Built { .. } => f.write_str("Opened::Built"),
            Opened::Described { providers, .. } => f
                .debug_struct("Opened::Described")
                .field("providers", providers)
                .finish(),
        }
    }
}

/// One provider an engine declares, served through tables the bridge
/// builds.
#[derive(Debug, Clone)]
pub struct Provided {
    id: String,
    name: String,
    serves: Vec<(String, Vec<String>)>,
    meta: Option<guatiao::Map>,
    config: Option<Value>,
    instances_only: bool,
    may_be_unavailable: bool,
}

impl Provided {
    /// A provider with this id, unique across every library a host loads.
    pub fn new(id: &str) -> Provided {
        Provided {
            id: id.to_string(),
            name: String::new(),
            serves: Vec::new(),
            meta: None,
            config: None,
            instances_only: false,
            may_be_unavailable: false,
        }
    }

    /// Built from a configuration fitting `schema` (a JSON Schema as a
    /// value): a host's `Offer::instantiate` reaches [`Engine::create`],
    /// and dropping what it built reaches [`Engine::destroy`]. The
    /// provider stays callable as itself too, unless
    /// [`instances_only`](Provided::instances_only).
    pub fn config(mut self, schema: Value) -> Provided {
        self.config = Some(schema);
        self
    }

    /// Callable only through instances [`config`](Provided::config)
    /// builds: no default instance is offered.
    pub fn instances_only(mut self) -> Provided {
        self.instances_only = true;
        self
    }

    /// Asked whether it can run before a host uses it: a host's
    /// `available` reaches [`Engine::available`]. Without this the
    /// provider is always available.
    pub fn may_be_unavailable(mut self) -> Provided {
        self.may_be_unavailable = true;
        self
    }

    /// A name to show a person.
    pub fn name(mut self, name: &str) -> Provided {
        self.name = name.to_string();
        self
    }

    /// Serves `kind`, implementing the slots named: a slot it leaves out
    /// is null in the table, so a caller runs the kind's default; a
    /// required one left out refuses the library.
    pub fn serves<S: AsRef<str>>(
        mut self,
        kind: &str,
        implements: impl IntoIterator<Item = S>,
    ) -> Provided {
        self.serves.push((
            kind.to_string(),
            implements
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
        ));
        self
    }

    /// What else it declares, as a map a host reads (`notices`, a form).
    pub fn meta(mut self, meta: guatiao::Map) -> Provided {
        self.meta = Some(meta);
        self
    }

    /// Its id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its display name.
    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// Every kind it serves, with the slots it implements.
    pub fn kinds(&self) -> &[(String, Vec<String>)] {
        &self.serves
    }

    /// What else it declares.
    pub fn declared_meta(&self) -> Option<&guatiao::Map> {
        self.meta.as_ref()
    }

    /// The schema of the configuration it is built from, when it is.
    pub fn config_schema(&self) -> Option<&Value> {
        self.config.as_ref()
    }

    /// Whether only instances of it are offered.
    pub fn is_instances_only(&self) -> bool {
        self.instances_only
    }

    /// Whether a host asks the engine if it can run.
    pub fn asks_available(&self) -> bool {
        self.may_be_unavailable
    }
}

/// Who a call is for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    /// A provider, by the id it was declared under.
    Provider(String),
    /// An object the engine answered, by the id it gave.
    Object(u64),
    /// An instance [`Engine::create`] built, by the id it gave.
    Instance(u64),
}

/// One argument, as the engine receives it.
///
/// Scalars, text, bytes and trees arrive as [`Value`]s: a number as its
/// value (exact, as the slot's width allows), text as a string, bytes as
/// bytes, a borrowed tree copied, a taken one moved.
#[derive(Debug)]
#[non_exhaustive]
pub enum CallArg {
    /// A value the engine reads, or keeps if it was handed over.
    Value(Value),
    /// The caller's tree, lent: what the engine leaves here is written
    /// back over the caller's after the call.
    Lent(Value),
    /// The caller's buffer to write into: written back after the call,
    /// as much as fits.
    Buffer(Vec<u8>),
    /// An object handed over: the engine owns it, and destroys it by
    /// dropping it.
    Object(RawObject),
    /// An object lent for this call: callable until the call returns.
    LentObject(LentHandle),
}

/// One call into an engine's library.
#[derive(Debug)]
pub struct Call {
    target: Target,
    kind: Arc<KindDescription>,
    slot: usize,
    /// The arguments, in the slot's order.
    pub args: Vec<CallArg>,
}

impl Call {
    /// A call of slot `slot` (an index into `kind`'s slots) on `target`.
    pub fn new(
        target: Target,
        kind: Arc<KindDescription>,
        slot: usize,
        args: Vec<CallArg>,
    ) -> Call {
        Call {
            target,
            kind,
            slot,
            args,
        }
    }

    /// Who it is for.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The kind called.
    pub fn kind(&self) -> &Arc<KindDescription> {
        &self.kind
    }

    /// The slot's index in the kind.
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// The slot's description: its name, arguments and answer.
    pub fn slot_description(&self) -> &SlotDescription {
        &self.kind.slots()[self.slot]
    }

    /// The slot's name.
    pub fn method(&self) -> &str {
        self.slot_description().name()
    }
}

/// What an engine answers a call with.
#[derive(Debug)]
#[non_exhaustive]
pub enum Reply {
    /// For a slot that answers nothing.
    Nothing,
    /// A value: converted to the slot's answer (a number narrowed to its
    /// width, text, a tree), or refused when it does not fit.
    Value(Value),
    /// An object the engine holds, of the object kind named, by an id of
    /// its own: its slots arrive as calls on [`Target::Object`], and
    /// [`Engine::release`] hears when it is destroyed.
    Object {
        /// The object kind.
        kind: String,
        /// The engine's id for it.
        id: u64,
    },
    /// An object handed back as it came.
    Handed(RawObject),
}
