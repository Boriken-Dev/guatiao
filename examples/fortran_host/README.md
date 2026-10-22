# A plugin host written in Fortran

`app/fortran_host.f90` scans the directories it is given for libraries
declaring the `greeter` kind, whatever language wrote them, registers the
greeters written in Fortran (`examples/fortran_greeter`) beside them, and
calls every greeter through the kind's table: a map back, a map lent and
changed in place, a map handed over, an object lent for one call, an
object handed in and one handed back. A provider building instances is
given a configuration its schema accepts.

```bash
cargo build --workspace --all-features
cd examples/fortran_host
GUATIAO_LIBRARY=../../target/debug fpm run --compiler gfortran -- ../../target/debug
```

With `--python-source BRIDGE --python-dir DIR` the guatiao-py bridge
serves the Python greeters described under DIR too.

## Tests

`test/test_greeters.f90` calls the greeters written in Rust, C, Python, Go
and Fortran through one description of the kind, every slot of each, and
has the Rust caller (`greeter_caller`) call each of them through the
kind's Rust proxy; it then calls the Fortran greeter from eight threads it
did not start. The Rust greeters come from the workspace's build, the C
one is built with the first C compiler found (`CC`, `clang`, `cc`,
`gcc`), the Go one from the directory `GUATIAO_GO_LIBRARY` names (built
with `go build -buildmode=c-shared ./go_provider/cshared` in `examples`),
the Python ones through the bridge `GUATIAO_PY_BRIDGE` names; each is
loaded by one refresh of the sources the registry keeps. A greeter that
is not there is a stated skip, and a failure with `GUATIAO_REQUIRE_ALL=1`.

```bash
GUATIAO_LIBRARY=../../target/debug fpm test --compiler gfortran
```
