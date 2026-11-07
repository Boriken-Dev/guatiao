// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Flutter widgets for guatiao values.
///
/// **Forms**: [IntakeFormView] draws a [FormModel], a schema and the form
/// it is shown with (guatiao_intake's sections, widget hints and
/// visibility), holding the value being filled in as a `guatiao.Value`.
library;

export 'src/form/field_schema.dart' show FieldSchema, VariantArm;
export 'src/form/form_model.dart' show FieldHints, FormGroup, FormModel;
export 'src/form/form_view.dart'
    show ChoicesFor, FieldChoices, IntakeFormView, PickBytes, SaveBytes;
export 'src/form/paths.dart'
    show entryPath, isBareName, keyedPath, memberPath, needsQuoting;
