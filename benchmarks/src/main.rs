// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Times the paths a host takes through a loaded library: loading and
//! scanning, finding offers, calling through a kind's table, building an
//! instance, driving an object, lending one, cloning a handle, retiring a
//! library, and a library asking its host for another provider.
//!
//! `cargo run -p guatiao-bench --release -- --save NAME` writes
//! `benchmarks/results/NAME.json`; `benchmarks/README.md` has the schema
//! and the protocol.

extern crate alloc;

mod walk;

use alloc::collections::BTreeMap;
use core::ffi::c_void;
use core::hint::black_box;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::time::Instant;

use greeter_kind::{Conversation, Counter, Greeter, Listener};
use guatiao::library::{Offer, Registry};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::{Map, Status, Str, Value};

// --- the libraries under test -------------------------------------------

/// Where the example libraries are: cargo builds them beside this runner,
/// and the scan reads a directory holding a copy of each and nothing else.
struct Libraries {
    hello: PathBuf,
    derived: PathBuf,
    scan: PathBuf,
}

fn file_name(name: &str) -> String {
    format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn artifact(dir: &Path, name: &str) -> PathBuf {
    let file = file_name(name);
    [dir.join("deps").join(&file), dir.join(&file)]
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| {
            panic!(
                "{file} is not beside {}: build the workspace in this profile",
                dir.display()
            )
        })
}

impl Libraries {
    fn find() -> Libraries {
        let exe = std::env::current_exe().expect("the runner knows its own path");
        let dir = exe.parent().expect("it is in a directory").to_path_buf();
        let hello = artifact(&dir, "hello_library");
        let derived = artifact(&dir, "derived_greeter");
        // Copied over on every run: the copies of an earlier run are not
        // mapped by anyone once it has exited.
        let scan = dir.join("bench-scan");
        std::fs::create_dir_all(&scan).expect("the scan directory can be made");
        for (from, name) in [(&hello, "hello_library"), (&derived, "derived_greeter")] {
            std::fs::copy(from, scan.join(file_name(name))).expect("a library copies");
        }
        Libraries {
            hello,
            derived,
            scan,
        }
    }
}

fn registry(paths: &[&Path]) -> Registry {
    let mut registry = Registry::new("guatiao-bench", env!("CARGO_PKG_VERSION"));
    for path in paths {
        registry
            .load_file(path)
            .expect("loads")
            .loaded()
            .expect("a library for this host");
    }
    registry
}

fn greeter_offer(registry: &Registry, key: &str) -> Offer<dyn Greeter> {
    registry
        .offer::<dyn Greeter>(key)
        .expect("filed under its id")
        .expect("a valid table")
}

/// `iters` calls of `f`, timed as one.
fn batch(iters: u64, mut f: impl FnMut()) -> Duration {
    let started = Instant::now();
    for _ in 0..iters {
        f();
    }
    started.elapsed()
}

/// A listener that only counts, so lending it costs the crossing alone.
struct Quiet(u64);

impl Listener for Quiet {
    fn heard(&mut self, _what: &str) {
        self.0 += 1;
    }
}

/// `hello_library`'s echo table, as that library declares it.
#[repr(C)]
struct EchoVtable {
    struct_size: u32,
    echo: Option<unsafe extern "C" fn(ctx: *mut c_void, name: Str, out: *mut Value) -> Status>,
}

// --- the metrics ----------------------------------------------------------

/// One timed batch: `iters` calls, and how long they took. Setup the
/// metric needs per call happens inside and is left out of the time.
type Run = Box<dyn FnMut(u64) -> Duration>;

struct Metric {
    name: &'static str,
    setup: fn(&Libraries) -> Run,
}

