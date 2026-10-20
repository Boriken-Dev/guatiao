// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example in `README.md`, compiled and run — and checked to still BE
//! the example in `README.md`.
//!
//! A README snippet that does not compile is worse than no snippet: it is
//! the first thing a reader tries, and when it fails they conclude the
//! crate is broken rather than the documentation. Rust checks doctests
//! inside `src/`, but nothing checks a fenced block in a `.md` file at
//! the crate root, so [`the_readme_example_compiles_and_holds`] is that
//! check.
//!
//! **Compiling a copy is not the same as checking the README, and this
//! file used to claim it was.** Its header said the body below was "kept
//! byte-identical to the README body so a change to one that is not made
//! to the other shows up as a failure here", and that was false: a
//! `#[test]` compiles its *own* copy of the code, so it can only catch a
//! snippet that stops compiling, never one that stops matching. The two
//! did in fact diverge — the README said `use guatiao_map::Map;`, a
//! crate that does not exist, while this file said `guatiao` and
//! passed — so a reader following the README verbatim got
//! `error[E0433]` from the first line of it.
//!
//! [`the_readme_example_is_the_readme_example`] is the check that claim
//! needed. It reads `README.md` at compile time with `include_str!`,
//! reads *this file* the same way, and compares the two texts. Keep the
//! marker comments below where they are: they are the second half of it.

#[test]
fn the_readme_example_compiles_and_holds() -> Result<(), Box<dyn core::error::Error>> {
    // README-EXAMPLE-BEGIN
    use guatiao::Map;

    let mut options = Map::new();
    options.set("compression", 6);

    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);
    map.set("tls", true);
    map.set("options", options);

    // Get the value, then convert it: no per-kind getters on a map, and no
    // per-kind readers on a value. `required` is the step from a lookup to a
    // value and it names the key it did not find; the rest is `TryInto`.
    let host: &str = map.required("host")?.try_into()?;
    let port: u16 = map.required("port")?.try_into()?;
    let options: &Map = map.required("options")?.try_into()?;
    let compression: i64 = options.required("compression")?.try_into()?;
    assert_eq!((host, port, compression), ("10.0.0.1", 5900, 6));

    // A key that is absent and a value of another kind each say which it was.
    assert!(map.required("missing").is_err());
    assert!(TryInto::<i64>::try_into(map.required("host")?).is_err());
    // README-EXAMPLE-END
    Ok(())
}

#[test]
fn the_readme_passing_example_compiles_and_holds() {
    // README-PASSING-BEGIN
    use guatiao::{Map, Value};

    let mut map = Map::new();
    map.set("host", "10.0.0.1");

    let node = Value::from(map); // a move: nothing copied
    let owned = node.into_local(); // already this binary's: handed back as it is
    let copy = owned.clone(); // a copy, into this binary's allocator
    assert_eq!(copy, owned);
    // README-PASSING-END
}

/// The README's "Passing data" block (its third `rust` block) and the
/// marked body above are the same text.
#[test]
fn the_readme_passing_example_is_the_readme_example() {
    assert_eq!(
        rust_block(include_str!("../README.md"), 2),
        marked_body(include_str!("readme_example.rs"), "README-PASSING"),
        "README.md's passing example and this file's marked body have diverged."
    );
}

/// The README's fenced `rust` block and the marked body above are the
/// same text.
///
/// Both sides go through exactly one transformation, and each is named
/// where it happens, because a normalising comparison that grows
/// transformations is how this kind of check stops checking: the README
/// loses its doctest-hidden `#` lines, and the test body loses the one
/// level of indentation it has for being inside a `fn`. Nothing else is
/// stripped, trimmed or reordered, so any other divergence — a renamed
/// method, a changed literal, a moved comment — fails here.
#[test]
fn the_readme_example_is_the_readme_example() {
    let from_readme = rust_block(include_str!("../README.md"), 0);
    let from_test = marked_body(include_str!("readme_example.rs"), "README-EXAMPLE");

    assert_eq!(
        from_readme, from_test,
        "README.md's example and this file's marked body have diverged. \
         Change both, or neither."
    );
}

/// The `index`-th ```` ```rust ```` fenced block of `md`, minus
/// doctest-hidden lines.
///
/// A line that is `#` alone or starts with `# ` is hidden: rustdoc strips
/// it before compiling and never renders it, so a reader copying the
/// block out of the README does not get it and the test body has no
/// counterpart for it. That is the only line this drops.
///
/// `str::lines` splits on `\n` *and* `\r\n`, so a checkout that has not
/// been normalised to LF cannot fail this test on line endings alone;
/// that is not the difference it is looking for.
fn rust_block(md: &str, index: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    let mut found = 0;

    for line in md.lines() {
        if !inside {
            if line.trim_end() == "```rust" {
                inside = found == index;
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

    panic!(
        "README.md has no closed ```rust block number {index}; if an example \
         moved, it moved out from under the check."
    );
}

/// The lines of `src` between the `name` marker comments, dedented one
/// level.
///
/// The four-space strip is the only transformation, and it is exactly the
/// indentation the body carries for being inside a `fn` — the README
/// block is at column zero. `strip_prefix` rather than `trim_start` on
/// purpose: `trim_start` would also flatten the *relative* indentation of
/// any nested line, which is a difference the README would show and this
/// test would then hide.
fn marked_body(src: &str, name: &str) -> String {
    let begin = format!("// {name}-BEGIN");
    let end = format!("// {name}-END");
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;

    for line in src.lines() {
        let trimmed = line.trim_start();
        if !inside {
            inside = trimmed == begin;
            continue;
        }
        if trimmed == end {
            return out.join("\n");
        }
        out.push(line.strip_prefix("    ").unwrap_or(line));
    }

    panic!("readme_example.rs has lost one of its {name} markers");
}
