# guatiao (Python) — API header

ctypes bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the wheel, so it names no repo path.

## Finding the native library

Importing `guatiao` touches no native library. A native call is resolved
lazily, on first use, in this order:

1. `GUATIAO_LIBRARY` — a file, or a directory holding
   `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib`.
2. `ctypes.util.find_library("guatiao")`.
3. This package's own `_native/` directory (empty until a build step
   populates it).

Not found raises `guatiao.LibraryNotFound` naming the three places it
looked. `guatiao_serde` and `guatiao_form` resolve the same way,
independently, only when `guatiao.serde` / `guatiao.form` is first used.

## Threading

A `Registry` is used from one thread at a time, like the Rust and C
handles it wraps. A `Value` is not shared across threads without the
caller's own lock.
