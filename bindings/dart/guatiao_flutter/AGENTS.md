# guatiao_flutter — API header

Flutter widgets for guatiao values: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.
Depends on Flutter's SDK (`package:flutter/material.dart`), package:guatiao
and package:guatiao_intake at the same version; needs their native library
(see package:guatiao_intake's header) or, on the web, the module the page
loaded with `loadModule`.

Import `package:guatiao_flutter/guatiao_flutter.dart`.

## Forms

### `IntakeFormView`

`IntakeFormView({required model, choicesFor, problems = {}, enabled = true,
unavailable = {}, unavailableChoices = {}, onPickBytes, onSaveBytes})` draws
a `FormModel`; put it in a scrollable.

- `choicesFor(format)` answers `FieldChoices?` for a text field's `format`.
- `problems`: path to message, shown before the model's own `issues`.
- `unavailable`: path to reason. The field is disabled and greyed with its
  reason, its value kept. Everything under it is disabled too, and an entry
  of a list or map can be named (`tags[1]`, `env[PATH]`). A container whose
  entry, or something in it, is unavailable cannot take that entry out, and
  an arm of a union or variant cannot be switched while a field under it is
  unavailable (the switch would drop it).
- `unavailableChoices`: path, then choice, to reason. An unavailable choice
  is listed greyed with its reason and is never newly chosen, typed in
  included; a field holding one is locked, and keeps it.
- `onPickBytes(path, field)` answers `Uint8List?`; `onSaveBytes(path, bytes,
  field)`. Without them a bytes field cannot be filled or saved. A sensitive
  bytes field shows `Set`, never a length.
- `FieldChoices(values, {open = false, labels = {}})`: what a field that
  names something may hold; `open` takes a typed value too.
- Typedefs `ChoicesFor`, `PickBytes`, `SaveBytes`.

### `FormModel` (a `ChangeNotifier`)

`FormModel({required schema, form, value})`: `schema` and `form` plain maps;
`form == null` uses guatiao_intake's `forSchema`. `value` is a map or a
`Ref`, copied. A schema with an `x-sensitive` or `writeOnly` field keeps the
whole value in the wiping allocator. Throws `StateError` when the form does
not fit the schema, `ArgumentError` when the value is not a map.
`FormModel.forProvider({required schema, meta, value})` takes the form
`providerForm` answers for a provider's configuration.

- Properties: `schema` (`FieldSchema`), `tree` (the `guatiao.Value` being
  filled in; change it through the model), `form` (plain Dart), `issues`
  (path to text that did not read as its kind or broke a bound).
- Reading: `value({issues})` the value as plain Dart (a map); `read(path)`
  one field as plain Dart, `guatiao.absent` when unset; `holds(path)`;
  `text(path)` what a text box shows; `copyOf(path)` an independent
  `guatiao.Value` the caller closes (a secret's copy is in the wiping
  allocator); `lengthOf`, `positions`, `entryIds` for lists; `mapKeys` for
  open maps; `usesMemberArm`, `variantName` for unions.
- Writing: `setText(path, text, {at})` reads text as the field's kind (an
  integer past 2^53 stays exact, bounds compare exactly, `0x1F` is not a
  number) and keeps text that does not read in a draft named in `issues`;
  `setValue`, `remove`; `addToList(path, value)`, `addTextToList(path, text,
  {at})` (answers why it refused, or `null`), `removeFromList`;
  `addToMap(path, key, [value])` (answers why it refused, or `null`: an
  existing key is refused), `removeFromMap`; `chooseArm(path, word)` and
  `chooseVariant(path, schema, name)` pick an arm and drop what the new
  arm does not declare.
- Layout: `layout([path, schema])` the sections of a group (`FormGroup`: `id`,
  `title`, `help`, `fields`), `field(path)` a field's schema, `hints(path,
  key, [schema])` (`FieldHints`: `widget`, `placeholder`, `unit`, `form`,
  `node`), `isVisible(path, key, [schema])`.
- `fieldNamedIn(reason)` the longest field path a refusal's text names.
- `close()` releases the value and the schemas; `dispose()` closes and
  disposes.

### Schemas and paths

- `FieldSchema(node)` reads a JSON Schema node: `node`, `isBool`,
  `isInteger`, `isNumber`, `isText`, `isList`, `isObject`, `isBytes`,
  `isAny`, `hasMembers`, `isOpenMap`, `title`, `description`, `format`,
  `section`, `order`, `advanced`, `sensitive`, `readOnly`, `defaultValue`,
  `hasDefault`, `minimum` and `maximum` (a `num` or a `BigInt`), `hasRange`,
  `contentEncoding`, `contentMediaType`, `isBase64`, `requiredKeys`,
  `choices`, `suggestions`, `labelOf(value)`, `arms`, `numberArm`,
  `wordArm`, `memberArm`, `variantTag`, `variantArms`, `variantArm(name)`,
  `member(key)`, `memberKeys`, `items`, `values`.
- `VariantArm`: `tag`, `name`, `schema`, `label`, `memberKeys`.
- Path helpers (guatiao_intake's grammar, `agent[1].name`, `env["a b"]`):
  `memberPath(owner, key)`, `entryPath(owner, position)`, `keyedPath(owner,
  key)`, `isBareName`, `needsQuoting`.

### What it reads

- From the schema: `type`, `format`, `enum`, `const`, `oneOf` and
  `x-enum-labels`, `examples` (suggestions for text), `x-variant-tag` (with
  `oneOf` arms each holding the tag's `const`), `anyOf`, `minimum`,
  `maximum`, `default`, `title`, `description`, `required`, `items`,
  `properties`, `additionalProperties`, `x-sensitive`/`writeOnly`,
  `readOnly`, `contentEncoding`, `contentMediaType`, `x-section`,
  `x-order`, `x-advanced`.
- From the form: `widget` (`radio`, `checkbox`, `slider`, `textarea`,
  `password`, `dialog`), `placeholder`, `unit`, `visibleWhen`, sections.
- A sensitive writable field is a password field, never pre-filled; a
  sensitive read-only one is masked, with a reveal toggle and a copy action.
- Widget keys: `form-field-<path>` on each control.
