// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:guatiao/guatiao.dart' as guatiao;

import 'field_schema.dart';
import 'form_model.dart';
import 'paths.dart';

part 'controls.dart';

/// What a field that names something may hold: what to offer, and whether
/// a value not offered is taken too.
class FieldChoices {
  const FieldChoices(this.values, {this.open = false, this.labels = const {}});

  final List<String> values;

  /// A list that cannot be complete, so a typed value is taken.
  final bool open;

  /// What a person is shown for a value, when not the value itself.
  final Map<String, String> labels;
}

/// Answers the choices for a `format` the schema names; `null` when that
/// format offers none.
typedef ChoicesFor = FieldChoices? Function(String format);

/// Asks the person for the bytes of the field at [path], whose schema is
/// [field] (its `contentMediaType` says what they are); `null` when they
/// chose nothing.
typedef PickBytes = Future<Uint8List?> Function(String path, FieldSchema field);

/// Hands the bytes the field at [path] holds to the person, to keep.
typedef SaveBytes =
    Future<void> Function(String path, Uint8List bytes, FieldSchema field);

/// Draws a [FormModel]: its sections in order, each control by what the
/// schema says the field holds, the fields the form hides left out, and
/// what is rarely changed (`x-advanced`) folded away.
class IntakeFormView extends StatelessWidget {
  const IntakeFormView({
    required this.model,
    this.choicesFor,
    this.problems = const {},
    this.enabled = true,
    this.unavailable = const {},
    this.unavailableChoices = const {},
    this.onPickBytes,
    this.onSaveBytes,
    super.key,
  });

  final FormModel model;

  /// Choices for a text field by its `format`, from the app.
  final ChoicesFor? choicesFor;

  /// What is wrong with a field, by its path; shown before the model's
  /// own [FormModel.issues].
  final Map<String, String> problems;

  /// `false` shows every control disabled.
  final bool enabled;

  /// Fields the person may not change, by path, with the reason: each is
  /// disabled and greyed with its reason shown, and keeps its value. A
  /// field with members takes everything under it along.
  final Map<String, String> unavailable;

  /// Choices the person may not pick, by the field's path and then the
  /// choice's value, with the reason: each is disabled and greyed with its
  /// reason shown. A field holding one keeps it.
  final Map<String, Map<Object, String>> unavailableChoices;

  /// Fills a bytes field; without it a bytes field cannot be filled.
  final PickBytes? onPickBytes;

  /// Hands a bytes field's content out; without it there is no save.
  final SaveBytes? onSaveBytes;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: model,
    builder: (context, _) => _Fields(
      form: _FormContext(
        model: model,
        choicesFor: choicesFor,
        problems: problems,
        enabled: enabled,
        unavailable: unavailable,
        unavailableChoices: unavailableChoices,
        onPickBytes: onPickBytes,
        onSaveBytes: onSaveBytes,
      ),
      path: '',
      schema: model.schema,
    ),
  );
}

class _FormContext {
  const _FormContext({
    required this.model,
    required this.choicesFor,
    required this.problems,
    required this.enabled,
    required this.unavailable,
    required this.unavailableChoices,
    required this.onPickBytes,
    required this.onSaveBytes,
  });

  final FormModel model;
  final ChoicesFor? choicesFor;
  final Map<String, String> problems;
  final bool enabled;
  final Map<String, String> unavailable;
  final Map<String, Map<Object, String>> unavailableChoices;
  final PickBytes? onPickBytes;
  final SaveBytes? onSaveBytes;

  /// The host's problem with [path], else the model's own.
  String? problemAt(String path) => problems[path] ?? model.issues[path];

  bool hasProblemIn(String path) =>
      problems.keys.any((key) => isAtOrUnder(key, path)) ||
      model.issues.keys.any((key) => isAtOrUnder(key, path));

  /// Why [path] may not be changed: its own entry in [unavailable], else
  /// the nearest field above it that has one.
  String? unavailableReason(String path) {
    String? found;
    var longest = -1;
    for (final MapEntry(:key, :value) in unavailable.entries) {
      if (key.isNotEmpty && isAtOrUnder(path, key) && key.length > longest) {
        found = value;
        longest = key.length;
      }
    }
    return found;
  }

  /// Why something under [path] may not be changed (the first of its
  /// entries in [unavailable]), or `null`.
  String? unavailableUnder(String path) {
    for (final MapEntry(:key, :value) in unavailable.entries) {
      if (key != path && isAtOrUnder(key, path)) return value;
    }
    return null;
  }

  /// Why [path] may not be replaced or taken out as a whole: it is
  /// unavailable, or something under it is, and a replacement would drop
  /// that.
  String? lockReason(String path) =>
      unavailableReason(path) ?? unavailableUnder(path);

  /// Why the choice [value] of [path] may not be picked, or `null`.
  String? choiceReason(String path, Object? value) =>
      value == null ? null : unavailableChoices[path]?[value];

  /// Whether a person may change [path] now.
  bool canEdit(String path, FieldSchema schema) =>
      enabled && !schema.readOnly && unavailableReason(path) == null;
}

/// The members of what is at [path]: the record, a group, an entry.
class _Fields extends StatelessWidget {
  const _Fields({required this.form, required this.path, required this.schema});

  final _FormContext form;
  final String path;
  final FieldSchema schema;

