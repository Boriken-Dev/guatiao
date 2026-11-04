# A provider's form

A provider built from a configuration declares the schema of that
configuration: its fields, their types, what each accepts. A **form** says
how a person is shown it: which section a field sits in, what a section is
called, a placeholder, a unit, a field that appears only while another
holds a given value. The schema says what is valid; the form says how to
ask for it.

A provider ships its form in its descriptor, so the library that knows its
settings also says how to present them, and a host in any language reads
it back beside the schema. The form is data the library wrote, so the
host **checks it against the schema before showing it**, as it would any
other input.

## Writing one

The form derive lives in `guatiao-intake`; `#[provider(.., form = T)]`
needs `guatiao`'s `provider` feature:

```toml
[dependencies]
guatiao = { version = "=0.0.0-alpha.1", features = ["provider"] }
guatiao-intake = { version = "=0.0.0-alpha.1", features = ["derive"] }
```

The configured greeter in `examples/derived_greeter` is its own
configuration, its own schema and its own form:

```rust
#[derive(guatiao::Schema, guatiao::FromValue, Form, Provider)]
#[form(
    section(id = "words", label = "Words"),
    section(id = "volume", label = "Volume", help = "How loud a shout is")
)]
#[provider(Greeter, config, form = Shouter)]
pub struct Shouter {
    /// What every shout starts with.
    #[schema(section = "words")]
    #[form(placeholder = "hey")]
    prefix: String,
    /// Whether a shout ends in exclamation marks.
    #[schema(section = "volume")]
    #[map(default)]
    exclaim: bool,
    /// How many exclamation marks end a shout.
    #[schema(section = "volume")]
    #[map(default = 1)]
    #[form(unit = "marks", visible_when(field = "exclaim", equals = true))]
    marks: i64,
}
```

- `#[derive(Form)]` builds the form from the `#[form(..)]` attributes:
  the sections on the type, and `placeholder`, `unit` and
  `visible_when(field = "..", equals = ..)` (or `in(..)`) on a field.
- `#[schema(section = "..")]` places a field in a section. It is written
  on the schema, as `x-section`, because which group a field belongs to
  holds wherever the schema is shown; what a section is *called* is the
  form's.
- `form = Shouter` in `#[provider(..)]` writes `Shouter`'s form into the
  descriptor's `meta`, under the key `"form"` (`PROVIDER_FORM_KEY`). Any
  type with a form can be named there, not only the configuration itself.

What lands in `meta` is a plain value, the same in every language:

```json
{
  "form": {
    "sections": [
      { "id": "words", "title": "Words" },
      { "id": "volume", "title": "Volume", "description": "How loud a shout is" }
    ],
    "fields": {
      "prefix": { "placeholder": "hey" },
      "marks": {
        "unit": "marks",
        "visibleWhen": { "field": "exclaim", "equals": true }
      }
    }
  }
}
```

## Reading it back

A host does the same three things in every language: read the form out of
the provider's `meta`, read the configuration schema, and check one
against the other. `check` refuses a form whose shape is wrong, that names
a field the schema does not declare, that declares a section twice, or
whose conditions could never let a field be shown. Once it passes,
`layout` groups and orders the fields, and `is_visible` says whether a
field is shown for the values entered so far.

### Rust

```rust
use guatiao::schema::SchemaRef;
use guatiao::{Map, Value};
use guatiao_intake::{FormRef, PROVIDER_FORM_KEY, check, is_visible, layout};

let offer = registry
    .offer::<dyn Greeter>("derived_greeter_shouter")
    .ok_or("nothing is filed under that key")??;
let schema = offer.config_schema().and_then(SchemaRef::new);
let form = offer
    .meta()
    .and_then(|meta| meta.get(PROVIDER_FORM_KEY))
    .and_then(FormRef::new);

if let (Some(schema), Some(form)) = (schema, form) {
    // A form is data a library wrote: check it before showing it.
    check(schema, form)?;
    for group in layout(schema, form) {
        let title = group.section.map_or("", |s| s.label());
        let keys: Vec<&str> = group.fields.iter().map(|p| p.field.key()).collect();
        println!("{title}: {keys:?}");
    }
    let mut values = Map::new();
    values.set("exclaim", false);
    let shown = is_visible(schema, form, "marks", &Value::from(values))?;
    assert!(!shown, "`marks` waits for `exclaim`");
}
```

