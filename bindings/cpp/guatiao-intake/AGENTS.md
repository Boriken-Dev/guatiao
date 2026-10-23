# guatiao-intake (C++) — API header

Taking a guatiao value in from a person, over the `guatiao_intake` C
library: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no
repository path.

Header-only, C++17 or later, over `<guatiao_intake.h>` and the
`guatiao_intake` shared library, built on the `guatiao` C++ package for
the value model and its errors. `#include <guatiao/intake.hpp>`;
namespace `guatiao::intake`.

## The package and the library

`find_package(guatiao-intake CONFIG)` (`guatiao-intake_DIR` naming this
package's directory), `add_subdirectory` or `FetchContent`, then
`target_link_libraries(app PRIVATE guatiao::intake)`. The library is
`GUATIAO_INTAKE_LIBRARY` (a file, or a directory holding
`guatiao_intake.dll` / `libguatiao_intake.so` / `.dylib`), then
`GUATIAO_LIBRARY` (a directory holding it; a file: the library beside it
when that file is the core's own, else the file itself), then a cargo
build from the Rust sources beside the package.

`<guatiao_intake.h>` defines a macro named `SEPARATOR`; include it after
anything of yours that uses that name.

## Forms

A form is a value: a map of `sections` (a list, in display order) and
`fields` (hints keyed by a field's path).

- `check(schema, form)`: BAD_VALUE when the form does not fit, with
  detail `{kind, at, path?, id?, field?, expected?, message}`; `kind` is
  `malformed`, `unknown_field`, `duplicate_section`, `condition_refused`
  or `cyclic_condition`.
- `layout(schema, form, alloc)`: a list of `{"section": map or null,
  "fields": [key, ...]}`, in order.
- `for_schema(schema, alloc)`: the form a schema implies, for when nobody
  wrote one.
- `form_for(form, schema, path, alloc)`: the form to show a field with,
  the assigned one or the one its schema implies; ABSENT when neither.
- `is_visible(schema, form, key, values)`: whether a field is shown given
  the values entered so far; NOT_FOUND for a key naming no field.
- `key_x_section`, `key_x_order`, `key_x_advanced`, `provider_form_key`:
  the keys this library reads.

## Paths and the flat store

- `at(value, path)`: the value at `agent[1].name` (a dot is a member, a
  bracket a position or a key, by what it applies to), a `view` borrowed
  from `value`; ABSENT for a path naming nothing or not a path.
- `resolve(schema, key)`: the field governing a flat key, borrowed from
  `schema`; ABSENT when none.
- `flat_keys(schema, key)`: the flat keys a field projects onto
  (`std::vector<std::string>`); WRONG_KIND for a key naming no field.
- `flatten(schema, key, value, alloc)`: a map of `key -> text`, one entry
  per scalar leaf; `unflatten(schema, key, flat, alloc)` the value back.
  WRONG_KIND when the shape does not fit the field, or nothing is stored
  under it.

Every call has a `try_` form answering `guatiao::result<T>`.