const METRICS: &[Metric] = &[
    Metric {
        name: "load_file_hello",
        setup: load_file_hello,
    },
    Metric {
        name: "load_file_derived",
        setup: load_file_derived,
    },
    Metric {
        name: "scan_dir",
        setup: scan_dir,
    },
    Metric {
        name: "offers",
        setup: offers,
    },
    Metric {
        name: "call_scalar",
        setup: call_scalar,
    },
    Metric {
        name: "call_map",
        setup: call_map,
    },
    Metric {
        name: "call_map_copied",
        setup: call_map_copied,
    },
    Metric {
        name: "call_map_arg",
        setup: call_map_arg,
    },
    Metric {
        name: "call_map_mut",
        setup: call_map_mut,
    },
    Metric {
        name: "call_map_mut_moved",
        setup: call_map_mut_moved,
    },
    Metric {
        name: "call_string",
        setup: call_string,
    },
    Metric {
        name: "instantiate_drop",
        setup: instantiate_drop,
    },
    Metric {
        name: "object_ref",
        setup: object_ref,
    },
    Metric {
        name: "object_mut",
        setup: object_mut,
    },
    Metric {
        name: "lent_call",
        setup: lent_call,
    },
    Metric {
        name: "remote_clone",
        setup: remote_clone,
    },
    Metric {
        name: "retire",
        setup: retire,
    },
    Metric {
        name: "host_get",
        setup: host_get,
    },
    Metric {
        name: "clone_map",
        setup: clone_map,
    },
    Metric {
        name: "into_local_local",
        setup: into_local_local,
    },
    Metric {
        name: "into_local_foreign",
        setup: into_local_foreign,
    },
    Metric {
        name: "config_wire_tree",
        setup: walk::config_wire_tree,
    },
    Metric {
        name: "config_wire_walk",
        setup: walk::config_wire_walk,
    },
    Metric {
        name: "wire_config_tree",
        setup: walk::wire_config_tree,
    },
    Metric {
        name: "wire_config_walk",
        setup: walk::wire_config_walk,
    },
    Metric {
        name: "floats_wire_tree",
        setup: walk::floats_wire_tree,
    },
    Metric {
        name: "floats_wire_walk",
        setup: walk::floats_wire_walk,
    },
    Metric {
        name: "wire_floats_tree",
        setup: walk::wire_floats_tree,
    },
    Metric {
        name: "wire_floats_walk",
        setup: walk::wire_floats_walk,
    },
    Metric {
        name: "bytes_wire_tree",
        setup: walk::bytes_wire_tree,
    },
    Metric {
        name: "bytes_wire_walk",
        setup: walk::bytes_wire_walk,
    },
    Metric {
        name: "wire_bytes_tree",
        setup: walk::wire_bytes_tree,
    },
    Metric {
        name: "wire_bytes_walk",
        setup: walk::wire_bytes_walk,
    },
    Metric {
        name: "config_json_tree",
        setup: walk::config_json_tree,
    },
    Metric {
        name: "config_json_walk",
        setup: walk::config_json_walk,
    },
    Metric {
        name: "json_config_tree",
        setup: walk::json_config_tree,
    },
    Metric {
        name: "json_config_walk",
        setup: walk::json_config_walk,
    },
    Metric {
        name: "floats_json_tree",
        setup: walk::floats_json_tree,
    },
    Metric {
        name: "floats_json_walk",
        setup: walk::floats_json_walk,
    },
    Metric {
        name: "json_floats_tree",
        setup: walk::json_floats_tree,
    },
    Metric {
        name: "json_floats_walk",
        setup: walk::json_floats_walk,
    },
    Metric {
        name: "tree_json_config",
        setup: walk::tree_json_config,
    },
    Metric {
        name: "bytes_json_config",
        setup: walk::bytes_json_config,
    },
    Metric {
        name: "tree_json_keys",
        setup: walk::tree_json_keys,
    },
    Metric {
        name: "bytes_json_keys",
        setup: walk::bytes_json_keys,
    },
    Metric {
        name: "tree_events_config",
        setup: walk::tree_events_config,
    },
    Metric {
        name: "bytes_events_config",
        setup: walk::bytes_events_config,
    },
    Metric {
        name: "tree_events_keys",
        setup: walk::tree_events_keys,
    },
    Metric {
        name: "bytes_events_keys",
        setup: walk::bytes_events_keys,
    },
    Metric {
        name: "value_json_config",
        setup: walk::value_json_config,
    },
    Metric {
        name: "value_json_keys",
        setup: walk::value_json_keys,
    },
    Metric {
        name: "value_json_blob",
        setup: walk::value_json_blob,
    },
    Metric {
        name: "tree_json_blob",
        setup: walk::tree_json_blob,
    },
    Metric {
        name: "bytes_json_blob",
        setup: walk::bytes_json_blob,
    },
    Metric {
        name: "keys_wire_walk",
        setup: walk::keys_wire_walk,
    },
    Metric {
        name: "wire_keys_tree",
        setup: walk::wire_keys_tree,
    },
    Metric {
        name: "wire_keys_walk",
        setup: walk::wire_keys_walk,
    },
    Metric {
        name: "json_keys_tree",
        setup: walk::json_keys_tree,
    },
    Metric {
        name: "sink_keys_tree",
        setup: walk::sink_keys_tree,
    },
];