`Offer::meta` and `Offer::config_schema` borrow from the registry; each is
`None` for a provider that declares nothing.

### C

`guatiao_registry_provider_meta` writes a **copy** of the provider's
`meta` in the allocator you pass, which you free and may keep past the
library. It leaves `out` ABSENT for a provider that declares none, and
answers `GUATIAO_ERR_NOT_FOUND` for a key nothing answers to. The schema
is borrowed from the registry and never freed. The key is a macro in
`guatiao_intake.h`:

```c
#include "guatiao.h"
#include "guatiao_intake.h"

guatiao_value meta = {0};
guatiao_status st = guatiao_registry_provider_meta(
    reg, guatiao_cstr("derived_greeter_shouter"), &alloc, &meta);
const guatiao_value *form =
    guatiao_map_find(&meta, guatiao_cstr(GUATIAO_INTAKE_PROVIDER_FORM_KEY));
const guatiao_value *schema =
    guatiao_registry_provider_config(reg, guatiao_cstr("derived_greeter_shouter"));

if (form != NULL && schema != NULL) {
  guatiao_value why = {0};
  st = guatiao_intake_check(schema, form, &alloc, &why);  /* GUATIAO_OK: it fits */
  guatiao_value_free(&why);
  /* then guatiao_intake_layout and guatiao_intake_is_visible */
}
guatiao_value_free(&meta);
```

### Python

```python
import guatiao_intake as intake           # pip install guatiao-intake
from guatiao.registry import Registry
from guatiao.value import Value

with Registry("form-host", "1.0") as reg:
    reg.scan_dir(plugins, rules="kind=greeter")
    meta = reg.provider_meta("derived_greeter_shouter")   # None: it declares no meta
    if meta is not None:
        with meta:
            form = meta.as_map()[intake.PROVIDER_FORM_KEY]
            schema = reg.provider_config("derived_greeter_shouter")
            assert intake.check(schema, form) is None    # or the error, as a dict
            intake.layout(schema, form)
            with Value.from_python({"exclaim": False}) as values:
                intake.is_visible(schema, form, "marks", values)   # False
```

### Dart

```dart
import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;   // dart pub add guatiao_intake

final reg = Registry('form-host', '1.0')..scanDir(plugins, rules: 'kind=greeter');
final meta = reg.providerMeta('derived_greeter_shouter');   // null: no meta
try {
  final form = meta!.asMap()[intake.providerFormKey]!;
  final schema = reg.providerConfig('derived_greeter_shouter')!;
  intake.check(schema, form);                               // null: it fits
  intake.layout(schema, form);
} finally {
  meta?.close();
  reg.close();
}
```

### Node.js

```ts
import { Registry } from 'guatiao';
import * as intake from 'guatiao-intake';   // npm install guatiao-intake

using reg = new Registry('form-host', '1.0');
reg.scanDir(plugins, { rules: 'kind=greeter' });
using meta = reg.providerMeta('derived_greeter_shouter')!;   // undefined: no meta
const form = meta.asMap().get(intake.providerFormKey)!;
const schema = reg.providerConfig('derived_greeter_shouter')!;
intake.check(schema, form);                                  // null: it fits
intake.layout(schema, form);
intake.isVisible(schema, form, 'marks', { exclaim: false }); // false
```

### Go

