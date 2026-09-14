// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A host, end to end: where it looks, what it found, and how it builds a
//! provider from a configuration it wrote itself.
//!
//! `examples/greeter_kind` is the two traits a host and a library both
//! compile against. `examples/derived_greeter` is one library offering
//! them. This is the other side -- the program that maps whatever
//! libraries are on disk and calls them as the traits:
//!
//! 1. **Where it looks** is a search path: the arguments, else
//!    `GREETER_HOST_PATH`, else the directory this executable is in.
//!    Every entry is walked once and the report says what happened at
//!    each place -- a library that loaded, a file that was not one, an
//!    entry that could not be read.
//! 2. **What it found** is asked for as `dyn Greeter`: an [`Offer`] per
//!    provider whose table fits the trait. A library with a hand-written
//!    table of the same kind is loaded but is not an offer, and that is
//!    the difference between "the file is a library" and "I can call it".
//! 3. **A configured provider** is built from a value the host made
//!    with its own derived type -- `#[derive(ToValue, Schema)]` on a
//!    struct the host owns -- checked against the schema the provider
//!    declared, and handed to `instantiate`. The instance is the trait.
//! 4. **Objects cross both ways**: the host implements `Listener` (an
//!    object kind) and hands one in; the provider hands a `Conversation`
//!    back; the host drives it with `&mut` calls, reads into its own
//!    buffer, and drops it. Each side destroys what it holds last.
//!
//! ```text
//! cargo build --workspace            # puts derived_greeter beside this binary
//! cargo run -p greeter_host          # scans target/debug
//! cargo run -p greeter_host -- /some/dir /some/lib.so
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use greeter_kind::{Counter, Greeter, Listener};
use guatiao::library::{Kind, Offer, Order, Registry, ScanRules, SearchPath, Skipped, scan_path};
use guatiao::schema::{SchemaRef, validate_map};
use guatiao::{Alloc, Schema, ToValue, Value};

/// What this host wants a shouting greeter to start with.
///
/// The provider declares what it takes as a schema; this host declares
/// what it has as a type. Neither knows the other's spelling, so the value
/// crossing between them is checked against the schema before it is
/// offered -- a refusal names the key, not the value, so it is safe to
/// print.
#[derive(ToValue, Schema, Debug)]
struct ShoutSettings {
    /// What every shout starts with.
    prefix: String,
}