  @override
  Widget build(BuildContext context) {
    final model = form.model;
    final children = <Widget>[];
    for (final group in model.layout(path, schema)) {
      final basic = <Widget>[];
      final advanced = <Widget>[];
      var advancedProblem = false;
      for (final key in group.fields) {
        if (!model.isVisible(path, key, schema)) continue;
        final field = schema.member(key);
        if (field == null) continue;
        final full = memberPath(path, key);
        final control = _Control(
          key: ValueKey('form-control-$full'),
          form: form,
          scopePath: path,
          scopeSchema: schema,
          relativeKey: key,
          path: full,
          schema: field,
          hints: model.hints(path, key, schema),
          required: schema.requiredKeys.contains(key),
        );
        if (field.advanced) {
          advanced.add(control);
          advancedProblem = advancedProblem || form.hasProblemIn(full);
        } else {
          basic.add(control);
        }
      }
      if (basic.isEmpty && advanced.isEmpty) continue;
      children.add(
        _Section(
          id: path.isEmpty ? group.id : '$path/${group.id}',
          title: group.title,
          help: group.help,
          basic: basic,
          advanced: advanced,
          showAdvanced: advancedProblem,
        ),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
  }
}

/// One section: what most people need, then what few do, folded. A section
/// of only the latter is folded whole.
class _Section extends StatelessWidget {
  const _Section({
    required this.id,
    required this.title,
    required this.help,
    required this.basic,
    required this.advanced,
    required this.showAdvanced,
  });

  final String id;
  final String title;
  final String help;
  final List<Widget> basic;
  final List<Widget> advanced;
  final bool showAdvanced;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    Widget folded(String label) => ExpansionTile(
      key: ValueKey('form-more-$id-$showAdvanced'),
      tilePadding: EdgeInsets.zero,
      initiallyExpanded: showAdvanced,
      title: Text(label),
      subtitle: basic.isEmpty && help.isNotEmpty ? Text(help) : null,
      childrenPadding: const EdgeInsets.only(bottom: 8),
      children: advanced,
    );
    if (basic.isEmpty) return folded(title.isEmpty ? 'More settings' : title);
    return Padding(
      key: ValueKey('form-section-$id'),
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (title.isNotEmpty)
            Semantics(
              header: true,
              child: Text(title, style: theme.textTheme.titleSmall),
            ),
          if (help.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(help, style: theme.textTheme.bodySmall),
            ),
          const SizedBox(height: 8),
          ...basic,
          if (advanced.isNotEmpty) folded('More settings'),
        ],
      ),
    );
  }
}

/// Picks the control for one field, from what its schema says it holds.
class _Control extends StatelessWidget {
  const _Control({
    required this.form,
    required this.scopePath,
    required this.scopeSchema,
    required this.relativeKey,
    required this.path,
    required this.schema,
    required this.hints,
    required this.required,
    super.key,
  });

  final _FormContext form;

  /// The record, group or entry the field is a member of, and its schema:
  /// where its hints and visibility are asked.
  final String scopePath;
  final FieldSchema scopeSchema;

  /// The field's key there: a member, or `owner.member` for a variant's.
  final String relativeKey;
  final String path;
  final FieldSchema schema;
  final FieldHints hints;
  final bool required;

  @override
  Widget build(BuildContext context) {
    final format = schema.format;
    final choices = format == null ? null : form.choicesFor?.call(format);
    final widget = hints.widget;
    final Widget control;
    if (schema.isBytes) {
      control = _BytesControl(form, path, schema);
    } else if (schema.variantTag != null) {
      control = _VariantChoice(this);
    } else if (schema.memberArm != null) {
      control = _ArmChoice(form, path, schema);
    } else if (schema.numberArm != null && schema.wordArm != null) {
      control = _NumberOrWord(form, path, schema, hints);
    } else if (schema.hasMembers) {
      control = widget == 'dialog'
          ? _DialogGroup(form, path, schema)
          : _Group(form, path, schema);
    } else if (schema.isList) {
      final items = schema.items ?? FieldSchema(const {});
      control = items.hasMembers
          ? _ListOfGroups(form, path, schema, items)
          : _ListOfValues(
              form,
              path,
              schema,
              items,
              choices ??
                  (items.format == null
                      ? null
                      : form.choicesFor?.call(items.format!)),
            );
    } else if (schema.isOpenMap) {
      final values = schema.values ?? FieldSchema(const {});
      control = values.hasMembers
          ? _MapOfGroups(form, path, schema, values)
          : _MapOfValues(form, path, schema, values);
    } else if (schema.isBool) {
      control = _Switch(form, path, schema, checkbox: widget == 'checkbox');
    } else if (schema.sensitive && schema.readOnly) {
      control = _SecretView(form, path, schema, hints);
    } else if (schema.choices != null) {
      control = widget == 'radio'
          ? _RadioChoice(form, path, schema, required)
          : _Choice(form, path, schema, required);
    } else if (choices != null) {
      control = _Picker(form, path, schema, choices);
    } else if (schema.suggestions case final offered?) {
      control = _Picker(
        form,
        path,
        schema,
        FieldChoices(
          offered,
          open: true,
          labels: {for (final value in offered) value: schema.labelOf(value)},
        ),
      );
    } else if (widget == 'slider' &&
        (schema.isInteger || schema.isNumber) &&
        schema.hasRange) {
      control = _SliderControl(form, path, schema, hints);
    } else {
      control = _TextControl(form, path, schema, hints, required);
    }
    // The reason shows on the field it names; what is under it is only
    // disabled.
    final reason = form.unavailable[path];
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: reason == null ? control : _Greyed(path, reason, control),
    );
  }
}
