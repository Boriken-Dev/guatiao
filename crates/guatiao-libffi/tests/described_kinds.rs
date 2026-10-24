// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Kind tables built at run time from `greeter_kind`'s descriptions,
//! served by one Rust handler and called as the Rust traits through a
//! registry -- the same proxies a compiled library's tables are called
//! through -- and a compiled provider called from its description.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use std::sync::{Mutex, OnceLock};

use greeter_kind::{
    Conversation, ConversationVtable, Counter, CounterVtable, Greeter, GreeterVtable, Listener,
    ListenerVtable, Wide, WideVtable,
};
use guatiao::library::kind::{
    Answer, Arg, Caller, DescribedRemote, KindDescription, LibraryParts, ProviderParts, RawObject,
    Serve, ServedTable,
};
use guatiao::library::{Object, ProviderError, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};
use guatiao_libffi::Libffi;

/// A kind as `#[guatiao::kind]` describes it.
fn described(json: &str) -> Arc<KindDescription> {
    let value = guatiao_serde::text::json::from_str(
        json,
        guatiao::Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .expect("JSON");
    Arc::new(KindDescription::from_value(&value).expect("a description"))
}

struct Kinds {
    greeter: Arc<KindDescription>,
    listener: Arc<KindDescription>,
    conversation: Arc<KindDescription>,
    counter: Arc<KindDescription>,
    wide: Arc<KindDescription>,
}

fn kinds() -> Kinds {
    Kinds {
        greeter: described(&GreeterVtable::description()),
        listener: described(&ListenerVtable::description()),
        conversation: described(&ConversationVtable::description()),
        counter: described(&CounterVtable::description()),
        wide: described(&WideVtable::description()),
    }
}

/// One conversation the engine holds: the listener it was handed, and
/// what was said.
struct Talk {
    listener: RawObject,
    said: Vec<String>,
}

/// An engine of one: every slot of every kind lands here.
struct Engine {
    listener: Caller,
    conversations: Mutex<BTreeMap<usize, Talk>>,
    next: Mutex<usize>,
    conversation_table: OnceLock<Weak<ServedTable>>,
    counted: Mutex<i64>,
}

fn arg_str<'a>(args: &'a [Arg<'_>], i: usize) -> &'a str {
    match &args[i] {
        Arg::Str(s) => s,
        other => panic!("{other:?}"),
    }
}

impl Engine {
    fn greeting(name: &str) -> Result<Map, ProviderError> {
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        let mut map = Map::new();
        map.set("greeting", format!("hello, {name}").as_str());
        Ok(map)
    }
}

impl Serve for Engine {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        match (kind.name(), kind.slots()[slot].name()) {
            ("greeter", "greet") => Ok(Answer::Map(Engine::greeting(arg_str(&args, 0))?)),
            ("greeter", "check") => {
                let Some(Arg::LentMap(config)) = args.pop() else {
                    panic!("a lent map");
                };
                if config.get("name").is_none() {
                    config.set("name", "ana");
                }
                Ok(Answer::Bool(true))
            }
            ("greeter", "greet_from") => {
                let Some(Arg::TakenMap(config)) = args.pop() else {
                    panic!("a taken map");
                };
                let name = config
                    .get("name")
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("")
                    .to_string();
                Ok(Answer::Map(Engine::greeting(&name)?))
            }
            ("greeter", "start") => {
                let Some(Arg::Object(listener)) = args.pop() else {
                    panic!("a taken object");
                };
                let id = {
                    let mut next = self.next.lock().unwrap();
                    *next += 1;
                    *next
                };
                self.conversations.lock().unwrap().insert(
                    id,
                    Talk {
                        listener,
                        said: Vec::new(),
                    },
                );
                let table = self
                    .conversation_table
                    .get()
                    .and_then(Weak::upgrade)
                    .expect("the provider keeps it");
                // The provider keeps the table with its parts.
                Ok(Answer::Object(table.object(id).expect("an object kind")))
            }
            ("greeter", "greet_into") => {
                let greeting = Engine::greeting(arg_str(&args, 0))?;
                let Some(Arg::LentObject(listener)) = args.pop() else {
                    panic!("a lent object");
                };
                let text = greeting
                    .get("greeting")
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("")
                    .to_string();
                listener.call(&self.listener, 0, vec![Arg::Str(&text)])?;
                Ok(Answer::Nothing)
            }
            ("conversation", "say") => {
                let what = arg_str(&args, 0).to_string();
                let mut talks = self.conversations.lock().unwrap();
                let talk = talks.get_mut(&ctx).expect("a live conversation");
                talk.listener
                    .call(&self.listener, 0, vec![Arg::Str(&what)])?;
                talk.said.push(what);
                Ok(Answer::Nothing)
            }
            ("conversation", "turns") => {
                let talks = self.conversations.lock().unwrap();
                Ok(Answer::Int(talks[&ctx].said.len() as i64))
            }
            ("conversation", "read") => {
                let Some(Arg::Buffer(dst)) = args.pop() else {
                    panic!("a buffer");
                };
                let talks = self.conversations.lock().unwrap();
                let text = talks[&ctx].said.join(" ");
                let n = text.len().min(dst.len());
                dst[..n].copy_from_slice(&text.as_bytes()[..n]);
                Ok(Answer::Int(n as i64))
            }
            ("counter", "count") => {
                let mut n = self.counted.lock().unwrap();
                *n += 1;
                Ok(Answer::Int(*n))
            }
            ("wide", "echo_i64") | ("wide", "echo_u64") => match args.pop() {
                Some(Arg::Int(x)) => Ok(Answer::Int(x)),
                Some(Arg::Uint(x)) => Ok(Answer::Uint(x)),
                other => panic!("{other:?}"),
            },
            (k, s) => panic!("{k}.{s}"),
        }
    }

    fn destroy(&self, ctx: usize) {
        // The listener goes with it: destroyed through its own table.
        self.conversations.lock().unwrap().remove(&ctx);
    }
}

/// A registry serving the engine's provider, every table built at run
/// time and kept with the provider.
fn serving(shout: bool) -> (Registry, Arc<Engine>) {
    let k = kinds();
    let engine = Arc::new(Engine {
        listener: Caller::new(Arc::clone(&k.listener), &Libffi).expect("interfaces"),
        conversations: Mutex::new(BTreeMap::new()),
        next: Mutex::new(0),
        conversation_table: OnceLock::new(),
        counted: Mutex::new(0),
    });
    let serve: Arc<dyn Serve> = engine.clone();
    let greeter = ServedTable::new(
        k.greeter,
        |s| shout || s.name() != "shout",
        Arc::clone(&serve),
        &Libffi,
    )
    .expect("built");
    let conversation = Arc::new(
        ServedTable::new(k.conversation, |_| true, Arc::clone(&serve), &Libffi).expect("built"),
    );
    assert!(
        engine
            .conversation_table
            .set(Arc::downgrade(&conversation))
            .is_ok(),
        "set once"
    );
    let counter =
        ServedTable::new(k.counter, |_| true, Arc::clone(&serve), &Libffi).expect("built");
    let wide = ServedTable::new(k.wide, |_| true, Arc::clone(&serve), &Libffi).expect("built");
    let provider = ProviderParts::builder("engine_greeter")
        .kinds(&["greeter", "counter", "wide"])
        .table(greeter.kind_table())
        .table(counter.kind_table())
        .table(wide.kind_table())
        .ctx(core::ptr::without_provenance_mut(1))
        .keep(greeter)
        .keep(counter)
        .keep(wide)
        .keep(conversation)
        .build();
    let mut registry = Registry::new("described-kinds", "1.0");
    registry
        .register_local("engine", |_| {
            Some(LibraryParts::new("engine", "1.0.0", vec![provider]))
        })
        .expect("registered");
    (registry, engine)
}

/// A listener the host implements: what it heard, shared with the test.
struct Heard(Arc<Mutex<Vec<String>>>);

impl Listener for Heard {
    fn heard(&mut self, what: &str) {
        self.0.lock().unwrap().push(what.to_string());
    }
}

#[test]
fn a_greeter_built_at_run_time_is_called_as_the_trait() {
    let (registry, _engine) = serving(false);
    let offer = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("offered as the trait");
    let greeter = offer.default_instance().expect("a default instance");

    let answer = greeter.greet("ana").expect("greeted");
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ana")
    );
    let refused = greeter.greet("").unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(refused.message(), "nobody to greet");

    // A slot left out runs the trait's default body, through `greet`.
    assert_eq!(greeter.shout("bo"), "HELLO, BO");

    // A lent map, completed in place and settled home by the proxy.
    let mut config = Map::new();
    assert!(greeter.check(&mut config).expect("checked"));
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );

    // A map handed over.
    let mut given = Map::new();
    given.set("name", "cy");
    let answer = greeter.greet_from(given).expect("greeted");
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, cy")
    );

    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .expect("offered")
        .shared()
        .expect("an instance");
    assert_eq!(counter.count(), 1);
    assert_eq!(counter.count(), 2);

    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .expect("offered")
        .shared()
        .expect("an instance");
    assert_eq!(wide.echo_i64(i64::MIN).expect("echoed"), i64::MIN);
    assert_eq!(wide.echo_u64(u64::MAX).expect("echoed"), u64::MAX);
}

