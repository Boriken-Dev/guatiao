// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Rust snippets of the landing page, the ownership guide, the migration
//! page, the configuration page and the root README, compiled and checked to
//! still be the text those pages show.
//!
//! Each test holds its own copy of a snippet between marker comments and
//! compares it, line for line, with the matching fenced `rust` block of the
//! page. A snippet that stops compiling fails the build; one that drifts from
//! its page fails the comparison. This crate holds them rather than `guatiao`
//! because the pages sit outside any published crate.

use std::path::Path;

const LANDING: &str = include_str!("../../../docs/index.md");
const README: &str = include_str!("../../../README.md");
const OWNERSHIP: &str = include_str!("../../../docs/ownership.md");
const MIGRATING: &str = include_str!("../../../docs/migrating.md");
const FORMS: &str = include_str!("../../../docs/forms.md");
const CONFIGURATION: &str = include_str!("../../../docs/configuration.md");
const DERIVED_GREETER: &str = include_str!("../../derived_greeter/src/lib.rs");

#[test]
fn the_landing_page_builds_and_reads_a_value() -> Result<(), Box<dyn core::error::Error>> {
    // TOUR-VALUE-BEGIN
    use guatiao::Map;

    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);

    let host: &str = map.required("host")?.try_into()?;
    let port: u16 = map.required("port")?.try_into()?;
    // TOUR-VALUE-END
    assert_eq!((host, port), ("10.0.0.1", 5900));

    assert_eq!(rust_block(LANDING, 0), marked("TOUR-VALUE"));
    Ok(())
}

#[test]
fn the_landing_page_scans_a_directory_for_a_provider() {
    fn tour() -> Result<(), Box<dyn core::error::Error>> {
        // TOUR-REGISTRY-BEGIN
        use guatiao::library::{Registry, scan_dir};
        use std::path::Path;

        let mut registry = Registry::new("my-host", "1.0");
        scan_dir(&mut registry, Path::new("/opt/myapp/plugins"))?;
        let greeter = registry.best("greeter").expect("something serves it");
        // TOUR-REGISTRY-END
        let _ = greeter;
        Ok(())
    }

    // The directory does not exist, so the scan is refused before `best`.
    assert!(tour().is_err());
    assert!(!Path::new("/opt/myapp/plugins").exists());

    assert_eq!(rust_block(LANDING, 1), marked("TOUR-REGISTRY"));
}

#[test]
fn the_readme_quick_start_compiles_and_holds() -> Result<(), Box<dyn core::error::Error>> {
    // README-QUICK-START-BEGIN
    use guatiao::Map;

    let mut options = Map::new();
    options.set("compression", 6);

    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);
    map.set("options", options);

    let host: &str = map.required("host")?.try_into()?;
    let port: u16 = map.required("port")?.try_into()?;
    assert_eq!((host, port), ("10.0.0.1", 5900));
    // README-QUICK-START-END

    assert_eq!(rust_block(README, 0), marked("README-QUICK-START"));
    Ok(())
}

#[test]
fn the_ownership_guide_passes_trees_by_value() -> Result<(), Box<dyn core::error::Error>> {
    // OWN-KIND-BEGIN
    use guatiao::library::ProviderError;
    use guatiao::{Alloc, Map, Text};

    #[guatiao::kind]
    pub trait Store: Send + Sync {
        /// Reads the configuration and keeps none of it: borrowed.
        fn check(&self, config: &Map) -> Result<bool, ProviderError>;
        /// Keeps the configuration: handed over, a move.
        fn save(&self, config: Map) -> Result<(), ProviderError>;
        /// An answer the caller owns, returned by value.
        fn load(&self) -> Result<Map, ProviderError>;
    }
    // OWN-KIND-END

    // OWN-PROVIDER-BEGIN
    use std::sync::Mutex;

    struct Files {
        saved: Mutex<Option<Map>>,
    }

    impl Store for Files {
        fn check(&self, config: &Map) -> Result<bool, ProviderError> {
            Ok(config.contains_key("path"))
        }

        fn save(&self, config: Map) -> Result<(), ProviderError> {
            // The caller's tree, now this provider's: kept as it arrived.
            *self.saved.lock().expect("not poisoned") = Some(config);
            Ok(())
        }

        fn load(&self) -> Result<Map, ProviderError> {
            self.load_in(Alloc::rust())
        }

        // What a call goes through: the answer is built in the allocator
        // the caller carried in, so it crosses back without a copy.
        fn load_in(&self, alloc: Alloc) -> Result<Map, ProviderError> {
            let mut map = Map::new_in(alloc);
            map.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;
            Ok(map)
        }
    }
    // OWN-PROVIDER-END

    // OWN-HOST-BEGIN
    fn configure(store: &dyn Store) -> Result<Map, ProviderError> {
        let mut config = Map::new();
        config.set("path", "/etc/app.toml");
        // Lent to `check` for the call, then handed to `save`: nothing is
        // copied either way.
        if store.check(&config)? {
            store.save(config)?;
        }
        // Built in this side's allocator, by the provider.
        store.load()
    }
    // OWN-HOST-END

    let files = Files {
        saved: Mutex::new(None),
    };
    let loaded = configure(&files)?;
    assert!(loaded.is_local());
    assert!(files.saved.lock().expect("not poisoned").is_some());

    assert_eq!(rust_block(OWNERSHIP, 0), marked("OWN-KIND"));
    assert_eq!(rust_block(OWNERSHIP, 1), marked("OWN-PROVIDER"));
    assert_eq!(rust_block(OWNERSHIP, 2), marked("OWN-HOST"));
    Ok(())
}

