# guatiao-source-py — shipped API header

A guatiao source serving libraries written in Python. A `cdylib` and
nothing else: no Rust API, nothing links it. A host adds the file with
`add_source_file` and calls `refresh()`; the libraries it serves are
Python modules written with the Python binding's `guatiao.provide`
(`bindings/python/guatiao/src/guatiao/AGENTS.md`, "Providers written in Python").

Depends on `guatiao`, `guatiao-bridge` (manifests, stamps, refresh and
the source table; CPython is its engine, `ThreadModel::Locked`, building
its own kind tables) and `pyo3` 0.30 with `abi3-py39`. Not published
(`publish = false`); a workspace of its own, so no other build needs a
Python to link against.

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
| `dirs` | directories whose `*.guatiao` files of type `python`, and those of their immediate subdirectories, are libraries | none |
| `manifests` | manifest files, listed as well | none |
| `paths` | put first on `sys.path` once, when the source is added: where `guatiao` and shared kind modules are | none |
| `isolation` | how much the interpreter this file starts takes from outside the configuration: `shipped`, `inherit` or `isolated` (below) | `shipped` |
| `home` | the Python installation that interpreter runs from (`sys.base_prefix`): an existing directory, relative to this file's; not taken under `inherit`. It does not decide where `python3.dll` comes from: that is mapped before this file runs (`import_dirs`) | CPython's own answer; under `isolated`, libpython's directory (Windows) or the one above its `lib` |
| `import_dirs` | the registry's, not this file's: on Windows, where `python3.dll` (and the `python3XX.dll` it forwards to) is found when it is not beside the file or the host, as `[sys.base_prefix]`; relative to this file's directory | none |

A list key holding anything but a list of text, `isolation` naming no
mode, or `home` under `inherit` or naming no directory is
`GUATIAO_ERR_BAD_VALUE`; a configuration that is not a map,
`GUATIAO_ERR_WRONG_KIND`; an interpreter that cannot start,
`GUATIAO_ERR_UNSUPPORTED`.

## Isolation

The interpreter is started from CPython's isolated configuration unless
`isolation` is `inherit`, so a host's environment does not choose the
code a library runs:

| `isolation` | `sys.path` and `site` read | ignored |
| --- | --- | --- |
| `shipped` (the default) | what is shipped with the program: a `._pth` or a `pyvenv.cfg` beside the host's executable, and on Windows the executable's own directory, which CPython puts on `sys.path` (a module or `sitecustomize.py` there is found); `home`; the installation's site-packages | `PYTHONPATH`, `PYTHONHOME` and every other `PYTHON*` variable, user site-packages, a program found on `PATH` |
| `inherit` | the environment as well, as a `python` started there would | nothing: the weaker setting |
| `isolated` | exactly the standard library of `home` (else of libpython's own installation) and `paths`: the module search path is set to it, `site` is not run, and from 3.14 CPython is told the executable is the installation's own | everything beside the host's executable, the installation's site-packages, the environment |

`inherit` and the import search `path` (the registry's) are the weaker
settings. The source tells its host when the environment reaches the
interpreter, as the refresh's notes (every binding's `refresh` hands them
over; `docs/encodings.md`, "A source's note"), the first refresh after it
was added:

| subject | when | detail |
| --- | --- | --- |
| `isolation` | this file started the interpreter under `inherit`, or joined one whose `sys.flags.ignore_environment` is 0 | `{"effective": "inherit", "configured": "<the configuration's isolation, shipped when unnamed>", "joined": bool}` |
| `home` | `home` was named and the interpreter was joined, so it is not applied | `{"home": "<as named, made absolute>"}` |

The import search and `import_dirs` are the registry's, set by the host
in its own call: no note tells them back.

`home` and `isolation` apply when this file starts the interpreter: the
first source added decides, and a later one asking for another
`isolation` or `home` is refused (`GUATIAO_ERR_BAD_VALUE`, naming both).
A relative `home` is this file's directory's. A start that failed leaves
CPython where it cannot start again: every later source is refused with
the first failure's words (`GUATIAO_ERR_UNSUPPORTED`). An interpreter
already running in the process (the host is Python, or another copy of
this file started it) is used as it is: `isolated` is refused unless its
`sys.flags.isolated` is set, and `shipped` named in the configuration
unless `sys.flags.ignore_environment` is (`GUATIAO_ERR_BAD_VALUE`); the
default and `inherit` join it, and `home` is not applied.

What CPython reads whatever it is told is refused rather than read
(`GUATIAO_ERR_UNSUPPORTED`, naming it): before 3.14 on Windows `site`
reads a `pyvenv.cfg` beside the executable or one directory up, so
`isolated` refuses while one is there; on macOS `site` takes `sys.prefix`
from `__PYVENV_LAUNCHER__`, so `shipped` and `isolated` refuse while it
is set (by reading CPython's `site`, not run on macOS). A Python older
than 3.9 is refused. 3.9 and 3.10 on Windows find no standard library
with no `home`, from either configuration, so `shipped` and `inherit` (with
no `PYTHONHOME`) give them libpython's directory. From 3.14 every mode is
started through `PyInitConfig` and a failure is a refusal; before, `inherit`
is `Py_InitializeEx(0)`, which ends the process where CPython cannot
start. Elsewhere than Windows `inherit` names the program `python3` and
CPython looks it up on `PATH`, so a virtual environment whose `bin` is
there decides `sys.prefix`; `shipped` and `isolated` name the program
themselves. Measured on Windows with 3.9, 3.11 and 3.14 and on Linux with
3.14 (`tests/isolation.rs`).

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
  build serves every CPython from 3.9 on. A Python host has it loaded
  already; any other names the Python installation in the
  configuration's `import_dirs` (the registry maps `python3.dll` and the
  `python3XX.dll` it forwards to from there, under its default import
  search), or puts it on `PATH` and sets the registry's import search to
  `path`. The file imports `python3.dll` by name, so the registry maps it
  before any of this file's code runs: `home` cannot choose it, and a
  configuration naming only `home` is refused under the default search.
  The installation named in `import_dirs` is the interpreter's `home`
  too when `home` is not given; name `home` as well only for a standard
  library kept elsewhere, of the same version (another version's is
  refused, in CPython's words).
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