#[test]
fn objects_cross_a_table_built_at_run_time_both_ways() {
    let (registry, engine) = serving(true);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("offered")
        .shared()
        .expect("an instance");
    let heard = Arc::new(Mutex::new(Vec::new()));

    // Lent for one call: the engine calls the host's listener through it.
    let mut lent = Heard(Arc::clone(&heard));
    greeter.greet_into("di", &mut lent).expect("told");
    assert_eq!(*heard.lock().unwrap(), ["hello, di"]);

    // Handed over: the engine keeps the listener, and answers a
    // conversation its own table serves.
    let listener = Heard(Arc::clone(&heard)).into_object();
    let mut talk: Object<dyn Conversation> = greeter.start(listener).expect("a conversation");
    talk.say("one").expect("said");
    talk.say("two").expect("said");
    assert_eq!(talk.turns(), 2);
    let mut buf = [0u8; 16];
    let n = talk.read(&mut buf) as usize;
    assert_eq!(&buf[..n], b"one two");
    assert_eq!(*heard.lock().unwrap(), ["hello, di", "one", "two"]);
    assert_eq!(engine.conversations.lock().unwrap().len(), 1);
    drop(talk);
    assert!(
        engine.conversations.lock().unwrap().is_empty(),
        "destroying the conversation reached the engine"
    );
}