/// One `load_file` into a fresh registry; making and dropping the
/// registry is left out.
fn load_one(path: PathBuf) -> Run {
    Box::new(move |iters| {
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            let mut registry = Registry::new("guatiao-bench", "0");
            let started = Instant::now();
            let loaded = registry.load_file(&path).expect("loads").loaded().is_some();
            total += started.elapsed();
            assert!(loaded, "a library for this host");
        }
        total
    })
}

fn load_file_hello(libs: &Libraries) -> Run {
    load_one(libs.hello.clone())
}

fn load_file_derived(libs: &Libraries) -> Run {
    load_one(libs.derived.clone())
}

fn scan_dir(libs: &Libraries) -> Run {
    let dir = libs.scan.clone();
    Box::new(move |iters| {
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            let mut registry = Registry::new("guatiao-bench", "0");
            let started = Instant::now();
            let report =
                guatiao::library::scan::scan_dir(&mut registry, &dir).expect("the directory reads");
            total += started.elapsed();
            black_box(report);
            assert_eq!(registry.loaded().len(), 2, "both copies load");
        }
        total
    })
}

fn offers(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.hello, &libs.derived]);
    Box::new(move |iters| {
        batch(iters, || {
            let found: Vec<Offer<dyn Greeter>> = registry.offers::<dyn Greeter>().collect();
            assert_eq!(found.len(), 2);
            black_box(found);
        })
    })
}

fn call_scalar(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = registry
        .offer::<dyn Counter>("derived_greeter_hello")
        .expect("filed under its id")
        .expect("a valid table");
    Box::new(move |iters| {
        let _keep = &registry;
        let counter = offer.default_instance().expect("its own instance");
        batch(iters, || {
            black_box(counter.count());
        })
    })
}

fn call_map(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    Box::new(move |iters| {
        let _keep = &registry;
        let greeter = offer.default_instance().expect("its own instance");
        batch(iters, || {
            let answer: Map = greeter.greet("ana").expect("greets");
            black_box(answer);
        })
    })
}

/// `call_map` through a greeter that builds its map in its own allocator,
/// which the caller then copies: what an implementation that does not
/// build where it is asked costs.
fn call_map_copied(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let shouter = greeter_offer(&registry, "derived_greeter_shouter");
    let mut config = Map::new();
    config.set("prefix", "hey");
    let instance = shouter
        .instantiate(&Value::from(config))
        .expect("a fitting configuration");
    Box::new(move |iters| {
        let _keep = &registry;
        batch(iters, || {
            let answer: Map = instance.greet("ana").expect("greets");
            black_box(answer);
        })
    })
}

/// `Greeter::greet_from`, handing over a one-entry map built for each
/// call: the map's building and its move are what this adds to `call_map`.
fn call_map_arg(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    Box::new(move |iters| {
        let _keep = &registry;
        let greeter = offer.default_instance().expect("its own instance");
        batch(iters, || {
            let mut config = Map::new();
            config.set("name", "ana");
            let answer: Map = greeter.greet_from(config).expect("greets");
            black_box(answer);
        })
    })
}

/// `Greeter::check`, lending a one-entry `Map` built for the call, to
/// which the provider adds an entry built in the map's own allocator: the
/// walk after the call finds nothing to move.
fn call_map_mut(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    Box::new(move |iters| {
        let _keep = &registry;
        let greeter = offer.default_instance().expect("its own instance");
        batch(iters, || {
            let mut config = Map::new();
            config.set("name", "ana");
            assert!(greeter.check(&mut config).expect("checks"));
            black_box(config);
        })
    })
}

/// `call_map_mut` through a configured instance whose `check` builds the
/// entry's value in its own allocator, so the proxy moves it into the
/// map's after the call.
fn call_map_mut_moved(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let shouter = greeter_offer(&registry, "derived_greeter_shouter");
    let mut settings = Map::new();
    settings.set("prefix", "hey");
    let instance = shouter
        .instantiate(&Value::from(settings))
        .expect("a fitting configuration");
    Box::new(move |iters| {
        let _keep = &registry;
        batch(iters, || {
            let mut config = Map::new();
            config.set("name", "ana");
            assert!(instance.check(&mut config).expect("checks"));
            black_box(config);
        })
    })
}