#[test]
fn the_ownership_guide_builds_where_the_tree_will_live() -> Result<(), Box<dyn core::error::Error>>
{
    // OWN-ALLOC-BEGIN
    use guatiao::{Alloc, Map, Text, ValueError};

    /// What a library builds outside any call, in the allocator its host
    /// offered: in a library, `host.alloc().unwrap_or_else(Alloc::rust)`.
    fn describe(alloc: Alloc) -> Result<Map, ValueError> {
        let mut meta = Map::new_in(alloc);
        meta.set_in("vendor", Text::new_in(alloc, "acme")?, alloc)?;
        Ok(meta)
    }

    let meta = describe(Alloc::rust())?;
    // A tree in hand goes where it is wanted, copied only if it is not
    // already there.
    let kept = meta.into_alloc(Alloc::rust())?;
    // OWN-ALLOC-END
    assert!(kept.is_local());

    assert_eq!(rust_block(OWNERSHIP, 3), marked("OWN-ALLOC"));
    Ok(())
}

#[test]
fn the_ownership_guide_moves_instead_of_copying() -> Result<(), Box<dyn core::error::Error>> {
    // OWN-MOVES-BEGIN
    use guatiao::{List, Map, Value};

    // Build in place: a child goes into its parent by moving.
    let mut child = Map::new();
    child.set("depth", 2);
    let mut parent = Map::new();
    parent.set("child", child);

    // A container becomes a node by moving, and comes back out the same way.
    let node = Value::from(parent);
    let mut parent = Map::try_from(node).expect("a map");

    // Read by borrowing, not by converting into an owned copy.
    let child: &Map = parent.required("child")?.try_into()?;
    let depth: i64 = child.required("depth")?.try_into()?;

    // Take a value out instead of cloning it: `remove` hands it back owned,
    // and `mem::take` leaves a null behind.
    let taken: Value = parent.remove("child").expect("present");
    let mut list: List = ["a", "b"].into_iter().collect();
    let first = core::mem::take(&mut list[0]);

    // Own a tree you were handed: `into_local` copies only what is elsewhere.
    let owned = taken.into_local();
    // OWN-MOVES-END
    assert_eq!(depth, 2);
    assert_eq!(first, Value::from("a"));
    assert_eq!(list[0], Value::null());
    assert!(owned.is_local());

    assert_eq!(rust_block(OWNERSHIP, 4), marked("OWN-MOVES"));
    Ok(())
}

#[test]
fn the_ownership_guide_lends_a_tree_to_be_changed() -> Result<(), Box<dyn core::error::Error>> {
    // OWN-LEND-BEGIN
    use guatiao::library::ProviderError;
    use guatiao::{Map, Text, TryAsRef};

    #[guatiao::kind]
    pub trait Settings: Send + Sync {
        /// Completes the configuration where it stands: lent, and changed.
        fn check(&self, config: &mut Map) -> Result<bool, ProviderError>;
    }

    struct Files;

    impl Settings for Files {
        fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
            // Built in the allocator the caller's map records, so nothing has
            // to be moved into it once the call returns.
            let alloc = config.alloc()?;
            if !config.contains_key("path") {
                config.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;
            }
            let Some(path) = config.get("path").and_then(TryAsRef::<str>::try_as_ref) else {
                return Ok(false);
            };
            let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
            let dir = Text::new_in(alloc, dir)?;
            config.set_in("dir", dir, alloc)?;
            Ok(true)
        }
    }
    // OWN-LEND-END

    // OWN-LEND-HOST-BEGIN
    fn prepare(settings: &dyn Settings) -> Result<Map, ProviderError> {
        let mut config = Map::new();
        config.set("mode", "strict");
        // Lent for the call: filled in where it stands, and still this
        // side's map afterwards.
        if !settings.check(&mut config)? {
            return Err(ProviderError::new(
                guatiao::Status::GUATIAO_ERR_BAD_VALUE,
                "no path",
            ));
        }
        Ok(config)
    }
    // OWN-LEND-HOST-END

    let config = prepare(&Files)?;
    assert_eq!(config.keys().collect::<Vec<_>>(), ["mode", "path", "dir"]);
    let dir: &str = config.required("dir")?.try_into()?;
    assert_eq!(dir, "/etc");
    assert!(config.is_local());

    assert_eq!(rust_block(OWNERSHIP, 5), marked("OWN-LEND"));
    assert_eq!(rust_block(OWNERSHIP, 6), marked("OWN-LEND-HOST"));
    Ok(())
}

