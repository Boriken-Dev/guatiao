// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The protocol from C: a codec checked against the shared vectors, a C
//! client calling a Rust server, and a Rust host calling a C server, at
//! the trusted level over TCP. Each skips, saying why, without a C
//! compiler or a built guatiao library.

#![cfg(feature = "tcp")]

mod common;

use common::procs::{self, finish, run, start};

use core::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use greeter_kind::Greeter;
use guatiao::value::convert::TryAsRef;
use guatiao::value::wire;
use guatiao::{List, Map, Status, Value};
use guatiao_source_remote::transport::tcp::{Accepted, Range, TcpConnect, TcpListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};
use serde_json::Value as Json;

const KEY: &str = "test-key";

/// The guatiao library beside this test binary, for the C peer's values.
fn guatiao_library() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    let name = format!(
        "{}guatiao{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [deps.to_path_buf(), deps.parent()?.to_path_buf()] {
        if dir.join(&name).is_file() {
            return Some((dir.clone(), dir.join(&name)));
        }
    }
    None
}

fn here() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vectors() -> Json {
    serde_json::from_str(
        &std::fs::read_to_string(here().join("tests/vectors/remote.json")).unwrap(),
    )
    .unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn tree(t: &Json) -> Value {
    let a = t.as_array().unwrap();
    match a[0].as_str().unwrap() {
        "number" => Value::from(guatiao::Number::new(a[1].as_str().unwrap()).unwrap()),
        "string" => Value::from(a[1].as_str().unwrap()),
        "map" => {
            let mut m = Map::new();
            for kv in a[1].as_array().unwrap() {
                m.set(kv[0].as_str().unwrap(), tree(&kv[1]));
            }
            Value::from(m)
        }
        other => panic!("{other}"),
    }
}

fn c_bytes(b: &[u8]) -> String {
    let mut s = String::from("(const uint8_t[]){");
    for x in b {
        write!(s, "0x{x:02x},").unwrap();
    }
    if b.is_empty() {
        s.push('0');
    }
    s.push('}');
    s
}

fn c_text(s: &str) -> String {
    format!("{s:?}")
}

/// `remote_vectors.h`: every frame vector as C data.
fn vectors_header() -> String {
    let doc = vectors();
    let mut h = String::from(
        "#include <stddef.h>\n#include <stdint.h>\n\
         typedef struct gr_vector { const char *name; const uint8_t *frame; size_t frame_len; \
         uint8_t type; uint64_t call, parent, target_code, slot, status, n, object; uint8_t trust; \
         const uint8_t *nonce; const uint8_t *blob; size_t blob_len; const uint8_t *options; \
         size_t options_len; const char *s1, *s2; } gr_vector;\n\
         typedef struct gr_proof_vector { const char *name; const uint8_t *key; size_t key_len; \
         const char *label; const uint8_t *client, *server, *proof; } gr_proof_vector;\n\
         static const gr_vector GR_VECTORS[] = {\n",
    );
    let frames = doc["frames"].as_array().unwrap();
    for v in frames {
        let f = &v["frame"];
        let n = |k: &str| f.get(k).and_then(Json::as_u64).unwrap_or(0);
        let ty: u8 = match f["type"].as_str().unwrap() {
            "hello" => 0x01,
            "welcome" => 0x02,
            "proof" => 0x03,
            "list" => 0x04,
            "open" => 0x05,
            "close" => 0x06,
            "call" => 0x10,
            "reply" => 0x11,
            "cancel" => 0x12,
            "release" => 0x13,
            "ping" => 0x14,
            "pong" => 0x15,
            "goodbye" => 0x16,
            other => panic!("{other}"),
        };
        let frame = unhex(v["hex"].as_str().unwrap());
        let blob = ["proof", "args", "body"]
            .iter()
            .find_map(|k| f.get(*k).and_then(Json::as_str))
            .map(unhex)
            .unwrap_or_default();
        let options = f
            .get("options")
            .map(|o| wire::to_bytes(&tree(o)).unwrap())
            .unwrap_or_default();
        let target = f.get("target").map_or(0, |t| {
            let id = t[1].as_u64().unwrap();
            (id << 1) | u64::from(t[0] == "object")
        });
        let trust = match f.get("trust").and_then(Json::as_str) {
            Some("trusted") => 1,
            Some("secure") => 2,
            _ => 0,
        };
        let nonce = f
            .get("nonce")
            .and_then(Json::as_str)
            .map_or("NULL".to_string(), |h| c_bytes(&unhex(h)));
        let (s1, s2) = match ty {
            0x05 | 0x06 => (
                f["library"].as_str().unwrap().to_string(),
                f["stamp"].as_str().unwrap().to_string(),
            ),
            0x16 => (f["message"].as_str().unwrap().to_string(), String::new()),
            _ => (String::new(), String::new()),
        };
        writeln!(
            h,
            "  {{{}, {}, {}, {ty}, {}u, {}u, {target}u, {}u, {}u, {}u, {}u, {trust}, {nonce}, {}, {}, {}, {}, {}, {}}},",
            c_text(v["name"].as_str().unwrap()),
            c_bytes(&frame),
            frame.len(),
            n("call"),
            n("parent"),
            n("slot"),
            n("status"),
            n("n"),
            n("object"),
            c_bytes(&blob),
            blob.len(),
            c_bytes(&options),
            options.len(),
            c_text(&s1),
            c_text(&s2),
        )
        .unwrap();
    }
    writeln!(
        h,
        "}};\nstatic const size_t GR_VECTOR_COUNT = {};",
        frames.len()
    )
    .unwrap();
    h.push_str("static const gr_proof_vector GR_PROOFS[] = {\n");
    let proofs = doc["proofs"].as_array().unwrap();
    for p in proofs {
        let key = unhex(p["key"].as_str().unwrap());
        writeln!(
            h,
            "  {{{}, {}, {}, {}, {}, {}, {}}},",
            c_text(p["name"].as_str().unwrap()),
            c_bytes(&key),
            key.len(),
            c_text(p["label"].as_str().unwrap()),
            c_bytes(&unhex(p["client"].as_str().unwrap())),
            c_bytes(&unhex(p["server"].as_str().unwrap())),
            c_bytes(&unhex(p["hex"].as_str().unwrap())),
        )
        .unwrap();
    }
    writeln!(
        h,
        "}};\nstatic const size_t GR_PROOF_COUNT = {};",
        proofs.len()
    )
    .unwrap();
    h
}

/// Compiles `source` into `dir`, linking guatiao when `with_guatiao`.
fn compile(
    source: &str,
    out_name: &str,
    with_guatiao: Option<&Path>,
    include: &Path,
) -> Option<PathBuf> {
    let Some(cc) = procs::compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, cc and gcc; GUATIAO_TEST_CC names one)"
        );
        return None;
    };
    let exe = std::env::current_exe().unwrap();
    let dir = with_guatiao
        .and_then(Path::parent)
        .map_or_else(|| exe.parent().unwrap().to_path_buf(), Path::to_path_buf);
    let out = dir.join(format!("{out_name}{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new(&cc);
    cmd.args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(here().join("tests/c"))
        .arg("-I")
        .arg(include)
        .arg("-I")
        .arg(here().join("../guatiao/include"))
        .arg("-o")
        .arg(&out)
        .arg(here().join("tests/c").join(source));
    if let Some(lib) = with_guatiao {
        if cfg!(windows) {
            cmd.arg(lib.with_extension("dll.lib")).arg("-lws2_32");
        } else {
            cmd.arg(format!("-L{}", dir.display()))
                .arg("-lguatiao")
                .arg("-Wl,-rpath,$ORIGIN");
        }
    }
    let (built, said) = run(&mut cmd);
    assert!(built, "{source} did not compile:\n{said}");
    Some(out)
}

fn generated_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("guatiao_remote_vectors");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_c_codec_encodes_and_decodes_every_vector() {
    let dir = generated_dir();
    std::fs::write(dir.join("remote_vectors.h"), vectors_header()).unwrap();
    let Some(exe) = compile("remote_frames.c", "guatiao_remote_frames", None, &dir) else {
        return;
    };
    let (ok, stdout) = run(&mut Command::new(&exe));
    assert!(ok, "{stdout}");
    assert!(stdout.contains("frames match"), "{stdout}");
}

fn need_guatiao() -> Option<PathBuf> {
    match guatiao_library() {
        Some((_, lib)) => Some(lib),
        None => {
            println!(
                "skipped: the guatiao library is not beside this test binary; \
                 build the workspace first (`cargo build --workspace`)"
            );
            None
        }
    }
}

#[test]
fn a_c_client_calls_a_rust_server() {
    let Some(lib) = need_guatiao() else {
        return;
    };
    let Some(exe) = compile(
        "remote_peer.c",
        "guatiao_remote_peer",
        Some(&lib),
        &generated_dir(),
    ) else {
        return;
    };
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()));
    let listener = TcpListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.1").unwrap()]).unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = server.clone();
    std::thread::spawn(move || {
        if let Ok(Accepted::Connection(conn)) = listener.accept() {
            let _ = serving.serve(conn);
        }
    });
    let (ok, stdout) = run(Command::new(&exe).args(["client", &port.to_string(), KEY]));
    assert!(ok, "{stdout}");
    for line in [
        "the server proved the key",
        "greeting: hello, from C",
        "refused (7): nobody to greet",
        "heard: hello, C",
        "heard: twice",
        "all checks passed",
    ] {
        assert!(stdout.contains(line), "missing `{line}` in:\n{stdout}");
    }
}

