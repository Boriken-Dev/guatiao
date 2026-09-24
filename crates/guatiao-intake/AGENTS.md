# guatiao-intake — shipped API header

How a guatiao schema is shown: sections, widget hints and conditional
visibility, as a **value beside the schema**. Depends on `guatiao` and
nothing else; the `derive` feature adds `#[derive(Form)]`.

## The document

```text
form     := { "sections": [section, …], "fields": { <path>: hints, … } }
section  := { "id": text, "title": text, "description": text }
hints    := { "widget": text, "placeholder": text,
              "visibleWhen": { "field": <path>, "equals": <value> },
              "form": form }        // a field that HAS members
```

- Every key is optional except a section's `id`. **An empty map is a
  complete form.**
- **A path is a flat key**, and the grammar is the one in `path`: a dot
  is a member (`connection.tls.ca`), a bracket is a list position or a
  map key (`agent[0].name`, `env[PATH]`), and which one a bracket means
  is decided by what it is applied to. `flat::resolve` and
  `guatiao_intake_resolve` take exactly this. It used to be one dot deep
  and variant-only, so a form that hinted `agent[0].name` had nothing to
  resolve it.
- **Sections are listed in display order.** Which section a field is in
  is the schema's `x-section`; the form only says what a section is called.
  **`x-section`, `x-order` and `x-advanced` are this crate's keys**, in
  its own `vocab` — `guatiao` does not name them, because how to group
  and order controls is an opinion it does not hold; it carries them as
  annotations like any key it does not know. They sit on the **schema**,
  beside the field, not in the form document. `FormBuilder` and
  `FormFieldBuilder` write them, `FormField` reads them back off a
  `FieldRef`, and C gets them from `guatiao_intake.h` as
  `GUATIAO_INTAKE_KEY_X_SECTION` and its two siblings.
- **`x-sensitive` is not ours.** It stays on `guatiao`'s
  `FieldBuilder::sensitive`, because "never print this value" is obeyed
  by a log and a crash dump as much as by a form.
- **The default section's id is `""`**, which is what a field with no
  `x-section` reads as. A field naming a section the form does not declare
  joins it too.
- **Widget names are an open set.** Suggestions in `vocab::widget`
  (`text`, `textarea`, `password`, `number`, `slider`, `checkbox`,
  `toggle`, `select`, `radio`); a renderer shows an unknown one as its
  default for the kind.
- **A field with members may carry a form of its own**, under `form`,
  and it is a complete form document — the same shape as the top level.
  Its paths are **relative to that field**: a form under `connection`
  names `tls.ca`. Given to a field whose kind has no members, `check`
  refuses it; there would be nothing for its paths to name. A **variant**
  is refused too, and that one is a judgement: its members differ by arm,
  so there is no one schema the sub-form could be checked against, and a
  form already reaches an arm's fields with `owner.member`.
- **A condition inside a sub-form may only name that form's own
  fields.** Not a rule written twice: the sub-form is checked against the
  member schema, so anything outside it is simply not a field. A window
  whose contents depended on something the window does not show would be
  a window a person cannot satisfy.
- Widgets for one: `dialog` (a window of its own) and `group` (inline,
  which is what a renderer does with no widget at all — it is there for a
  form that wants to say so beside one that says `dialog`).
- Anything else is an **annotation**: carried, never interpreted.
- A value's own read-only-ness is **not** a form hint. It is a statement
  about the value, and JSON Schema's `readOnly` in the schema says it.

## What else lives here

Three things that are **not** about drawing, and are here because they
are decisions a consumer makes about a schema rather than facts about
the struct:

```rust
// naming one place inside a value: `agent[1].name[home].host`
path::parse(text) -> Result<Path, PathError>    // every refusal says its byte offset
path::get(&value, path) -> Option<&Value>  /  path::get_mut(..)
Segment::{Field(&str), Index(usize), Key(Cow<str>)}
// C: guatiao_intake_path_get(value, path) -> const guatiao_value*  (borrowed, null when absent)

// projecting a value onto flat `key -> text` storage
flatten(field, &value, &mut BTreeMap<String, String>) -> bool
unflatten(alloc, field, &store) -> Option<Value>
resolve(schema, key) -> Option<FieldRef>         // for the schema alone
resolve_in(schema, key, selected) -> Result<FieldRef, ValidationError>
keys(field) -> Vec<String>  /  split(key)  /  is_sensitive(schema, key)
clear_under(&mut store, path)                    // everything AT or UNDER it
SEPARATOR = '.'

// checking a whole store of text
validate_texts(schema, &BTreeMap<String, String>) -> Result<(), ValidationError>
```

**The grammar**: a `.` is a field, `[0]` a list position **or** a map
key, `[name]` a map key, `["a.b"]` a map key that would otherwise read
as something else. Which one a bracket means is decided where it is
applied, not by how it is written -- so `env[PATH]` needs no ceremony
and `["1"]` is the escape for a key that looks like a position. A path
is rootless: no leading `.`, no `$`.