// --- the migration page --------------------------------------------------
//
// One test per section of `docs/migrating.md`, its blocks in page order.

#[test]
fn migrating_values() -> Result<(), Box<dyn core::error::Error>> {
    let alloc = guatiao::Alloc::rust();

    // MIG-TO-VALUE-BEGIN
    use guatiao::{Alloc, Schema, ToValue, Value, ValueError};

    struct Port(u16);

    // A hand-written impl names the allocator in `to_value_in`.
    impl ToValue for Port {
        fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
            self.0.to_value_in(alloc)
        }
    }

    // Building in this binary's allocator names none.
    let port = Port(5900).to_value()?;
    let schema = Vec::<String>::schema()?;

    // Building in another one says so with `_in`.
    let port_there = Port(5900).to_value_in(alloc)?;
    let schema_there = Vec::<String>::schema_in(alloc)?;
    // MIG-TO-VALUE-END
    assert_eq!(port, port_there);
    assert_eq!(schema, schema_there);

    // MIG-SET-BEGIN
    use guatiao::{List, Map, Text};

    // The short forms grow in the container's own allocator and abort if it
    // refuses, as `Vec::push` does: nothing to `?` or `unwrap`.
    let mut map = Map::new();
    map.set("port", 5900);
    let mut list = List::new();
    list.push("a");
    let mut text = Text::new("a");
    text.push_str("b");

    // Where the allocator may refuse and you handle it, say `try_`.
    let mut settings = Map::new_in(alloc);
    settings.try_set("port", 5900)?;
    list.try_push("b")?;
    text.try_push_str("c")?;
    // MIG-SET-END
    assert_eq!(settings.len(), 1);

    // MIG-ITER-MUT-BEGIN
    for (_key, value) in &mut map {
        *value = Value::null();
    }
    // MIG-ITER-MUT-END
    assert_eq!(map.get("port"), Some(&Value::null()));

    let value = Value::from(map);
    // MIG-CLONE-BEGIN
    // A copy in this binary's allocator, whatever the source recorded.
    let copy = value.clone();
    // A copy beside its source, as `clone()` made before.
    let beside = value.clone_in(value.alloc()?)?;
    // MIG-CLONE-END
    assert_eq!(copy, beside);

    // MIG-STATUS-BEGIN
    use guatiao::{Status, Tag};

    fn describe(status: Status) -> &'static str {
        match status {
            Status::GUATIAO_OK => "done",
            Status::GUATIAO_ERR_BUSY => "try again later",
            // Any code is a `Status`, a host's own included.
            _ => "refused",
        }
    }

    let status = Status::from(14u32);
    let code: u32 = u32::from(status);

    let is_map = match value.tag()? {
        Tag::GUATIAO_MAP => true,
        // Tags are appended, so a `match` keeps a `_` arm.
        _ => false,
    };
    // MIG-STATUS-END
    assert_eq!(
        (describe(status), code, is_map),
        ("try again later", 14, true)
    );

    for (n, name) in [
        "MIG-TO-VALUE",
        "MIG-SET",
        "MIG-ITER-MUT",
        "MIG-CLONE",
        "MIG-STATUS",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(rust_block(MIGRATING, n), marked(name), "{name}");
    }
    Ok(())
}

