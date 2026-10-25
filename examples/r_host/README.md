# A plugin host in R

`host.R` loads greeter libraries written in any language and calls each
one through the `greeter` kind's table: a map back, a map lent and changed
in place, a map handed over, an object handed in and one handed back (a
conversation), an object lent for one call (a listener). A provider that
builds instances from a configuration is given one its schema refuses,
then one it accepts. The greeters written in R
([`../r_provider`](../r_provider/)) are registered beside the ones it
loads, and the greeters written in Python are served by the guatiao-source-py
bridge when one is named. It closes everything it opened and prints what
it did.

The kinds are declared in
[`../greeter_kind/r/greeterkind`](../greeter_kind/r/greeterkind/R/kinds.R),
from the kind's C header
([`../greeter_kind/include/greeter_kind.h`](../greeter_kind/include/greeter_kind.h)).

## Run it

From the repository root, with R 4.2 or later, after building the
workspace, install the binding and the two example packages, then run the
host:

```bash
cargo build --workspace --all-features
R CMD INSTALL bindings/r/guatiao
R CMD INSTALL examples/greeter_kind/r/greeterkind
R CMD INSTALL examples/r_provider/rgreeter
export GUATIAO_LIBRARY=target/debug
Rscript examples/r_host/host.R target/debug
```

`target/debug` holds the Rust greeters (`derived_greeter`, written with
the derive macros) and `hello_library`, whose table is not this kind's,
which the host reports and passes over.

To add the greeter written in C, build it into a directory of its own and
name that directory too:

```bash
mkdir -p c-greeter
cc -shared -fPIC -std=c11 -I crates/guatiao/include -I examples/greeter_kind/include \
   -o c-greeter/libc_greeter.so crates/guatiao/tests/c_consumer/greeter_in_c.c
Rscript examples/r_host/host.R target/debug c-greeter
```

On Windows the output is `c-greeter/c_greeter.dll`; on macOS,
`c-greeter/libc_greeter.dylib`. The library must be built for the R
process's architecture.

The greeter written in Go is a library file too
([`../go_provider/README.md`](../go_provider/README.md) builds it); name
its directory the same way. To add the greeters written in Python,
through the `guatiao-source-py` bridge (built as
[`../python_provider/README.md`](../python_provider/README.md) says), name
the bridge, where its manifests are, and what they import:

```bash
Rscript examples/r_host/host.R target/debug \
  --python-source crates/guatiao-source-py/target/debug/libguatiao_source_py.so \
  --python-dir examples/python_provider \
  --python-path bindings/python/guatiao/src --python-path examples/greeter_kind/python
```

The binding's tests (`bindings/r/guatiao`, `R CMD check`) run this host, call
every greeter (Rust, C, Python, Go and R) through every slot, have the
Rust caller in `greeter_caller` call each of them through the kind's Rust
proxy, and call the R greeter from threads R did not start, which it
refuses.

## What it shows

- **One description of the kind serves every language.** The Rust, C,
  Go, Python and R greeters are called through the same `GreeterRemote`;
  a table whose `FLOOR_HASH` is not the kind's is refused before anything
  in it runs (`guatiao_floor_mismatch`).
- **A table may be older than the kind.** The C greeter fills four of six
  slots; `has_slot(g, "slot")` asks before calling, and calling a missing
  slot answers `Unsupported`.
- **A refusal crosses with its words**: `greet(g, "")` signals a
  `guatiao_error` `BadValue` carrying the provider's message.
- **Objects cross both ways.** The listener is an S4 object with a method
  of `heard` and of `close`; the conversation a greeter hands back is
  closed, which calls its `destroy` slot, and the greeter lets the
  listener go.
- **Unmapped when the library agrees.** A Rust library counts what it
  handed out and agrees once nothing is alive; the C greeter has no say,
  a Go library never unmaps, and the library written in R is linked rather
  than mapped, so each is retired instead.