`validate_text` (one text, one field) stays in `guatiao`: that one asks
about the schema alone.

## The whole surface

```rust
// declaring, beside the schema: these EXTEND `guatiao`'s builders, which
// write JSON Schema's own `title`, `description` and nothing presentational
trait FormBuilder: guatiao::schema::Extras {   // SchemaBuilder, FieldBuilder, ArmBuilder
    fn section(self, id: &str) -> Self;        // x-section
}
trait FormFieldBuilder: FormBuilder {          // FieldBuilder only
    fn order(self, order: i64) -> Self;        // x-order
    fn advanced(self) -> Self;                 // x-advanced
}
trait FormField {                              // reading, on guatiao's FieldRef
    fn section(&self) -> &str;                 // "" when unset
    fn order(&self) -> i64;                    // 0 means "no ordering information"
    fn is_advanced(&self) -> bool;
}

// building (names no allocator; `_in` forms name one; errors collected)
Form::new() / Form::new_in(Alloc) -> Form
  .section(Section) / .field(path: &str, Hints) / .option(key, impl Into<Value>)
  .finish() -> Result<Value, ValueError>
Section::new(id) / Section::new_in(Alloc, id) -> Section
  .label(&str)   // written as `title`
  .help(&str)    // written as `description`
  .option(key, impl Into<Value>)
Hints::new() / Hints::new_in(Alloc) -> Hints
  .widget(&str) / .placeholder(&str)
  .visible_when(path: &str, equals: impl Into<Value>)
  .form(Form)                        // a field that has members; paths RELATIVE
  .form_value(Result<Value, ValueError>)   // the same, for generated code
  .option(key, impl Into<Value>)

// reading (borrowed views; skip what is malformed)
FormRef::new(&Value) -> Option<FormRef>
  .sections() -> impl Iterator<Item = SectionRef>
  .section(id) -> Option<SectionRef>
  .hints(path) -> HintsRef                     // empty when none
  .fields() -> impl Iterator<Item = (&str, HintsRef)>
  .extra(key) -> Option<&Value>  /  .as_value() -> &Value
SectionRef: .id() .label() .help() -> &str     // "" when absent
            .extra(key) -> Option<&Value>  .as_value() -> &Value
HintsRef:   .is_empty() -> bool  .widget() .placeholder() -> &str
            .visible_when() -> Option<Condition>
            .form() -> Option<FormRef>     // what a renderer nests by
            .extra(key) -> Option<&Value>
            .as_value() -> Option<&Value>      // None when the form says nothing
Condition:  .field() -> &str  .equals() -> &Value

// a type's default screen (`derive` feature); the trait is `Screen`
// because `Form` is the builder
#[derive(Schema, Form)]
#[form(section(id = "net", label = "Network", help = ".."), section(id = "login"))]  // display order
struct T {
    #[schema(section = "net")] host: String,              // WHICH section stays the schema's
    #[form(widget = "password")] token: String,
    #[form(placeholder = "..", visible_when(field = "verify", equals = true))] ca: Option<String>,
    #[form(nested)] auth: Auth,                           // `Auth: Screen`; its hints land under `auth.`
}
trait Screen { fn form(Alloc) -> Result<Value, ValueError>; fn hints(prefix: &str, into: Form) -> Form; }
T::form(alloc)                                 // sections, then hints at the root
Auth::hints("session.auth.", Form::new())      // a member's hints under a prefix; no sections
Form::alloc() -> Alloc                          // the builder's allocator, for hints built beside it

// the judgement
check(SchemaRef, FormRef) -> Result<(), FormError>
layout(SchemaRef, FormRef) -> Vec<Group>       // Group { section: Option<SectionRef>, fields: Vec<Placed> }
                                               // Placed { field: FieldRef, hints: HintsRef }
is_visible(SchemaRef, FormRef, path: &str, values: &Value) -> Result<bool, FormError>

FormError = Malformed { at, expected } | UnknownField { at, path }
          | DuplicateSection { id } | ConditionRefused { at, field, expected }
          | CyclicCondition { path }              // #[non_exhaustive]

// the keys a form is written with: `pub mod vocab`
vocab::{SECTIONS, FIELDS}                         // the form
vocab::{ID, TITLE, DESCRIPTION, DEFAULT_SECTION}  // a section; DEFAULT_SECTION is ""
vocab::{WIDGET, PLACEHOLDER, VISIBLE_WHEN}        // a field's hints
vocab::{FIELD, EQUALS}                            // a condition
vocab::widget::{TEXT, TEXTAREA, PASSWORD, NUMBER, SLIDER,
                CHECKBOX, TOGGLE, SELECT, RADIO}  // suggestions, an open set
```

**`is_visible` answers a `Result`**: a path the schema does not declare —
including one a condition reads — is the same `UnknownField` `check`
gives it, rather than `true` for a field that does not exist.