/// A listener served from its description, counting what it heard and
/// how often it was destroyed.
#[derive(Default)]
struct Ears {
    heard: Mutex<Vec<String>>,
    destroyed: Mutex<usize>,
}

impl Serve for Ears {
    fn call(
        &self,
        _: usize,
        _: &KindDescription,
        _: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        self.heard
            .lock()
            .unwrap()
            .push(arg_str(&args, 0).to_string());
        Ok(Answer::Nothing)
    }

    fn destroy(&self, _: usize) {
        *self.destroyed.lock().unwrap() += 1;
    }
}

#[test]
fn an_owned_object_is_lent_and_stays_its_owners() {
    let (registry, _engine) = serving(true);
    let provider = registry.provider("engine_greeter").expect("registered");
    let caller = Arc::new(Caller::new(kinds().greeter, &Libffi).expect("interfaces"));
    let remote = DescribedRemote::new(provider.view(), caller).expect("its table fits");
    let ears = Arc::new(Ears::default());
    let listeners =
        ServedTable::new(kinds().listener, |_| true, ears.clone(), &Libffi).expect("built");
    let owned = listeners.object(1).expect("an object kind");
    remote
        .call_named(
            "greet_into",
            vec![Arg::Str("ed"), Arg::LentObject(owned.lend())],
        )
        .expect("told");
    assert_eq!(*ears.heard.lock().unwrap(), ["hello, ed"]);
    assert_eq!(
        *ears.destroyed.lock().unwrap(),
        0,
        "a lend destroys nothing"
    );
    drop(owned);
    assert_eq!(*ears.destroyed.lock().unwrap(), 1, "its owner destroys it");
}

#[test]
fn a_compiled_provider_is_called_from_its_description() {
    #[derive(Default)]
    struct Hello;
    impl Greeter for Hello {
        fn greet(&self, name: &str) -> Result<Map, ProviderError> {
            Engine::greeting(name)
        }
    }
    let parts = ProviderParts::builder("compiled_hello")
        .kinds(&["greeter"])
        .table(guatiao::library::KindTable::new(
            "greeter",
            (&HELLO_TABLE as *const GreeterVtable).cast(),
            size_of::<GreeterVtable>(),
        ))
        .ctx((&HELLO as *const Hello).cast_mut().cast())
        .build();
    static HELLO: Hello = Hello;
    static HELLO_TABLE: GreeterVtable = GreeterVtable::of::<Hello>();
    let mut registry = Registry::new("described-kinds", "1.0");
    registry
        .register_local("compiled", |_| {
            Some(LibraryParts::new("compiled", "1.0.0", vec![parts]))
        })
        .expect("registered");
    let provider = registry.provider("compiled_hello").expect("registered");
    let caller = Arc::new(Caller::new(kinds().greeter, &Libffi).expect("interfaces"));
    let remote = DescribedRemote::new(provider.view(), caller).expect("its table fits");
    let Answer::Map(map) = remote
        .call_named("greet", vec![Arg::Str("ed")])
        .expect("greeted")
    else {
        panic!("a map");
    };
    assert_eq!(
        map.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ed")
    );
    let refused = remote.call_named("greet", vec![Arg::Str("")]).unwrap_err();
    assert_eq!(refused.message(), "nobody to greet");
    let mut config = Map::new();
    assert!(matches!(
        remote.call_named("check", vec![Arg::LentMap(&mut config)]),
        Ok(Answer::Bool(false))
    ));
}

