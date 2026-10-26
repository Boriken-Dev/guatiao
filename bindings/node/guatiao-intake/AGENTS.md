# guatiao-intake (Node.js) — API header

Taking a value in from a person, over the `guatiao_intake` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the npm package, so it names no
repository path.

ESM only, typed, Node 22 or later, 64-bit platforms and 32-bit ARM. One dependency,
`guatiao`, at the same version: its `Value`, `Ref`, errors and koffi
(`guatiao/ffi`). This package ships no native code and loads no koffi of
its own.

## Finding the native library

Importing touches nothing native. The first call that needs it opens the
library, in this order:

1. `useLibrary(pathOrUrl)`, when called before that first call: a file
   exporting its functions, whatever it is called, or a directory.
   Naming another file once one is loaded throws; naming the same one
   does nothing.
2. `GUATIAO_INTAKE_LIBRARY`: a file, or a directory holding
   `guatiao_intake.dll` / `libguatiao_intake.so` /
   `libguatiao_intake.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_intake_check` (an application's own library carrying several
   surfaces), else the library beside it.
4. The platform's own library search, by file name.

Not found throws `LibraryNotFound` (of `guatiao`) naming each route.
`library()` answers the loaded `Surface`. The library carries the core
too, so one file can serve both packages.

## Forms, paths and the flat store

```ts
import * as intake from 'guatiao-intake';

intake.check(schema, form);              // null, or the error map
intake.layout(schema, form);             // [{ section, fields: [...] }, ...]
intake.isVisible(schema, form, 'key', values);
intake.at(value, 'agent[1].name');       // a borrowed Ref, or undefined
intake.forSchema(schema);                // the form a schema implies (a Value to close)
intake.formFor(form, schema, 'path');    // a Value to close, or undefined
intake.resolve(schema, 'auth.password'); // borrowed Ref onto the field, or undefined
intake.flatKeys(schema, 'auth');         // string[]; WrongKind for an unknown key
intake.flatten(schema, 'auth', value);   // Record<string, string>
intake.unflatten(schema, 'auth', store); // a Value to close, or undefined
```

- `check`: `null` when the form fits; otherwise the error map (`kind`,
  `at`, and whichever of `path`, `id`, `field`, `expected`, `message`
  apply).
- `at(value, path)`: a dot is a member, a bracket a list position or a
  map key, decided by what it applies to; `undefined` for a path that
  names nothing or is no path.
- `forSchema`: a section per distinct `x-section`, ids only, and each
  member's own form. `formFor`: the form for the field at `path`,
  assigned or implied; `undefined` when there is none.
- The flat store keeps one entry per scalar leaf (a tagged field's
  discriminant under its key and its payload under `key.member`, a list
  as `agent[0].name`, an open map as `env[PATH]`). `unflatten` answers
  `undefined` only for "nothing stored"; a store that is not strings to
  strings (a plain object, a `Map` or a `Ref` onto a map) is a
  `TypeError`.

Every `schema`, `form`, `value` and `values` is a `Ref` of `guatiao`
(built by any guatiao library) or plain data, converted for the call.
What `at` and `resolve` answer borrows from what it came from: read-only
(every write is a `TypeError`), found again on every access, and an error
once the source is closed; `.clone()` copies it out. A `Value` answered
is built in the library's own allocator; close it. A refusal throws the
status's `GuatiaoError`.

## A provider's form

A provider's form travels in its `meta`, under `providerFormKey`
(`'form'`), beside the configuration schema it is shown with:

```ts
using meta = reg.providerMeta('acme_dialer')!;   // undefined when it declares no meta
const form = meta.asMap().get(intake.providerFormKey)!;
const schema = reg.providerConfig('acme_dialer')!;
intake.check(schema, form);                      // null: it fits
intake.layout(schema, form);
```

## Threads

Everything runs on Node's main thread, as in `guatiao`.