fn call_string(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    Box::new(move |iters| {
        let _keep = &registry;
        let greeter = offer.default_instance().expect("its own instance");
        batch(iters, || {
            black_box(greeter.shout("ana"));
        })
    })
}

fn instantiate_drop(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let shouter = greeter_offer(&registry, "derived_greeter_shouter");
    let mut config = Map::new();
    config.set("prefix", "hey");
    let config = Value::from(config);
    Box::new(move |iters| {
        let _keep = &registry;
        batch(iters, || {
            let instance = shouter
                .instantiate(&config)
                .expect("a fitting configuration");
            black_box(&instance);
        })
    })
}

/// A conversation the derived greeter started, with one thing said.
fn conversation(libs: &Libraries) -> (Registry, guatiao::library::Object<dyn Conversation>) {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    let mut chat = offer
        .default_instance()
        .expect("its own instance")
        .start(Quiet(0).into_object())
        .expect("holds conversations");
    chat.say("hi").expect("says");
    (registry, chat)
}

fn object_ref(libs: &Libraries) -> Run {
    let (registry, chat) = conversation(libs);
    Box::new(move |iters| {
        let _keep = &registry;
        batch(iters, || {
            black_box(chat.turns());
        })
    })
}

fn object_mut(libs: &Libraries) -> Run {
    let (registry, mut chat) = conversation(libs);
    Box::new(move |iters| {
        let _keep = &registry;
        let mut buffer = [0u8; 8];
        batch(iters, || {
            black_box(chat.read(&mut buffer));
        })
    })
}

fn lent_call(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    Box::new(move |iters| {
        let _keep = &registry;
        let greeter = offer.default_instance().expect("its own instance");
        let mut quiet = Quiet(0);
        batch(iters, || {
            greeter.greet_into("eve", &mut quiet).expect("greets");
        })
    })
}

// A `Remote` was `Copy` before handles held a lease; the metric is the
// same call either way.
#[allow(clippy::clone_on_copy)]
fn remote_clone(libs: &Libraries) -> Run {
    let registry = registry(&[&libs.derived]);
    let offer = greeter_offer(&registry, "derived_greeter_hello");
    let remote = offer.remote().expect("its own instance");
    Box::new(move |iters| {
        let _keep = (&registry, &offer);
        batch(iters, || {
            black_box(remote.clone());
        })
    })
}

/// One `retire` of a library loaded just before it; the load is left out.
fn retire(libs: &Libraries) -> Run {
    let path = libs.derived.clone();
    let mut registry = registry(&[&libs.hello]);
    Box::new(move |iters| {
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            registry
                .load_file(&path)
                .expect("loads")
                .loaded()
                .expect("loads again once retired");
            let started = Instant::now();
            let retired = registry.retire("derived_greeter").expect("retires");
            total += started.elapsed();
            black_box(retired);
        }
        total
    })
}

/// `hello_library`'s echo: a library listing the host's greeters through
/// its services, reading each descriptor, and calling one.
fn host_get(libs: &Libraries) -> Run {
    // `hello_library` last: its echo asks the host that loaded it most
    // recently.
    let registry = registry(&[&libs.derived, &libs.hello]);
    let provider = registry
        .provider("hello_library_echo")
        .expect("filed under its id");
    let (table, size) = provider.vtable();
    assert!(!table.is_null() && size >= size_of::<EchoVtable>());
    // SAFETY: `hello_library` declares this table for its echo provider,
    // laid out as `EchoVtable`, and the registry keeps it.
    let echo = unsafe { (*table.cast::<EchoVtable>()).echo }.expect("the echo slot");
    let ctx = provider.ctx();
    Box::new(move |iters| {
        let _keep = &registry;
        batch(iters, || {
            let mut out = Value::absent();
            // SAFETY: the slot's contract: a readable name and a writable
            // node, which the caller then owns.
            let status = unsafe { echo(ctx, Str::new("ana"), &mut out) };
            assert_eq!(status, Status::GUATIAO_OK);
            black_box(out);
        })
    })
}

/// Eight entries: text, numbers, a nested map and a list, all local.
fn sample_map() -> Map {
    let mut nested = Map::new();
    nested.set("depth", 2);
    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);
    map.set("tls", true);
    map.set("user", "ana");
    map.set("timeout", 30);
    map.set("nested", nested);
    map.set(
        "tags",
        ["a", "b", "c"].into_iter().collect::<guatiao::List>(),
    );
    map.set("note", "kept as it is");
    map
}

