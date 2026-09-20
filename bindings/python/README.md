# guatiao (Python)

ctypes bindings over the `guatiao` C ABI: read and build a value tree,
load a library into a registry, call a provider's kind table, and use
`guatiao-serde`/`guatiao-form` when those libraries are present. See
`src/guatiao/AGENTS.md` for the API.

## Install

```bash
pip install -e ".[dev]"
```

The native libraries (`guatiao.dll`/`libguatiao.so`/`libguatiao.dylib`
and, optionally, the `guatiao_serde`/`guatiao_form` siblings) are found at
import time, not bundled by this package yet — see `GUATIAO_LIBRARY` in
`src/guatiao/AGENTS.md`.

## Test

```bash
pytest tests -rs
```

Tests that need a native library skip cleanly when it is absent.