```go
import (
	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"
)

meta, ok, err := reg.ProviderMeta("derived_greeter_shouter") // ok false: no meta
if err == nil && ok {
	defer meta.Close()
	form := meta.Key(intake.ProviderFormKey)
	schema, _, _ := reg.ProviderConfig("derived_greeter_shouter")
	why, _ := intake.Check(schema, form)                      // nil: it fits
	shown, _ := intake.IsVisible(schema, form, "marks", map[string]any{"exclaim": false})
}
```

### Julia

```julia
using Guatiao, GuatiaoIntake

meta = Guatiao.provider_meta(reg, "derived_greeter_shouter")   # nothing: no meta
form = meta[GuatiaoIntake.PROVIDER_FORM_KEY]
schema = Guatiao.provider_config(reg, "derived_greeter_shouter")
GuatiaoIntake.check(schema, form)                               # nothing: it fits
GuatiaoIntake.isvisible(schema, form, "marks", Dict("exclaim" => false))
close(meta)
```

### R

```r
meta <- reg$provider_meta("derived_greeter_shouter")   # NULL: no meta
form <- meta[[intake_provider_form_key]]
schema <- reg$provider_config("derived_greeter_shouter")
intake_check(schema, form)                             # NULL: it fits
intake_is_visible(schema, form, "marks", list(exclaim = FALSE))
close(meta)
```

### MATLAB and Octave

```matlab
meta = reg.provider_meta('derived_greeter_shouter');   % []: no meta
form = meta.at(guatiao.intake.provider_form_key());
schema = reg.provider_config('derived_greeter_shouter');
guatiao.intake.check(schema, form)                     % []: it fits
guatiao.intake.is_visible(schema, form, 'marks', struct('exclaim', false))
meta.close();
```

### Fortran

```fortran
call reg%provider_meta('derived_greeter_shouter', meta)   ! absent: no meta
form = meta%key(GUATIAO_INTAKE_PROVIDER_FORM_KEY)
schema = reg%provider_config('derived_greeter_shouter')
call guatiao_intake_check(schema, form, why, st)          ! GUATIAO_OK: it fits
call guatiao_intake_is_visible(schema, form, 'marks', values, shown)
```

In every binding the `meta` is a copy, outliving the library, that you
close (or that its finalizer frees); the schema is the registry's (a
borrowed view in Python, Dart, Node, Go, Julia and Fortran, a copy in R
and MATLAB).

## Every provider, before any is built

A settings screen lists every provider of every library a host loaded,
with each one's form, and builds none of them: its kinds, id, name and
version, the digest of the file its library was mapped from, whether it
can run here and why not, its configuration schema, and the form to show
it with. A provider that declares no form is shown with the one its
schema implies (one section per `x-section`, ids only, a secret masked);
one that takes no configuration has nothing to show. Only asking for an
instance calls a provider's `create`.

### Rust

```rust
use guatiao::Alloc;
use guatiao_intake::provider_form;

for provider in registry.all() {
    let digest = registry.library_of(provider).map(|library| library.digest());
    let unavailable = provider.available().err();        // asked now
    let form = provider_form(provider.config_schema(), provider.meta(), Alloc::rust())?;
    // provider.kinds(), .id(), .display_name(), .version(), .config_schema()
}
```

`Loaded::digest` answers `sha256:` and 64 hex digits of the file the
library was mapped from, and only while the file at its absolute path is
still that file, unchanged: the registry records the file's identity
(device and inode, or volume and file index, with its size and last
write) just before and after mapping it, and checks it before and after
hashing. A file replaced or written since is an error, "the file changed
since it was loaded", never another file's digest. `None` for a library
from no file (registered, or served by a source). The first time it is
asked it reads the whole file on the calling thread; a digest is kept,
an error is not.

### C

