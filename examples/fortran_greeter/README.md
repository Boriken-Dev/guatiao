# Greeters written in Fortran

The same kinds the Rust, C, Python and Go greeters serve
(`examples/greeter_kind`, here `greeter_kind/fortran`: the kinds as
abstract types, written once from `greeter_kind.h`), offered by a library
any host calls, whatever language wrote it:

- `fortran_greeter_hello` is its one instance and serves two kinds: the
  greeter, every optional slot of it declared where it is registered, and
  the counter, through a second object sharing its state;
- `fortran_greeter_shouter` is built from a configuration
  (`{"prefix": text}`, its schema described by `shouter_config`), one
  instance per configuration.

The library is named and versioned as its `fpm.toml` states, and its own
test checks the two agree. A host in the same program registers it with
`guatiao_register_library(reg, fortran_greeter_library())`
(`examples/fortran_host` does).

## As a library file a scan finds

`src/fortran_greeter_entry.f90` exports `guatiao_library_entry` and the
declaration a scan reads before it maps the file, `guatiao_declares`, both
written in Fortran; on Windows `library.def` names them for the linker.
CMake builds the file, the binding and the kinds linked in:

```bash
cmake -S examples/fortran_greeter -B build/fortran_greeter
cmake --build build/fortran_greeter      # fortran_greeter.dll, libfortran_greeter.so, .dylib
```

Any host then finds it by its declared kind. The Rust example host:

```bash
GREETER_HOST_PATH=build/fortran_greeter GUATIAO_LIBRARY=target/debug cargo run -p greeter_host
```

and the Python one, `python examples/python_host/host.py
build/fortran_greeter`. The library opens its own guatiao as any program
using the binding does: `GUATIAO_LIBRARY`, or the platform's search. A
Fortran library is never unmapped: its `unload` refuses.

## Tests

```bash
cd examples/fortran_greeter
fpm test --compiler gfortran    # or flang, ifx
```