/// The greeter kind's description, wire-encoded, for the C server.
fn greeter_description(dir: &Path) -> PathBuf {
    let doc = guatiao_serde::text::json::from_str(
        &std::fs::read_to_string(
            here().join("../../examples/greeter_kind/include/greeter_kind.kinds.json"),
        )
        .unwrap(),
        guatiao::Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .unwrap();
    let kinds = TryAsRef::<Map>::try_as_ref(&doc)
        .and_then(|m| m.get("kinds"))
        .and_then(TryAsRef::<List>::try_as_ref)
        .unwrap();
    let greeter = kinds
        .iter()
        .find(|k| {
            TryAsRef::<Map>::try_as_ref(*k)
                .and_then(|m| m.get("name"))
                .and_then(TryAsRef::<str>::try_as_ref)
                == Some("greeter")
        })
        .unwrap();
    let path = dir.join("greeter.wire");
    std::fs::write(&path, wire::to_bytes(greeter).unwrap()).unwrap();
    path
}

#[test]
fn a_rust_host_calls_a_c_server() {
    let Some(lib) = need_guatiao() else {
        return;
    };
    let Some(exe) = compile(
        "remote_peer.c",
        "guatiao_remote_peer_s",
        Some(&lib),
        &generated_dir(),
    ) else {
        return;
    };
    let description = greeter_description(&generated_dir());
    let (child, lines) = start(Command::new(&exe).args(["server", KEY]).arg(&description));
    let wait = core::time::Duration::from_secs(10);
    let first = lines.recv_timeout(wait).expect("the port first");
    let port = first.strip_prefix("port ").expect("the port first");

    let source = RemoteSource::new(
        Box::new(TcpConnect::new(&format!("127.0.0.1:{port}"))),
        LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()),
    );
    let registry = common::host(&source);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let answer = greeter.greet("ana").unwrap();
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ana, from C")
    );
    let e = greeter.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet, says C");
    // The C server fills `greet` alone: `shout` is the kind's own default,
    // run here through the remote `greet`.
    assert_eq!(greeter.shout("bo"), "HELLO, BO, FROM C");
    drop(greeter);
    drop(registry);
    source.disconnect();
    let (ok, rest) = finish(child, &lines);
    assert!(ok, "{rest}");
    let served = rest.lines().last().unwrap_or_default();
    assert_eq!(served, "served 3 calls");
}