fn main() -> ExitCode {
    let path = search_path();
    println!("looking in:");
    for entry in path.entries() {
        println!("  {}", entry.display());
    }

    // The host's registry: its own id and version, and its own allocator
    // for every value that crosses back (`None` is the Rust one).
    let mut registry = Registry::with_alloc("greeter_host", env!("CARGO_PKG_VERSION"), None);

    // Only libraries that declare a greeter are mapped. The declaration is
    // read out of the file as data before anything in it runs, so a
    // directory holding unrelated libraries costs nothing but the read.
    let rules = ScanRules::parse(&[format!("kind={}", <dyn Greeter as Kind>::NAME)])
        .expect("a rule written by hand");
    let report = scan_path(&mut registry, &path, Order::Ascending, &rules);

    println!("\nloaded ({}):", report.loaded.len());
    for file in &report.loaded {
        println!("  {}", file.display());
    }
    if !report.skipped.is_empty() {
        println!("skipped ({}):", report.skipped.len());
        for (file, why) in &report.skipped {
            println!("  {}: {}", file.display(), skip_reason(why));
        }
    }
    for (file, error) in &report.failed {
        println!("failed: {}: {error}", file.display());
    }
    for (place, error) in &report.unreadable {
        println!("unreadable: {}: {error}", place.display());
    }
    if report.loaded.is_empty() {
        eprintln!(
            "\nno greeter library found -- `cargo build --workspace` first, or name a directory"
        );
        return ExitCode::FAILURE;
    }

    // Everything the libraries registered, whatever its kind and however
    // its table was written.
    println!("\nproviders ({}):", registry.all().len());
    for provider in registry.all() {
        println!(
            "  {} [{}] from {}",
            provider.id(),
            provider.kinds().join(", "),
            provider.library()
        );
    }

    // The ones this host can call as the trait. An offer derefs to the
    // trait, so `greet` here is the proxy the kind generated, marshalling
    // across the table.
    let offers: Vec<Offer<dyn Greeter>> = registry.offers::<dyn Greeter>().collect();
    println!("\noffers as dyn Greeter ({}):", offers.len());
    for offer in &offers {
        let availability = match offer.available() {
            Ok(()) => "available".to_string(),
            Err(why) => format!("unavailable: {why}"),
        };
        println!("  {} ({availability})", offer.id());
    }
    for (provider, mismatch) in registry.mismatches::<dyn Greeter>() {
        println!(
            "  {} is a greeter this host cannot call: {mismatch}",
            provider.id()
        );
    }

    // 2. A provider that is its one instance answers as it is.
    for offer in offers.iter().filter(|o| !o.builds_instances()) {
        match offer.greet("ana") {
            Ok(answer) => println!(
                "\n{} greets: {}",
                offer.id(),
                answer
                    .get("greeting")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
            ),
            Err(e) => println!("\n{} refused: {e}", offer.id()),
        }
        println!("{} shouts: {}", offer.id(), offer.shout("ana"));
        // The same provider through its other kind, if it serves one.
        if let Some(Ok(counter)) = offer
            .key()
            .and_then(|key| registry.offer::<dyn Counter>(key))
        {
            println!("{} has been asked {} time(s)", offer.id(), counter.count());
        }
    }

    // 3. A provider built from a configuration: read what it takes, build
    // what this host has, check the one against the other, instantiate.
    let mut status = ExitCode::SUCCESS;
    for offer in offers.iter().filter(|o| o.builds_instances()) {
        let Some(schema) = offer.config_schema() else {
            println!("\n{} builds instances but declares no schema", offer.id());
            continue;
        };
        println!("\n{} takes:", offer.id());
        println!("{}", indent(&render(schema)));

        let settings = ShoutSettings {
            prefix: "hey".to_string(),
        };
        let config = settings
            .to_value(Alloc::rust())
            .expect("a host's own type converts");
        // The host's schema for the type it wrote, beside the provider's
        // for the value it takes: the two are independent documents that
        // happen to agree, and `validate_map` is what says so.
        println!("this host offers: {}", render(&config));
        let provider_schema = SchemaRef::new(schema).expect("a schema is a map");
        if let Err(e) = validate_map(provider_schema, &config) {
            println!("{} would refuse this configuration: {e}", offer.id());
            status = ExitCode::FAILURE;
            continue;
        }
        match offer.instantiate(&config) {
            Ok(instance) => match instance.greet("ana") {
                Ok(answer) => println!(
                    "{} configured with {:?} greets: {}",
                    offer.id(),
                    settings,
                    answer
                        .get("greeting")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                ),
                Err(e) => println!("{} refused: {e}", offer.id()),
            },
            Err(e) => {
                println!("{} could not be built: {e}", offer.id());
                status = ExitCode::FAILURE;
            }
        }
        // And what the provider says to a value that does not fit -- the
        // refusal comes from the provider's decode, across the boundary.
        let empty = Value::map();
        if let Err(e) = offer.instantiate(&empty) {
            println!("{} given nothing: {e}", offer.id());
        }
    }

    // 4. An object: the host hands a listener IN (an object kind this
    // program implements, owned by the library from then on), gets a
    // conversation OUT (an object kind the library implements, owned by
    // this program), drives it, reads into its own buffer, drops it --
    // and both objects are destroyed on the side that holds them last.
    let heard = Arc::new(Mutex::new(Vec::new()));
    let listener_dropped = Arc::new(AtomicBool::new(false));
    for offer in offers.iter().filter(|o| !o.builds_instances()) {
        let listener = Echo {
            heard: Arc::clone(&heard),
            dropped: Arc::clone(&listener_dropped),
        }
        .into_object();
        match offer.start(listener) {
            Ok(mut chat) => {
                for line in ["good morning", "how are you"] {
                    if let Err(e) = chat.say(line) {
                        println!("{} would not hear \"{line}\": {e}", offer.id());
                    }
                }
                let mut buffer = [0u8; 64];
                let n = chat.read(&mut buffer);
                let transcript = String::from_utf8_lossy(&buffer[..n.max(0) as usize]).into_owned();
                println!(
                    "\n{} held a conversation of {} turn(s); the transcript read back into a {}-byte buffer:",
                    offer.id(),
                    chat.turns(),
                    buffer.len()
                );
                println!("{}", indent(&transcript));
                drop(chat);
                println!(
                    "the listener heard {:?} and was dropped with the conversation: {}",
                    heard.lock().map(|h| h.clone()).unwrap_or_default(),
                    listener_dropped.load(Ordering::SeqCst)
                );
            }
            Err(e) => println!("\n{} holds no conversations: {e}", offer.id()),
        }
    }
    status
}

/// This program's listener: an object kind implemented on the HOST side
/// and handed to the library, which calls it back across the boundary
/// and destroys it when the conversation ends.
struct Echo {
    heard: Arc<Mutex<Vec<String>>>,
    dropped: Arc<AtomicBool>,
}

impl Listener for Echo {
    fn heard(&mut self, what: &str) {
        if let Ok(mut heard) = self.heard.lock() {
            heard.push(what.to_string());
        }
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// The arguments, else `GREETER_HOST_PATH`, else beside this executable.
///
/// Adjacency accumulates with the override rather than being replaced by
/// it, and a place named twice is one entry.
fn search_path() -> SearchPath {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path = if args.is_empty() {
        SearchPath::parse(&std::env::var("GREETER_HOST_PATH").unwrap_or_default())
    } else {
        let mut path = SearchPath::new();
        for arg in args {
            path.push(PathBuf::from(arg));
        }
        path
    };
    if path.is_empty()
        && let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        path.push(dir);
    }
    path
}

/// One line per reason, in the host's words.
fn skip_reason(why: &Skipped) -> String {
    match why {
        Skipped::NoEntrySymbol => "not a guatiao library".to_string(),
        Skipped::Filtered { by } => format!("declares nothing for the rule `{by}`"),
        Skipped::AlreadyLoaded { from } => format!("already loaded from {}", from.display()),
        Skipped::ProviderAlreadyLoaded { id, .. } => {
            format!("its provider `{id}` is already registered")
        }
        Skipped::DeclinedThisHost => "declined this host".to_string(),
        Skipped::NotExaminable => "could not be examined without running it".to_string(),
        Skipped::UnsupportedAbi { declared } => format!("built for envelope ABI {declared}"),
        other => format!("{other:?}"),
    }
}

fn render(value: &Value) -> String {
    guatiao_serde::text::json::to_string_pretty(value, guatiao_serde::Presentation::new())
        .unwrap_or_else(|e| format!("<not renderable: {e}>"))
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
