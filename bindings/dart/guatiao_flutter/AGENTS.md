# guatiao_flutter — API header

Flutter widgets for guatiao values: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.
Depends on Flutter's SDK (`package:flutter/material.dart`), package:guatiao
and package:guatiao_intake at the same version; needs their native library
(see package:guatiao_intake's header).

Import `package:guatiao_flutter/guatiao_flutter.dart`.

## Forms

- `FormModel({required schema, form, value})` — a `ChangeNotifier` holding
  the value being filled in as a `guatiao.Value`. `schema`/`form` are plain
  maps; `form == null` uses `forSchema`. `value` is a map or `Ref` (copied).
  Throws `StateError` when the form does not fit the schema, `ArgumentError`
  when the value is not a map. `FormModel.forProvider(schema:, meta:, value:)`
  uses `providerForm`. `read(path)` plain Dart, `holds`, `text`, `setText`,
  `setValue`, `remove`, `addToList`, `removeFromList`, `addToMap`,
  `removeFromMap`, `chooseArm`, `issues`; `dispose()` frees the value.
- `IntakeFormView({required model, choicesFor, problems = {}, enabled = true,
  unavailable = {}, unavailableChoices = {}, onPickBytes, onSaveBytes})` —
  draws the model; put it in a scrollable.
  - `choicesFor(format)` returns `FieldChoices?` for a text field's `format`.
  - `problems`: path to message. `unavailable`: path to reason (disabled,
    greyed, value kept; a field takes its members along).
    `unavailableChoices`: path, then choice, to reason.
  - `onPickBytes(path, field)` returns `Uint8List?`;
    `onSaveBytes(path, bytes, field)`. Without them a bytes field cannot be
    filled or saved.
- Read from the schema: `type`, `format`, `enum`/`oneOf`, `x-enum-labels`,
  `minimum`/`maximum`, `x-sensitive`/`writeOnly`, `readOnly`,
  `contentEncoding`, `contentMediaType`, `default`, `title`, `description`,
  `x-section`, `x-order`, `x-advanced`; from the form: `widget`,
  `placeholder`, `unit`, `visibleWhen`, sections.
- Paths use guatiao_intake's grammar (`agent[1].name`); controls carry keys
  `form-field-<path>`.