/// The C compilers worth trying, in order (as `c_library.rs`).
fn find_compiler() -> Option<std::path::PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = std::path::PathBuf::from(name);
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if std::process::Command::new(&path)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Some(path);
        }
    }
    None
}

type DriveGreet = unsafe extern "C" fn(
    *const core::ffi::c_void,
    *mut core::ffi::c_void,
    *const core::ffi::c_char,
    *mut Map,
    *mut ProviderError,
) -> Status;
type DriveCheck = unsafe extern "C" fn(
    *const core::ffi::c_void,
    *mut core::ffi::c_void,
    *mut Map,
    *mut bool,
    *mut ProviderError,
) -> Status;
type DriveTable = unsafe extern "C" fn(*const core::ffi::c_void) -> i32;
type DriveWide = unsafe extern "C" fn(*const core::ffi::c_void, *mut core::ffi::c_void) -> i32;

#[test]
fn a_c_host_calls_a_table_built_at_run_time() {
    let Some(cc) = find_compiler() else {
        println!("skipped: no C compiler found; this is the test that a C host can call the table");
        return;
    };
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "{}described_caller{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compiled = std::process::Command::new(&cc)
        .args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .args(if cfg!(windows) {
            &[][..]
        } else {
            &["-fPIC"][..]
        })
        .arg("-I")
        .arg(manifest.join("../guatiao/include"))
        .arg("-I")
        .arg(manifest.join("../../examples/greeter_kind/include"))
        .arg("-o")
        .arg(&out)
        .arg(manifest.join("tests/c_consumer/described_caller.c"))
        .output()
        .expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C caller did not compile:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    // SAFETY: the library the compiler just wrote, which runs nothing on load.
    let library = unsafe { libloading::Library::new(&out) }.expect("it loads");
    // SAFETY: the signatures `described_caller.c` defines.
    let (greet, check, shout_is_null, drive_wide) = unsafe {
        (
            *library
                .get::<DriveGreet>(b"drive_greet\0")
                .expect("exported"),
            *library
                .get::<DriveCheck>(b"drive_check\0")
                .expect("exported"),
            *library
                .get::<DriveTable>(b"drive_shout_is_null\0")
                .expect("exported"),
            *library.get::<DriveWide>(b"drive_wide\0").expect("exported"),
        )
    };

    let k = kinds();
    let engine = Arc::new(Engine {
        listener: Caller::new(Arc::clone(&k.listener), &Libffi).expect("interfaces"),
        conversations: Mutex::new(BTreeMap::new()),
        next: Mutex::new(0),
        conversation_table: OnceLock::new(),
        counted: Mutex::new(0),
    });
    let greeter = ServedTable::new(k.greeter, |s| s.name() != "shout", engine.clone(), &Libffi)
        .expect("built");
    let wide = ServedTable::new(k.wide, |_| true, engine, &Libffi).expect("built");
    let ctx = core::ptr::without_provenance_mut(1);

    let mut answer = Map::new();
    let mut err = ProviderError::none();
    // SAFETY: a greeter table, a C string, writable out slots.
    let status = unsafe {
        greet(
            greeter.as_ptr(),
            ctx,
            c"ana".as_ptr(),
            &mut answer,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ana")
    );
    let mut refused = Map::new();
    // SAFETY: as above.
    let status = unsafe { greet(greeter.as_ptr(), ctx, c"".as_ptr(), &mut refused, &mut err) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(err.message(), "nobody to greet");

    let mut config = Map::new();
    let mut checked = false;
    let mut err = ProviderError::none();
    // SAFETY: a lent map and writable out slots.
    let status = unsafe { check(greeter.as_ptr(), ctx, &mut config, &mut checked, &mut err) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert!(checked);
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
    // SAFETY: a greeter table.
    assert_eq!(unsafe { shout_is_null(greeter.as_ptr()) }, 1);
    // SAFETY: a wide table and its `ctx`.
    assert_eq!(unsafe { drive_wide(wide.as_ptr(), ctx) }, 0);
}
