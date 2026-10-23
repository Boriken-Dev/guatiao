# Greeters written in C++

The same kinds the Rust, C, Python, Go and Fortran greeters serve
(`examples/greeter_kind`, here `greeter_kind/cpp/greeter_kind.hpp`: the
kinds declared once over `greeter_kind.h`), offered by a library any host
calls, whatever language wrote it:

- `cpp_greeter_hello` is its one instance and serves two kinds: the
  greeter, every slot of it, and the counter;
- `cpp_greeter_shouter` is built from a configuration (`{"prefix": text}`,
  its schema `shouter::schema()`), one instance per configuration.

Each is an ordinary class whose methods are the slots
(`src/cpp_greeter.hpp`); a method that does not fit its slot does not
compile. `GUATIAO_LIBRARY(cpp_greeter::library, ...)` exports
`guatiao_library_entry` and the declaration a scan reads before it maps
the file, `guatiao_declares`.

## As a library file a scan finds

```bash
cargo build --workspace --all-features
GUATIAO_LIBRARY=target/debug cmake -S examples/cpp_greeter -B build/cpp_greeter
cmake --build build/cpp_greeter      # cpp_greeter.dll, libcpp_greeter.so, .dylib
```

Any host then finds it by its declared kind. The Rust example host:

```bash
GREETER_HOST_PATH=build/cpp_greeter cargo run -p greeter_host
```

and the Python one, `python examples/python_host/host.py build/cpp_greeter`.
The library links the guatiao library the CMake package found; on Windows
a host finds `guatiao.dll` as it finds any DLL's dependency (beside the
host's executable, on `PATH`, or already loaded). Unlike a library whose
language keeps a runtime of its own, a C++ library agrees to be unmapped
once nothing it served is alive.