```c
guatiao_value catalog = {0}, form = {0};
guatiao_registry_catalog(reg, &alloc, &catalog);   /* a list of maps, best first */
guatiao_values all = guatiao_list_items(&catalog);
for (size_t i = 0; i < all.len; i++) {
  const guatiao_value *one = &all.ptr[i];
  /* id, display_name, version, kinds, library, library_version, digest
     or digest_error, available and reason, config, meta */
  guatiao_intake_provider_form(guatiao_map_find(one, guatiao_cstr("config")),
                               guatiao_map_find(one, guatiao_cstr("meta")), &alloc, &form);
  /* ABSENT: nothing to configure */
  guatiao_value_free(&form);
}
guatiao_value_free(&catalog);
```

### Python

```python
import guatiao_intake as intake

for entry in reg.catalog():                     # plain dicts, best first
    form = intake.provider_form(entry.get("config"), entry.get("meta"))
    # entry.get("digest") or entry["digest_error"], entry["available"], ...
```

### Dart

```dart
import 'package:guatiao_intake/guatiao_intake.dart' as intake;

for (final entry in reg.catalog()) {             // plain maps, best first
  final form = intake.providerForm(entry['config'], entry['meta']);   // null: nothing to configure
  // entry['digest'] ?? entry['digest_error'], entry['available'], ...
  form?.close();
}
```

`providerForm` reads the form; whatever is malformed is skipped as above,
so a value under `"form"` that is not a map gives the schema's form.

### Node.js

```ts
for (const entry of reg.catalog()) {             // plain objects, best first
  using form = intake.providerForm(entry.config, entry.meta);   // null: nothing to configure
  // entry.digest ?? entry.digest_error, entry.available, ...
}
```

### Go

```go
catalog, _ := reg.Catalog()                      // []map[string]any, best first
for _, entry := range catalog {
	form, _ := intake.ProviderForm(entry["config"], entry["meta"]) // nil: nothing to configure
	// entry["digest"] or entry["digest_error"], entry["available"], ...
	form.Close()
}
```

### Julia

```julia
for entry in Guatiao.catalog(reg)                # Dicts, best first
    form = GuatiaoIntake.provider_form(get(entry, "config", nothing), get(entry, "meta", nothing))
    # nothing: nothing to configure; else close(form)
    # get(entry, "digest", nothing) or entry["digest_error"], entry["available"], ...
end
```

### R

```r
for (entry in reg$catalog()) {                   # lists, best first
  form <- intake_provider_form(entry[["config"]], entry[["meta"]])   # NULL: nothing to configure
  # entry$digest or entry$digest_error, entry$available, ...
  if (!is.null(form)) close(form)
}
```

### MATLAB and Octave

```matlab
catalog = reg.catalog();                         % a cell of structs, best first
for i = 1:numel(catalog)
  entry = catalog{i};
  config = []; meta = [];                        % a field the entry lacks is []
  if isfield(entry, 'config'), config = entry.config; end
  if isfield(entry, 'meta'), meta = entry.meta; end
  form = guatiao.intake.provider_form(config, meta);   % []: nothing to configure
  % entry.digest or entry.digest_error, entry.available, ...
end
```

### Fortran

```fortran
call reg%catalog(catalog)                        ! a list of maps, best first
do i = 1, catalog%length()
  entry = catalog%item(i)
  config = entry%key('config')                   ! absent when the entry has none
  meta = entry%key('meta')
  call guatiao_intake_provider_form(config, meta, form)   ! absent: nothing to configure
  ! entry%key('digest') or entry%key('digest_error'), entry%key('available'), ...
end do
```

### C++

```cpp
guatiao::value catalog = reg.catalog();          // a list of maps, best first
for (guatiao::view entry : catalog.as_list()) {
  guatiao::value form = guatiao::intake::provider_form(entry["config"], entry["meta"]);
  // form.is_absent(): nothing to configure
  // entry["digest"] or entry["digest_error"], entry["available"], ...
}
```

Every `providerForm` reads the form the same way: whatever is malformed is
skipped, and a provider with neither a form nor a configuration schema
answers nothing, which is not an error.
