# A plugin host written in C++

`src/cpp_host.cpp` watches the directories it is given for libraries
declaring the `greeter` kind, whatever language wrote them, loads them with
one `refresh()`, and calls every greeter through the kind's table: a map
back, a map lent and changed in place, a map handed over, an object lent
for one call, an object handed in and one handed back. A provider building
instances is given a configuration its schema accepts. Then every library
is unloaded where it agrees, and retired where it does not.

```bash
cargo build --workspace --all-features
GUATIAO_LIBRARY=target/debug cmake -S examples/cpp_host -B build/cpp_host
cmake --build build/cpp_host
build/cpp_host/cpp_host target/debug build/cpp_host/greeters/cpp
```

With `--python-source BRIDGE --python-dir DIR` the guatiao-py bridge
serves the Python greeters described under DIR too.

## Tests

`test/test_greeters.cpp` calls the greeters written in Rust, C, Python, Go
and C++ through one description of the kind, every slot of each, and has
the Rust caller (`greeter_caller`) call each of them through the kind's
Rust proxy; then a configured provider builds instances, eight threads the
host did not start call the C++ greeter, and the C++ library is unloaded
once nothing it served is alive. Every greeter comes through a source the
registry keeps, loaded by one refresh:

- the Rust greeters from the workspace's build (`GUATIAO_RUST_GREETERS`,
  by default the directory the guatiao library was found in);
- the C greeter and the C++ greeter, built by this project;
- the Go greeter from the directory `GUATIAO_GO_LIBRARY` names (built with
  `go build -buildmode=c-shared ./go_provider/cshared` in `examples`);
- the Python greeters through the bridge `GUATIAO_PY_BRIDGE` names.

A greeter that is not there is a stated skip, and a failure with
`GUATIAO_REQUIRE_ALL=1`.

`test/test_wide.cpp` crosses the 64-bit limits of the `wide` kind (2^53 + 1,
INT64_MIN and INT64_MAX, 2^53 + 1, INT64_MAX + 1 and UINT64_MAX) through a
provider written in Rust, one in C and one in C++ registered by the test,
and has the Rust caller call each of them.

```bash
ctest --test-dir build/cpp_host --output-on-failure
```