/// `clone()` of `sample_map`, and dropping the copy.
fn clone_map(_: &Libraries) -> Run {
    let map = sample_map();
    Box::new(move |iters| {
        batch(iters, || {
            black_box(black_box(&map).clone());
        })
    })
}

/// `into_local()` of a tree that is already local: judged, handed back.
fn into_local_local(_: &Libraries) -> Run {
    let mut held = Some(sample_map());
    Box::new(move |iters| {
        batch(iters, || {
            let map = held.take().expect("put back each time");
            held = Some(black_box(map.into_local()));
        })
    })
}

/// A tree moved between this binary's allocator and another, alternately:
/// each call is what `into_local()` costs a tree from elsewhere, one copy
/// and one free of the original.
fn into_local_foreign(_: &Libraries) -> Run {
    // Rust's heap under another address, standing for a library's or a
    // host's allocator; leaked, as an allocator outlives its trees.
    let vtable: &'static Allocator = Box::leak(Box::new(rust_alloc()));
    // SAFETY: a complete vtable that lives for the program.
    let other = unsafe { Alloc::from_raw(vtable) }.expect("a complete vtable");
    let mut held = Some(sample_map().into_alloc(other).expect("copied"));
    let mut home = true;
    Box::new(move |iters| {
        batch(iters, || {
            let map = held.take().expect("put back each time");
            let moved = if home {
                map.into_local()
            } else {
                map.into_alloc(other).expect("copied")
            };
            home = !home;
            held = Some(black_box(moved));
        })
    })
}

// --- running --------------------------------------------------------------

struct Options {
    name: Option<String>,
    out: Option<PathBuf>,
    rounds: usize,
    samples: usize,
    target: Duration,
    only: Vec<String>,
    merge: Vec<PathBuf>,
}

fn results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("results")
}

const USAGE: &str = "\
guatiao-bench [--save NAME | --out FILE [--name NAME]] [--rounds N] [--samples N]
              [--target-ms N] [--only METRIC,...]
guatiao-bench --merge FILE... (--save NAME | --out FILE [--name NAME])

  --save NAME     write benchmarks/results/NAME.json
  --out FILE      write FILE instead
  --rounds N      rounds, each running every metric in turn (default 6)
  --samples N     timed samples per metric per round (default 15)
  --target-ms N   how long one sample runs (default 5)
  --only LIST     run only these metrics
  --merge FILE... pool the samples of several result files into one";

fn parse_options() -> Options {
    let mut options = Options {
        name: None,
        out: None,
        rounds: 6,
        samples: 15,
        target: Duration::from_millis(5),
        only: Vec::new(),
        merge: Vec::new(),
    };
    let mut args = std::env::args().skip(1).peekable();
    let value = |args: &mut dyn Iterator<Item = String>, flag: &str| {
        args.next()
            .unwrap_or_else(|| fail(&format!("{flag} takes a value")))
    };
    let number = |text: String, flag: &str| {
        text.parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or_else(|| fail(&format!("{flag} takes a positive number")))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save" => {
                let name = value(&mut args, "--save");
                options.out = Some(results_dir().join(format!("{name}.json")));
                options.name = Some(name);
            }
            "--out" => options.out = Some(PathBuf::from(value(&mut args, "--out"))),
            "--name" => options.name = Some(value(&mut args, "--name")),
            "--rounds" => options.rounds = number(value(&mut args, "--rounds"), "--rounds"),
            "--samples" => options.samples = number(value(&mut args, "--samples"), "--samples"),
            "--target-ms" => {
                let ms = number(value(&mut args, "--target-ms"), "--target-ms");
                options.target = Duration::from_millis(ms as u64);
            }
            "--only" => {
                options.only = value(&mut args, "--only")
                    .split(',')
                    .map(str::to_string)
                    .collect();
            }
            "--merge" => {
                while let Some(file) = args.next_if(|a| !a.starts_with("--")) {
                    options.merge.push(PathBuf::from(file));
                }
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => fail(&format!("unknown argument `{other}`")),
        }
    }
    options
}

fn fail(why: &str) -> ! {
    eprintln!("guatiao-bench: {why}\n\n{USAGE}");
    std::process::exit(2);
}