## The rules the judgement applies

- **`check`** refuses: a path that resolves to no field, a section id
  declared twice, a key of the wrong shape (named by its location, e.g.
  `fields["port"].widget`), a condition whose `equals` the referenced field
  would never hold, and a field whose conditions lead back to itself. A
  section a field names but the form does not declare is **not** an error.
  **No error quotes a value**: a condition's `equals` is something a field
  could hold, and a field can be a secret.
- **`layout`** puts the default section first unless the form declares
  `""` somewhere else; then the declared sections in order; a section
  nothing is in is left out. Within a group, fields with an explicit
  `x-order` come first, lowest first, and the rest keep the schema's
  order. **Top-level fields only**: an arm's fields are shown with their
  variant, and their hints are `hints("owner.member")`. Visibility is not
  applied — it needs values.
- **`is_visible`**: no condition is shown. A condition is met when the
  field it reads is **itself shown** and holds `equals`, so hiding a field
  hides everything that waits on it. A field holding nothing reads as its
  schema `default`; with no default the condition is not met. A variant
  named by its own key reads as its **discriminant**, so `equals` is an arm
  name. An `owner.member` path is read **only while the arm that declares
  `member` is the one picked** — a variant's value is one map, so moving
  between arms leaves keys behind and a stale one is not what the field
  holds. A cycle is never shown. A path the schema does not declare is an
  error. `values` is shaped like the schema's values: a variant's value is
  a map carrying its tag.

## From C (`include/guatiao_intake.h`)

Where the header is, for a consumer's own build script:
`DEP_GUATIAO_INTAKE_INCLUDE` (this crate says `links = "guatiao-intake"` and
its `build.rs` publishes `include=`), the arrangement `guatiao` has with
`DEP_GUATIAO_INCLUDE`. Copy the file beside your own header from there
rather than hard-coding a path into a checkout.

```c
guatiao_status guatiao_intake_check(const guatiao_value *schema, const guatiao_value *form,
                                  const guatiao_alloc *alloc, guatiao_value *out_error);
guatiao_status guatiao_intake_layout(const guatiao_value *schema, const guatiao_value *form,
                                   const guatiao_alloc *alloc, guatiao_value *out);
guatiao_status guatiao_intake_is_visible(const guatiao_value *schema, const guatiao_value *form,
                                       guatiao_str key, const guatiao_value *values, bool *out);
```

- `_check`: `GUATIAO_ERR_BAD_VALUE` on a misfit; `out_error` (may be null)
  gets `{ kind, at, path | id | field, expected, message }`, where `kind`
  is `malformed`, `unknown_field`, `duplicate_section`,
  `condition_refused` or `cyclic_condition`.
- `_layout`: a list of `{ "section": <the section's map, or null>,
  "fields": [key, …] }`. **Keys, not copies** — a field's schema is one
  `guatiao_intake_resolve` away and its hints are in the form you passed.
- `_is_visible`: the answer through `out`.
- `_is_visible`: `GUATIAO_ERR_NOT_FOUND` when `key`, or a key a condition
  reads, names no field the schema declares — and `out` is then not
  written.
- Every function: `GUATIAO_ERR_NULL` for a null required pointer, checked
  before anything else happens; `GUATIAO_ERR_WRONG_KIND` when the schema
  or form is not a map; **`GUATIAO_ERR_ALLOC` for an allocator that is
  null or cannot allocate**, the same status `guatiao-serde` gives; and
  `GUATIAO_ERR_INTERNAL` if a panic was caught. Results are built through
  `alloc` and freed with `guatiao_value_free`.

## Gotchas

- `equals` is compared **structurally**, as `guatiao::value::read::equal`
  does. A number is its text, so `443` and `443.0` are different values.
- `visibleWhen` is for what a variant cannot say. "These fields exist only
  for this arm" is already a variant, and hiding them again with a
  condition would give two sources of truth for one rule.
- `Form::field` twice with one path **replaces**, the way `set` does.
- **`#[derive(Form)]` is one screen, the type's default.** A second
  screen is a form built or read as a value, and a form beside the derived
  one overrides it key for key. Keys follow `#[map(rename)]`; a
  `#[map(skip)]` field cannot carry hints; `#[form(section = ..)]` on a
  field is refused (that is `#[schema(section)]`); an enum of unit
  variants is refused (a choice has no fields to place); a tagged enum
  places its arm fields, composed under `owner.` by the owner's `nested`.
- `HintsRef::as_value` answers `Option<&Value>` — `None` when the form
  says nothing about the field — where `FormRef` and `SectionRef` answer a
  plain `&Value`.
- **`unsafe` lives in `src/exports.rs` and `src/exports_flat.rs`**,
  checked by `tests/unsafe_stays_in_exports.rs`; every other module carries
  `#![forbid(unsafe_code)]`.