#[test]
fn migrating_schema_and_derives() -> Result<(), Box<dyn core::error::Error>> {
    let mut schema = guatiao::Map::new();
    schema.set("type", "object");
    let mut properties = guatiao::Map::new();
    let mut port = guatiao::Map::new();
    port.set("type", "integer");
    properties.set("port", port);
    let mut verbose = guatiao::Map::new();
    verbose.set("type", "boolean");
    properties.set("verbose", verbose);
    schema.set("properties", properties);
    let schema = guatiao::Value::from(schema);
    let schema = guatiao::schema::SchemaRef::new(&schema).expect("a schema");

    // MIG-VALIDATE-BEGIN
    use guatiao::Map;
    use guatiao::schema::{validate_map, validate_text};

    // Each field holds its own type: a number for an integer, a boolean for
    // a boolean, never their text.
    let mut config = Map::new();
    config.set("port", 5900);
    config.set("verbose", true);
    validate_map(schema, &config.into())?;

    // Text a person typed is checked as text, one field at a time.
    let port = schema.find("port").expect("declared");
    validate_text(port, "5900")?;
    // MIG-VALIDATE-END

    // MIG-NULLABLE-BEGIN
    use guatiao::Schema;
    use guatiao::schema::{KindRef, SchemaRef};

    #[derive(Schema)]
    struct Settings {
        /// `type: ["string", "null"]` now, as `FromValue` reads it.
        proxy: Option<String>,
    }

    let document = Settings::schema()?;
    let settings = SchemaRef::new(&document).expect("a schema");
    for field in settings.fields() {
        // Read through the field, never the `type` key: it is either shape.
        let optional = field.is_nullable();
        let is_text = matches!(field.kind(), KindRef::Str { .. });
        assert!(optional && is_text);
    }
    // MIG-NULLABLE-END
    assert!(Settings { proxy: None }.proxy.is_none());

    let (system, user) = (guatiao::Value::from(guatiao::Map::new()), config_value());
    // MIG-MERGE-BEGIN
    use guatiao::MergeMode;
    use guatiao::schema::merge::{merge_options, merge_overrides};

    // Reading the `x-merge` annotations can fail: past `MAX_DEPTH`.
    let overrides = merge_overrides(settings)?;
    let options = merge_options(settings)?;

    // A path is its segments: `tls.ca` as a key is `["tls.ca"]`.
    let overrides = overrides.with(["tls", "ciphers"], MergeMode::Deep);
    let (merged, provenance) = MergeMode::Substitute.merge_layers_with(
        [("system", &system), ("user", &user)],
        guatiao::Alloc::rust(),
        options,
        Some(&overrides),
    )?;
    let from = provenance.source_of(["tls", "ca"]);
    for (path, layer) in provenance.leaves() {
        println!("{} from {layer}", path.join("."));
    }
    // MIG-MERGE-END
    let _ = (merged, from);

    // MIG-RAW-IDENT-BEGIN
    use guatiao::{FromValue, ToValue};

    #[derive(ToValue, FromValue)]
    struct Record {
        // Keyed `type` now, as serde keys it. To keep reading a stored
        // `r#type`, name that key:
        #[map(rename = "r#type")]
        r#type: String,
    }
    // MIG-RAW-IDENT-END
    let record = Record { r#type: "a".into() }.to_value()?;
    assert!(
        guatiao::TryAsRef::<Map>::try_as_ref(&record)
            .expect("a map")
            .contains_key("r#type")
    );
    assert_eq!(Record::from_value(&record)?.r#type, "a");

    for (n, name) in ["MIG-VALIDATE", "MIG-NULLABLE", "MIG-MERGE", "MIG-RAW-IDENT"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(rust_block(MIGRATING, 5 + n), marked(name), "{name}");
    }
    Ok(())
}

/// A layer for the merge snippet: one leaf under `tls`.
fn config_value() -> guatiao::Value {
    let mut tls = guatiao::Map::new();
    tls.set("ca", "/etc/ca.pem");
    let mut map = guatiao::Map::new();
    map.set("tls", tls);
    map.into()
}

/// A kind and two providers registered in this process, for the kinds and
/// loader sections of the migration page.
mod acme {
    use guatiao::library::{ProviderError, Registry};
    use guatiao::{FromValue, Map, Provider, Schema};

    #[guatiao::kind]
    pub trait Greeter: Send + Sync {
        /// A greeting for `name`.
        fn greet(&self, name: &str) -> Result<Map, ProviderError>;
    }

    #[guatiao::kind(object)]
    pub trait Note: Send {
        /// How many characters the note holds.
        fn size(&self) -> i64;
    }

    /// One instance, built without a configuration.
    #[derive(Default, Provider)]
    #[provider(Greeter, id = "acme_hello")]
    pub struct Hello;

    impl Greeter for Hello {
        fn greet(&self, name: &str) -> Result<Map, ProviderError> {
            let mut map = Map::new();
            map.set("greeting", format!("hello, {name}"));
            Ok(map)
        }
    }

    /// Built only from a configuration.
    #[derive(Schema, FromValue, Provider)]
    #[provider(Greeter, config, id = "acme_shouter")]
    pub struct Shouter {
        /// What every shout starts with.
        pub prefix: String,
    }

    impl Greeter for Shouter {
        fn greet(&self, name: &str) -> Result<Map, ProviderError> {
            let mut map = Map::new();
            map.set(
                "greeting",
                format!("{} {}", self.prefix, name.to_uppercase()),
            );
            Ok(map)
        }
    }

    guatiao::local_providers!(id = "acme", version = "1.0.0", providers = [Hello, Shouter]);

    pub fn registry() -> Registry {
        let mut registry = Registry::new("my-host", "1.0");
        registry
            .register_local("acme", library)
            .expect("the linked library registers")
            .loaded()
            .expect("it accepts this host");
        registry
    }
}

#[test]
fn migrating_kinds_and_providers() -> Result<(), Box<dyn core::error::Error>> {
    use acme::{Greeter, Note};

    let registry = acme::registry();
    let mut config = guatiao::Map::new();
    config.set("prefix", "hey");
    let config = guatiao::Value::from(config);

    // MIG-OFFER-BEGIN
    use guatiao::library::Remote;

    let offer = registry
        .offer::<dyn Greeter>("acme_hello")
        .expect("filed under that key")?;
    // A provider built only from a configuration has no default instance.
    if let Some(greeter) = offer.default_instance() {
        greeter.greet("ana")?;
    }
    // `remote()`, `boxed()` and `shared()` answer an `Option` for the same
    // reason.
    let remote: Option<Remote<dyn Greeter>> = offer.remote();
    // MIG-OFFER-END
    assert!(remote.is_some());

    // MIG-INSTANTIATE-BEGIN
    use guatiao::library::InstantiateError;

    let shouter = registry
        .offer::<dyn Greeter>("acme_shouter")
        .expect("filed under that key")?;
    match shouter.instantiate(&config) {
        Ok(instance) => println!("{:?}", instance.greet("ana")?),
        // The configuration does not fit the provider's schema: the
        // provider was never asked.
        Err(InstantiateError::Invalid(why)) => println!("check the settings: {why}"),
        // The provider's own refusal.
        Err(InstantiateError::Refused(why)) => println!("refused: {why}"),
        Err(other) => return Err(other.into()),
    }
    // MIG-INSTANTIATE-END

    let instance = shouter.instantiate(&config)?;
    // MIG-REMOTE-BEGIN
    // `Remote` is `Clone`, not `Copy`: each holds its library open.
    let remote = offer.remote().expect("a default instance");
    let kept = remote.clone();
    // `Instance::remote()` lends the instance's own; clone it to keep it.
    let owned: Remote<dyn Greeter> = instance.remote().clone();
    // MIG-REMOTE-END
    let _ = (kept, owned);

    // MIG-AVAILABLE-BEGIN
    // The reason is borrowed from the offer, which holds the library open.
    let why_not: Option<String> = offer.available().err().map(str::to_string);
    // MIG-AVAILABLE-END
    assert_eq!(why_not, None);

    // MIG-PARTS-BEGIN
    use guatiao::library::kind::ProviderParts;

    let parts = ProviderParts::builder("acme_hello")
        .name("Hello")
        .version("1.0.0")
        .kinds(&["greeter"])
        // then `.table(..)` per table, `.ctx(..)`, and one call per slot
        // you fill (`.available`, `.create`, `.destroy`, `.config`, `.meta`)
        .build();
    // MIG-PARTS-END
    let _ = parts;

    // MIG-OBJECT-BEGIN
    use guatiao::library::Object;
    use guatiao::library::kind::ObjectCell;

    struct Sticky(String);

    impl Note for Sticky {
        fn size(&self) -> i64 {
            self.0.chars().count() as i64
        }
    }

    // What the kind attribute builds for you:
    let note: Object<dyn Note> = Sticky("hi".into()).into_object();
    // or the cell spelled out, its table chosen by the value's type:
    let cell = Box::new(ObjectCell::new(Sticky("hi".into())));
    let same: Object<dyn Note> = Object::from_cell(cell)?;
    // MIG-OBJECT-END
    assert_eq!(note.size(), same.size());

    for (n, name) in [
        "MIG-OFFER",
        "MIG-INSTANTIATE",
        "MIG-REMOTE",
        "MIG-AVAILABLE",
        "MIG-PARTS",
        "MIG-OBJECT",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(rust_block(MIGRATING, 9 + n), marked(name), "{name}");
    }
    Ok(())
}

/// The example library cargo built beside this test binary.
fn derived_greeter() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("in a directory");
    let file = format!(
        "{}derived_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [deps.to_path_buf(), deps.join("..")]
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
        .expect("the dev-dependency's cdylib is beside the test binary")
}

#[test]
fn migrating_the_loader_and_handles() -> Result<(), Box<dyn core::error::Error>> {
    use greeter_kind::Greeter;
    use guatiao::library::Registry;

    let path = derived_greeter();
    let mut registry = Registry::new("my-host", "1.0");
    let loaded = registry.load_file(&path)?.loaded().expect("loaded");
    let library_key = loaded.key.to_string();
    let offer = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .expect("filed")?;

    // MIG-VIEW-BEGIN
    let view = offer.view();
    let id: &str = view.id();
    let version: Option<&str> = view.version();
    let has_config = view.config().is_some();
    // MIG-VIEW-END
    // No version of its own: the provider's is its library's.
    assert_eq!(
        (id, version, has_config),
        ("derived_greeter_hello", None, false)
    );

    // MIG-HOST-BEGIN
    // The `Host` a library is handed; a registry gives its own.
    let host = registry.host();
    // A view, read under the size its descriptor declares, holding the
    // provider's library open while it lives.
    if let Ok(Some(view)) = host.get("derived_greeter_hello") {
        println!("{} {}", view.id(), view.display_name());
    }
    // MIG-HOST-END

    // MIG-UNLOAD-BEGIN
    use guatiao::library::UnloadError;

    // Drop what you took from the library first: every `Remote`, `Offer`,
    // `Instance`, `Object` and `ProviderView` holds it open.
    drop(offer);
    match registry.unload(&library_key) {
        Ok(()) => println!("unloaded"),
        Err(UnloadError::Busy { leases, .. }) => println!("{leases} handles still hold it"),
        Err(other) => return Err(other.into()),
    }
    // MIG-UNLOAD-END
    assert!(registry.provider("derived_greeter_hello").is_none());

    let dir = path.parent().expect("in a directory");
    // MIG-LOADED-BEGIN
    use guatiao::library::scan_dir;

    let report = scan_dir(&mut registry, dir)?;
    for (path, key) in &report.loaded {
        println!("{} filed as {key}", path.display());
    }
    // MIG-LOADED-END

    // MIG-MALFORMED-BEGIN
    use guatiao::library::LoadError;

    match registry.load_file(&path) {
        Err(LoadError::Malformed { path, reason, .. }) => {
            println!("{} refused: {reason}", path.display());
        }
        Err(other) => println!("{other}"),
        Ok(loading) => println!("{loading:?}"),
    }
    // MIG-MALFORMED-END

    // MIG-LOCAL-BEGIN
    use guatiao::library::Host;
    use guatiao::library::kind::LibraryParts;

    // `local_providers!` writes `library(Host) -> Option<LibraryParts>`,
    // built on each call; a describe of your own may answer one by value.
    fn describe(host: Host) -> Option<LibraryParts> {
        acme::library(host)
    }
    registry.register_local("acme", describe)?;
    // MIG-LOCAL-END
    assert!(registry.provider("acme_hello").is_some());

    for (n, name) in [
        "MIG-UNLOAD",
        "MIG-VIEW",
        "MIG-HOST",
        "MIG-LOCAL",
        "MIG-MALFORMED",
        "MIG-LOADED",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(rust_block(MIGRATING, 15 + n), marked(name), "{name}");
    }
    Ok(())
}

#[test]
fn migrating_serde_and_intake() -> Result<(), Box<dyn core::error::Error>> {
    let text = "{\"port\": }";

    // MIG-SYNTAX-BEGIN
    use guatiao_serde::text::{Error, json};

    match json::from_str(text, guatiao::Alloc::rust(), Default::default()) {
        Ok(value) => println!("{value:?}"),
        // Where it stopped, and the parser's words; never the text itself.
        Err(error @ Error::Syntax { .. }) => {
            if let Some(at) = error.position() {
                println!("line {}, column {}: {error}", at.line, at.column);
            }
        }
        Err(other) => return Err(other.into()),
    }
    // MIG-SYNTAX-END
    assert!(matches!(
        json::from_str(text, guatiao::Alloc::rust(), Default::default()),
        Err(Error::Syntax {
            position: Some(_),
            ..
        })
    ));

    // MIG-SPLIT-BEGIN
    use guatiao_intake::flat::split;

    // The member is a `Cow`: borrowed unless the key had to be unquoted.
    if let Some((owner, member)) = split("tls[\"a.b\"]") {
        assert_eq!((owner, &*member), ("tls", "a.b"));
    }
    // MIG-SPLIT-END

    for (n, name) in ["MIG-SYNTAX", "MIG-SPLIT"].into_iter().enumerate() {
        assert_eq!(rust_block(MIGRATING, 21 + n), marked(name), "{name}");
    }
    Ok(())
}

// --- a provider's form ----------------------------------------------------

/// The provider the page shows is the example library's own, as compiled.
#[test]
fn the_forms_page_shows_the_example_provider() {
    assert_eq!(
        rust_block(FORMS, 0),
        marked_in(DERIVED_GREETER, "FORM-PROVIDER")
    );
}

#[test]
fn the_forms_page_reads_a_provider_s_form() -> Result<(), Box<dyn core::error::Error>> {
    use greeter_kind::Greeter;
    use guatiao::library::Registry;

    let mut registry = Registry::new("form-host", "1.0");
    registry.load_file(&derived_greeter())?;

    // FORM-HOST-BEGIN
    use guatiao::schema::SchemaRef;
    use guatiao::{Map, Value};
    use guatiao_intake::{FormRef, PROVIDER_FORM_KEY, check, is_visible, layout};

    let offer = registry
        .offer::<dyn Greeter>("derived_greeter_shouter")
        .ok_or("nothing is filed under that key")??;
    let schema = offer.config_schema().and_then(SchemaRef::new);
    let form = offer
        .meta()
        .and_then(|meta| meta.get(PROVIDER_FORM_KEY))
        .and_then(FormRef::new);

    if let (Some(schema), Some(form)) = (schema, form) {
        // A form is data a library wrote: check it before showing it.
        check(schema, form)?;
        for group in layout(schema, form) {
            let title = group.section.map_or("", |s| s.label());
            let keys: Vec<&str> = group.fields.iter().map(|p| p.field.key()).collect();
            println!("{title}: {keys:?}");
        }
        let mut values = Map::new();
        values.set("exclaim", false);
        let shown = is_visible(schema, form, "marks", &Value::from(values))?;
        assert!(!shown, "`marks` waits for `exclaim`");
    }
    // FORM-HOST-END

    assert_eq!(rust_block(FORMS, 1), marked("FORM-HOST"));
    Ok(())
}

/// The configuration page's type, at module level as a program declares
/// it, with the `std` paths a program writes.
#[allow(clippy::std_instead_of_core, clippy::std_instead_of_alloc)]
mod configuration_page {
    // CONFIG-TYPE-BEGIN
    use std::collections::BTreeMap;
    use std::net::{IpAddr, SocketAddr};
    use std::path::PathBuf;

    use guatiao::{FromValue, Schema, ToValue};
    use guatiao_intake::Form;

    /// The certificate the daemon presents.
    #[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
    #[map(untagged, rename_all = "lowercase")]
    pub enum Tls {
        /// The machine's own identity.
        Identity,
        /// A certificate and its key, from files.
        Files { certificate: PathBuf, key: PathBuf },
    }

    #[derive(ToValue, FromValue, Schema, Form, Debug, PartialEq)]
    #[form(section(id = "net", label = "Network"))]
    pub struct Daemon {
        /// Where to listen.
        #[schema(section = "net")]
        #[form(placeholder = "0.0.0.0:7000")]
        pub listen: SocketAddr,
        /// The address to advertise, when it is not the one listened on.
        #[schema(section = "net")]
        pub advertise: Option<IpAddr>,
        pub tls: Tls,
        /// Where state is kept.
        #[map(default = PathBuf::from("/var/lib/daemon"))]
        pub state: PathBuf,
        /// Each plugin's own settings, which the plugin describes.
        #[map(default, with = guatiao_serde::adapt::object)]
        pub plugins: BTreeMap<String, toml::Table>,
    }
    // CONFIG-TYPE-END
}

#[test]
fn the_configuration_page_reads_and_describes_a_daemon() -> Result<(), Box<dyn core::error::Error>>
{
    use configuration_page::{Daemon, Tls};

    let text = String::from(
        "listen = \"0.0.0.0:7000\"\n\
         tls = { certificate = \"/etc/ssl/d.pem\", key = \"/etc/ssl/d.key\" }\n\
         [plugins.scaler]\n\
         factor = 1.5\n",
    );

    // CONFIG-READ-BEGIN
    use guatiao::schema::SchemaRef;
    use guatiao::schema::validate::validate_map;
    use guatiao::{Alloc, FromValue, Schema};
    use guatiao_serde::Presentation;
    use guatiao_serde::text::toml;

    let value = toml::from_str(&text, Alloc::rust(), Presentation::new())?;

    // The schema refuses what the type does not declare, a misspelled key
    // included, naming the key; reading alone would skip it.
    let schema = Daemon::schema()?;
    validate_map(SchemaRef::new(&schema).ok_or("not a schema")?, &value)?;

    let daemon = Daemon::from_value(&value)?;
    // CONFIG-READ-END
    assert_eq!(daemon.listen.port(), 7000);
    assert!(matches!(daemon.tls, Tls::Files { .. }));
    assert_eq!(daemon.state, std::path::PathBuf::from("/var/lib/daemon"));
    assert_eq!(daemon.plugins["scaler"]["factor"].as_float(), Some(1.5));

    // CONFIG-DESCRIBE-BEGIN
    use guatiao_intake::{FormRef, Screen, check};
    use guatiao_serde::text::json;

    let schema = Daemon::schema()?;
    let form = Daemon::form(Alloc::rust())?;
    check(
        SchemaRef::new(&schema).ok_or("not a schema")?,
        FormRef::new(&form).ok_or("not a form")?,
    )?;

    // The schema is a JSON Schema: an editor that reads one completes and
    // checks the file as it is written.
    let document = json::to_string_pretty(&schema, Presentation::new())?;
    // CONFIG-DESCRIBE-END
    assert!(document.contains("\"anyOf\""));

    assert_eq!(rust_block(CONFIGURATION, 0), marked("CONFIG-TYPE"));
    assert_eq!(rust_block(CONFIGURATION, 1), marked("CONFIG-READ"));
    assert_eq!(rust_block(CONFIGURATION, 2), marked("CONFIG-DESCRIBE"));
    Ok(())
}

#[test]
#[allow(clippy::std_instead_of_core)]
fn the_configuration_page_s_with_module_converts_a_field() -> Result<(), Box<dyn core::error::Error>>
{
    use guatiao::{FromValue, Schema, ToValue};

    // CONFIG-WITH-BEGIN
    mod millis {
        use std::time::Duration;

        use guatiao::schema::KindBuilder;
        use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError};

        pub fn to_value_in(timeout: &Duration, alloc: Alloc) -> Result<Value, ValueError> {
            let ms = u64::try_from(timeout.as_millis()).map_err(|_| ValueError::OutOfRange)?;
            ms.to_value_in(alloc)
        }

        pub fn from_value(value: &Value) -> Result<Duration, MapError> {
            u64::from_value(value).map(Duration::from_millis)
        }

        pub fn kind(alloc: Alloc) -> KindBuilder {
            u64::kind(alloc).description("milliseconds")
        }
    }

    #[derive(ToValue, FromValue, Schema)]
    struct Retry {
        #[map(with = millis)]
        timeout: std::time::Duration,
    }
    // CONFIG-WITH-END

    let retry = Retry {
        timeout: std::time::Duration::from_millis(1500),
    };
    let value = retry.to_value()?;
    let map: &guatiao::Map = (&value).try_into()?;
    assert_eq!(u64::from_value(map.required("timeout")?)?, 1500);
    assert_eq!(Retry::from_value(&value)?.timeout, retry.timeout);
    let schema = Retry::schema()?;
    let s = guatiao::schema::SchemaRef::new(&schema).ok_or("not a schema")?;
    assert_eq!(
        s.find("timeout").ok_or("no field")?.description(),
        "milliseconds"
    );

    assert_eq!(rust_block(CONFIGURATION, 3), marked("CONFIG-WITH"));
    Ok(())
}

/// The `index`-th fenced `rust` block of `md`, minus doctest-hidden lines
/// (`#` alone or `# ` first).
fn rust_block(md: &str, index: usize) -> String {
    let mut found = 0;
    let mut inside = false;
    let mut out: Vec<&str> = Vec::new();

    for line in md.lines() {
        if !inside {
            if line.trim_end() == "```rust" {
                if found == index {
                    inside = true;
                }
                found += 1;
            }
            continue;
        }
        if line.trim_end() == "```" {
            return out.join("\n");
        }
        if line == "#" || line.starts_with("# ") {
            continue;
        }
        out.push(line);
    }
    panic!("no closed rust block number {index}");
}

/// The lines of this file between `// <name>-BEGIN` and `// <name>-END`,
/// dedented by the four spaces they carry for being inside a function; a
/// snippet nested one function deeper carries eight.
fn marked(name: &str) -> String {
    marked_in(include_str!("docs_snippets.rs"), name)
}

/// [`marked`], in another source file.
fn marked_in(source: &str, name: &str) -> String {
    let begin = format!("// {name}-BEGIN");
    let end = format!("// {name}-END");
    let mut inside = false;
    let mut lines: Vec<&str> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_start();
        if !inside {
            inside = trimmed == begin;
            continue;
        }
        if trimmed == end {
            let indent = lines
                .iter()
                .filter(|l| !l.is_empty())
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(0);
            let body: Vec<&str> = lines
                .iter()
                .map(|l| if l.len() >= indent { &l[indent..] } else { *l })
                .collect();
            return body.join("\n");
        }
        lines.push(line);
    }
    panic!("the markers for {name} are missing");
}