/// What one run, or several pooled, measured.
#[derive(Default)]
struct Results {
    rounds: usize,
    samples_per_round: usize,
    runs: usize,
    processor: String,
    os: String,
    /// Milliseconds per call, every sample.
    samples: BTreeMap<String, Vec<f64>>,
    /// Each round's median, in milliseconds per call.
    round_medians: BTreeMap<String, Vec<f64>>,
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v
}

/// Doubles a batch until it runs for half the target, then scales.
fn calibrate(run: &mut Run, target: Duration) -> u64 {
    let mut iters = 1u64;
    loop {
        let took = run(iters);
        if took >= target / 2 || iters >= 1 << 30 {
            let per_call = took.as_secs_f64() / iters as f64;
            return ((target.as_secs_f64() / per_call.max(1e-12)) as u64).max(1);
        }
        iters *= 2;
    }
}

fn measure(options: &Options) -> Results {
    let libs = Libraries::find();
    let metrics: Vec<&Metric> = METRICS
        .iter()
        .filter(|m| options.only.is_empty() || options.only.iter().any(|o| o == m.name))
        .collect();
    if metrics.is_empty() {
        fail("no metric matches --only");
    }
    let mut results = Results {
        rounds: options.rounds,
        samples_per_round: options.samples,
        runs: 1,
        processor: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        ..Results::default()
    };
    let mut iterations: BTreeMap<&str, u64> = BTreeMap::new();
    for round in 0..options.rounds {
        // Each round starts one metric later, so no metric always runs
        // first, or always after the same neighbour.
        for at in 0..metrics.len() {
            let metric = metrics[(at + round) % metrics.len()];
            let mut run = (metric.setup)(&libs);
            let iters = *iterations
                .entry(metric.name)
                .or_insert_with(|| calibrate(&mut run, options.target));
            run(iters);
            let mut these = Vec::with_capacity(options.samples);
            for _ in 0..options.samples {
                let took = run(iters);
                these.push(took.as_secs_f64() * 1e3 / iters as f64);
            }
            results
                .round_medians
                .entry(metric.name.to_string())
                .or_default()
                .push(median(&sorted(&these)));
            results
                .samples
                .entry(metric.name.to_string())
                .or_default()
                .extend(these);
        }
        eprintln!("round {} of {} done", round + 1, options.rounds);
    }
    results
}

fn report(results: &Results) {
    println!(
        "{:<20} {:>12} {:>12} {:>12}   (microseconds per call)",
        "metric", "min", "median", "max"
    );
    for (name, samples) in &results.samples {
        let s = sorted(samples);
        println!(
            "{:<20} {:>12.4} {:>12.4} {:>12.4}",
            name,
            s[0] * 1e3,
            median(&s) * 1e3,
            s[s.len() - 1] * 1e3
        );
    }
}

// --- the result file ------------------------------------------------------

fn number_list(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| format!("{v:.6e}")).collect();
    format!("[{}]", items.join(", "))
}

fn to_json(name: &str, results: &Results) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"name\": \"{name}\",\n"));
    out.push_str("  \"metrics\": {\n");
    let metrics: Vec<String> = results
        .samples
        .iter()
        .map(|(metric, samples)| {
            let s = sorted(samples);
            format!(
                "    \"{metric}\": {{\"min_ms\": {:.6e}, \"median_ms\": {:.6e}, \"max_ms\": {:.6e}}}",
                s[0],
                median(&s),
                s[s.len() - 1]
            )
        })
        .collect();
    out.push_str(&metrics.join(",\n"));
    out.push_str("\n  },\n  \"raw\": {\n");
    out.push_str(&format!("    \"processor\": \"{}\",\n", results.processor));
    out.push_str(&format!("    \"os\": \"{}\",\n", results.os));
    out.push_str(&format!("    \"runs\": {},\n", results.runs));
    out.push_str(&format!("    \"rounds\": {},\n", results.rounds));
    out.push_str(&format!(
        "    \"samples_per_round\": {},\n",
        results.samples_per_round
    ));
    for (key, map, last) in [
        ("round_medians_ms", &results.round_medians, false),
        ("samples_ms", &results.samples, true),
    ] {
        out.push_str(&format!("    \"{key}\": {{\n"));
        let rows: Vec<String> = map
            .iter()
            .map(|(metric, values)| format!("      \"{metric}\": {}", number_list(values)))
            .collect();
        out.push_str(&rows.join(",\n"));
        out.push_str(if last { "\n    }\n" } else { "\n    },\n" });
    }
    out.push_str("  }\n}\n");
    out
}

