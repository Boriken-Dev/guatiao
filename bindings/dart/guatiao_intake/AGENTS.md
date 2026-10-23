# guatiao_intake (Dart) — API header

Taking a value in from a person, over the `guatiao_intake` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.
Built on package:guatiao (`Value`, `Ref`, its errors,
`package:guatiao/ffi.dart`), at the same version. 64-bit native platforms,
and the web over the module the page loaded (below).

## Finding the native library

Importing touches nothing native. The first call that needs it opens the
library, through package:guatiao's `openSurface('guatiao_intake')`:

1. a library registered with package:guatiao's `useLibrary` for
   `'guatiao_intake'` (with no `forSurfaces`, any library exporting
   `guatiao_intake_check` is registered for it);
2. `GUATIAO_INTAKE_LIBRARY`: a file taken as it is, or a directory holding
   `guatiao_intake.dll` / `libguatiao_intake.so` /
   `libguatiao_intake.dylib`;
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_intake_check` (an application's own library carrying several
   surfaces), else the library beside it;
4. package:guatiao's `libraryDirectory`;
5. the platform's own search, by name.

Not found throws `LibraryNotFound` naming every route. `library()`
answers the opened `NativeLib`. The library carries the core too, so one
file can serve both packages.

## On the web

The same functions under dart2js and dart2wasm, through the one
WebAssembly module the page loaded with package:guatiao's `loadModule`:
every surface is called in that module, since a value lives in its
memory. The module must carry this surface (`guatiao_intake_check`): an
application's own crate built with it, or for forms
`guatiao_intake.wasm` itself. Without it every function throw `LibraryNotFound`.
`library()` answers the module (`WasmModule`), and every `alloc:` is an
`Address` (package:guatiao/wasm.dart) rather than a `Pointer`.

## Forms

```dart
import 'package:guatiao_intake/guatiao_intake.dart' as intake;

intake.check(schema, formDoc);          // null, or the error map
intake.layout(schema, formDoc);         // [{'section': ..., 'fields': [...]}, ...]
intake.isVisible(schema, formDoc, 'key', values);   // bool
intake.at(value, 'agent[1].name');      // a borrowed Ref, or null
intake.forSchema(schema);               // the form a schema implies (a Value to close)
intake.formFor(formDoc, schema, 'path');   // a Value to close, or null
```

- `Map<String, Object?>? check(Ref schema, Ref form, {alloc})`: `null`
  when `form` fits `schema`, else the error map (`kind`, `at`, and
  whichever of `path`, `id`, `field`, `expected`, `message` apply).
- `List<Map<String, Object?>> layout(Ref schema, Ref form, {alloc})`: the fields in
  sections, in order.
- `bool isVisible(Ref schema, Ref form, String key, Ref values)`.
- `Ref? at(Ref value, String path)`: a bracket is a position or a map key
  by what it is applied to; `null` for a path that names nothing or is
  not a path.
- `Value forSchema(Ref schema, {alloc})`: one section per distinct
  `x-section`, ids only, plus each member's own form.
- `Value? formFor(Ref form, Ref schema, String path, {alloc})`: the form
  for the field at `path`, assigned or implied.

## The flat store

A value kept as `key -> text`, one entry per scalar leaf (a tagged
field's discriminant under its key and its payload under `key.member`, a
list as `agent[0].name`, an open map as `env[PATH]`):

```dart
intake.resolve(schema, 'auth.password');  // borrowed Ref onto the field, or null
intake.flatKeys(schema, 'auth');          // List<String>; WrongKind for an unknown key
intake.flatten(schema, 'auth', value);    // Map<String, String>; WrongKind when it does not apply
intake.unflatten(schema, 'auth', store);  // Value to close, or null; store is a
                                          // Map<String, String> or a Ref onto one
```

An empty container flattens to nothing, and an entry the schema does not
declare is dropped by `unflatten`. `unflatten` answers `null` only for
"nothing stored": no such field, no entry under it, or a discriminant
that names no arm. A store that is not text to text throws
`ArgumentError`.

## Arguments and answers

Every argument is a `Ref` of package:guatiao (a `Value`, a borrowed view
included), built by any guatiao library. What `resolve` and `at` return
is a read-only view: it holds the tree it came from, is found again on
every access, and throws `StateError` once that is closed; any write
throws `StateError`, and `.clone()` copies it out. A `Value` answered is
built in `alloc` (default the library's own allocator); `close()` it. A
refusal throws the status's `GuatiaoException`.

## A provider's form

A provider's form travels in its `meta`, under `providerFormKey`
(`'form'`), beside the configuration schema it is shown with:

```dart
final meta = reg.providerMeta('acme_dialer');    // null when it declares no meta
try {
  final form = meta!.asMap()[intake.providerFormKey]!;
  final schema = reg.providerConfig('acme_dialer')!;
  intake.check(schema, form);                    // null: it fits
  intake.layout(schema, form);
} finally {
  meta?.close();
}
```

## Generated bindings

`lib/src/bindings.g.dart` is written by ffigen from `guatiao_intake.h`,
reusing package:guatiao's types through its symbol file, and is never
edited by hand.
