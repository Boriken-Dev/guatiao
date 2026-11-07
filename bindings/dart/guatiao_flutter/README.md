# guatiao_flutter

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)

Flutter widgets for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values, over the [`guatiao`](https://pub.dev/packages/guatiao) and
[`guatiao_intake`](https://pub.dev/packages/guatiao_intake) packages, which
stay free of Flutter.

> **Status: alpha.** It uses Flutter's own Material widgets and no other
> UI package.

## Forms

`IntakeFormView` draws a form for a schema: a control for each field by
what the schema says it holds, the sections and visibility of the form it
is shown with, and the value being filled in kept as a guatiao value.

- **Controls by kind**: text, number, integer, boolean, enum, a union's
  arm, lists and maps of values or of groups, nested groups.
- **Secrets**: a sensitive writable field is a password field; a sensitive
  read-only one is masked, with a reveal toggle and a copy action.
- **Bytes**: the app supplies `onPickBytes` and `onSaveBytes`; the view
  shows a length and the `contentMediaType`.
- **Unavailable**: `unavailable` and `unavailableChoices` grey out a field
  or a choice the person lacks, with the reason as helper text and tooltip;
  the value is kept.
- **Problems**: `problems` shows what is wrong with a field by its path.

```dart
final model = FormModel(
  schema: {
    'type': 'object',
    'properties': {
      'name': {'type': 'string', 'title': 'Name'},
      'retries': {'type': 'integer', 'title': 'Retries', 'minimum': 0},
      'verbose': {'type': 'boolean', 'title': 'Verbose'},
    },
  },
  value: {'name': 'demo', 'retries': 3},
);
// In a build method, inside a scrollable:
IntakeFormView(model: model)
```

`model.value()` answers the value as plain Dart; call `model.dispose()`
when done. A provider's configuration form comes from
`FormModel.forProvider(schema: ..., meta: ...)`.

## Native library

Like `guatiao_intake`, the package needs the guatiao library: an
application's build hook bundles it; for this package's own tests set
`GUATIAO_LIBRARY` to a directory built with `cargo build -p guatiao-intake`.