/// Just enough JSON to read back what [`to_json`] writes.
#[derive(Debug)]
enum Json {
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(String, Json)>),
    Other,
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Json::Text(t) => Some(t),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip_space(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_space();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Ok(Json::Object(fields));
                }
                loop {
                    self.skip_space();
                    let Json::Text(key) = self.value()? else {
                        return Err(format!("a key at byte {}", self.at));
                    };
                    if !self.eat(b':') {
                        return Err(format!("`:` at byte {}", self.at));
                    }
                    fields.push((key, self.value()?));
                    if self.eat(b'}') {
                        return Ok(Json::Object(fields));
                    }
                    if !self.eat(b',') {
                        return Err(format!("`,` or `}}` at byte {}", self.at));
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::List(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b']') {
                        return Ok(Json::List(items));
                    }
                    if !self.eat(b',') {
                        return Err(format!("`,` or `]` at byte {}", self.at));
                    }
                }
            }
            Some(b'"') => {
                self.at += 1;
                let start = self.at;
                while self.at < self.bytes.len() && self.bytes[self.at] != b'"' {
                    self.at += 1;
                }
                let text = core::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| "text that is not UTF-8".to_string())?
                    .to_string();
                self.at += 1;
                Ok(Json::Text(text))
            }
            Some(_) => {
                let start = self.at;
                while self.at < self.bytes.len()
                    && !matches!(self.bytes[self.at], b',' | b'}' | b']')
                    && !self.bytes[self.at].is_ascii_whitespace()
                {
                    self.at += 1;
                }
                let word = core::str::from_utf8(&self.bytes[start..self.at]).unwrap_or("");
                Ok(word.parse::<f64>().map_or(Json::Other, Json::Number))
            }
            None => Err("an early end".to_string()),
        }
    }
}

fn numbers(json: Option<&Json>) -> BTreeMap<String, Vec<f64>> {
    let mut out = BTreeMap::new();
    if let Some(Json::Object(fields)) = json {
        for (metric, values) in fields {
            if let Json::List(items) = values {
                out.insert(
                    metric.clone(),
                    items.iter().filter_map(Json::number).collect(),
                );
            }
        }
    }
    out
}

fn merge(files: &[PathBuf]) -> Results {
    let mut pooled = Results::default();
    for file in files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| fail(&format!("{} cannot be read: {e}", file.display())));
        let json = Reader {
            bytes: text.as_bytes(),
            at: 0,
        }
        .value()
        .unwrap_or_else(|e| {
            fail(&format!(
                "{} is not a result file: expected {e}",
                file.display()
            ))
        });
        let raw = json
            .get("raw")
            .unwrap_or_else(|| fail(&format!("{} has no `raw` section", file.display())));
        let processor = raw.get("processor").and_then(Json::text).unwrap_or("");
        let os = raw.get("os").and_then(Json::text).unwrap_or("");
        if pooled.runs == 0 {
            pooled.processor = processor.to_string();
            pooled.os = os.to_string();
            pooled.rounds = raw.get("rounds").and_then(Json::number).unwrap_or(0.0) as usize;
            pooled.samples_per_round = raw
                .get("samples_per_round")
                .and_then(Json::number)
                .unwrap_or(0.0) as usize;
        } else if (processor, os) != (pooled.processor.as_str(), pooled.os.as_str()) {
            fail(&format!(
                "{} was measured on another platform",
                file.display()
            ));
        }
        pooled.runs += raw.get("runs").and_then(Json::number).unwrap_or(1.0) as usize;
        for (key, into) in [
            ("samples_ms", &mut pooled.samples),
            ("round_medians_ms", &mut pooled.round_medians),
        ] {
            for (metric, values) in numbers(raw.get(key)) {
                into.entry(metric).or_default().extend(values);
            }
        }
    }
    pooled
}

fn main() {
    let options = parse_options();
    let results = if options.merge.is_empty() {
        measure(&options)
    } else {
        merge(&options.merge)
    };
    report(&results);
    if let Some(out) = &options.out {
        let name = options.name.clone().unwrap_or_else(|| {
            out.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).expect("the results directory can be made");
        }
        std::fs::write(out, to_json(&name, &results)).expect("the result file writes");
        eprintln!("wrote {}", out.display());
    }
}
