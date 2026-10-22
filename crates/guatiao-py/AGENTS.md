# guatiao-py — shipped API header

A guatiao source serving libraries written in Python. A `cdylib` and
nothing else: no Rust API, nothing links it. A host adds the file with
`add_source_file` and calls `refresh()`; the libraries it serves are
Python modules written with the Python binding's `guatiao.provide`
(`bindings/python/guatiao/src/guatiao/AGENTS.md`, "Providers written in Python").

Depends on `guatiao`, `guatiao-serde` (`toml`, for manifests) and `pyo3`
0.30 with `abi3-py39`. Not published (`publish = false`); a workspace of
its own, so no other build needs a Python to link against.

## The C surface

```c
guatiao_status guatiao_source(const guatiao_host_info *host,
                              const guatiao_value *config,      /* a map, or null */
                              const guatiao_source_info **out,  /* kept for the process */
                              guatiao_provider_error *err);
```

As `guatiao.h` declares the entry point of any source file. The file
declares `source=python`, so `scan_dir` reports it as `Skipped::Source`
and never maps it as a library. The answered table is leaked for the
process, as is the interpreter; adding the file twice adds two sources
over the one interpreter.

## Configuration

| key | holds | default |
| --- | --- | --- |
| `dirs` | directories whose `*.guatiao-py.toml` files, and those of their immediate subdirectories, are libraries | none |
| `manifests` | manifest files, listed as well | none |
| `paths` | put first on `sys.path` once, when the source is added: where `guatiao` and shared kind modules are | none |

A key holding anything but a list of text is `GUATIAO_ERR_BAD_VALUE`; a
configuration that is not a map, `GUATIAO_ERR_WRONG_KIND`; an
interpreter that cannot start, `GUATIAO_ERR_UNSUPPORTED`.

The binding inside loads the guatiao functions this file exports, unless
`GUATIAO_LIBRARY` is set or the host is Python and its binding already
loaded a library: one copy of the C surface either way.

## A manifest

```toml
module = "pkg.greeters"             # required: imported by this name; the library's id
kinds = ["greeter", "counter"]      # required: exactly the kinds its providers serve
paths = ["."]                       # put on sys.path while open, relative to this file
version = "1.0.0"                   # optional
```

Without `version`, the module's source is found on `paths` (without
importing it) and its `__version__` read as a string literal with `ast`;
anything else is a refusal asking for `version` in the manifest.

## What each slot does

- **`list`**: every manifest, in path order, each once. A candidate's id
  is its `module`, its stamp the manifest's content with the newest
  modification time of a `.py` file under its `paths` (`__pycache__`
  skipped; at most 10 000 files, 8 levels). Imports nothing. A manifest
  that does not parse is listed under its path, so opening it reports
  why.
- **`open`**: takes the GIL, puts the manifest's `paths` on `sys.path`,
  drops any module already imported from under them, imports `module`
  afresh, builds its library with `guatiao.provide.library_of`, and
  answers its descriptor. Refused (`GUATIAO_ERR_BAD_VALUE`, the reason
  in `err`): a module whose version or kinds differ from the manifest; a
  module whose top-level package is already imported from elsewhere
  (both paths named); any exception the import raises, as
  `TypeName: message`. An id no longer listed is `GUATIAO_ERR_NOT_FOUND`.
- **`close`**: lets the library's own instances go (`Library.close`),
  drops from `sys.modules` exactly the modules that open imported (never
  a newer build's), and takes its paths off `sys.path` once no open
  library uses them. The registry calls it when nothing holds the
  library, so an old build stays open for an instance built from it.

## Where it runs

pyo3's embedding links one `libpython` at build:

- **Windows**: the DLL imports `python3.dll` and nothing versioned; one
  build serves every CPython from 3.9 on. `python3.dll` is found on the
  DLL search path: beside `python.exe` for a Python host, through `PATH`
  for any other.
- **Linux**: the build has `NEEDED libpython3.X.so.1.0`, the minor of the
  Python it was built with (`PYO3_PYTHON`), abi3 or not; build one per
  minor, against the host's own when the host is Python.
- **macOS**: links that Python's framework
  (`/Library/Frameworks/Python.framework/Versions/3.X/Python`) or
  `libpython3.X.dylib` likewise; one build per minor.

A host that is not Python must find that library: a system Python's is
where the loader looks; another (pyenv, a CI toolchain cache) needs its
`lib` directory on `LD_LIBRARY_PATH` (Linux) or
`DYLD_FALLBACK_LIBRARY_PATH` (macOS).

On Linux and macOS a host maps the bridge, and libpython with it,
`RTLD_LOCAL`, where an extension module (`_ctypes`, any compiled package)
cannot find the interpreter's symbols. `guatiao_source` reopens the
libpython already mapped with `RTLD_GLOBAL | RTLD_NOLOAD` before starting
the interpreter; a Python host's own symbols still come first.

## Gotchas

- **One interpreter, shared modules.** Every library imports into the
  same `sys.modules`: two libraries needing different versions of one
  package need separate processes. A compiled extension module stays
  loaded after close; only `.py` code is imported afresh.
- **The interpreter is never finalised**: CPython cannot be initialised
  again reliably, so the source is added once and kept.
- **`paths` is how a non-Python host finds `guatiao`** and the kind
  modules, unless they are installed in the interpreter's own
  `site-packages`; a virtual environment's packages are not seen by an
  embedded interpreter unless its `site-packages` is on `paths`.
- **Every call into Python takes the GIL**; calls are serialised by it.
