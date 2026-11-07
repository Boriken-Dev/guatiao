// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

part of 'form_view.dart';

InputDecoration _decoration(
  _FormContext form,
  String path,
  FieldSchema schema, {
  bool required = false,
  String? hint,
  String? suffix,
  Widget? suffixIcon,
}) => InputDecoration(
  labelText: required ? '${schema.title} (required)' : schema.title,
  helperText: schema.description.isEmpty ? null : schema.description,
  helperMaxLines: 6,
  errorMaxLines: 6,
  errorText: form.problemAt(path),
  hintText: hint,
  suffixText: suffix,
  suffixIcon: suffixIcon,
  border: const OutlineInputBorder(),
);

/// How a field's default reads in an empty box.
String? _defaultText(FieldSchema schema) {
  final value = schema.defaultValue;
  return switch (value) {
    null => null,
    final String text when text.isEmpty => null,
    final String text => 'Default: ${schema.labelOf(text)}',
    final num number => 'Default: ${_grouped(number)}',
    final bool on => on ? 'Default: on' : 'Default: off',
    final List<Object?> list when list.isNotEmpty =>
      'Default: ${list.join(', ')}',
    _ => null,
  };
}

String _grouped(num number) {
  final text = number.toString();
  if (number is! int) return text;
  final out = StringBuffer();
  for (var index = 0; index < text.length; index++) {
    if (index > 0 && (text.length - index) % 3 == 0 && text[index - 1] != '-') {
      out.write(',');
    }
    out.write(text[index]);
  }
  return out.toString();
}

/// A field the person may not change: greyed, its reason under it and in
/// a tooltip.
class _Greyed extends StatelessWidget {
  const _Greyed(this.path, this.reason, this.child);

  final String path;
  final String reason;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Tooltip(
      key: Key('form-unavailable-$path'),
      message: reason,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          child,
          Padding(
            padding: const EdgeInsets.only(top: 4, left: 12),
            child: Text(
              reason,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.disabledColor,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// One choice's words, greyed with its reason under them when it may not
/// be picked.
Widget _choiceLabel(BuildContext context, String label, String? reason) {
  if (reason == null) return Text(label, overflow: TextOverflow.ellipsis);
  final theme = Theme.of(context);
  return Tooltip(
    message: reason,
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          label,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(color: theme.disabledColor),
        ),
        Text(
          reason,
          overflow: TextOverflow.ellipsis,
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.disabledColor,
          ),
        ),
      ],
    ),
  );
}

/// What a hidden secret shows in its box: a fixed mask, never its length.
const _mask = '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022';

/// Keeps a text box in step with the model when the app changes the value
/// under it, and leaves it alone while the person is typing.
class _TextSync {
  _TextSync(this.controller, this._seen);

  final TextEditingController controller;
  String _seen;
  bool _typing = false;

  /// Call from the box's `onChanged`, before the model hears the text.
  void typed() => _typing = true;

  /// Call from `build` with what the model shows now.
  void sync(String current) {
    if (_typing) {
      _typing = false;
      _seen = current;
      return;
    }
    if (current == _seen) return;
    _seen = current;
    if (controller.text != current) controller.text = current;
  }
}

/// A field's title, its description and its problem, above what it holds.
class _Titled extends StatelessWidget {
  const _Titled({
    required this.schema,
    required this.children,
    this.problem,
    super.key,
  });

  final FieldSchema schema;
  final String? problem;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(schema.title, style: theme.textTheme.bodyMedium),
        if (schema.description.isNotEmpty)
          Text(schema.description, style: theme.textTheme.bodySmall),
        if (problem case final problem?)
          Text(
            problem,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
        const SizedBox(height: 6),
        ...children,
      ],
    );
  }
}

class _TextControl extends StatefulWidget {
  const _TextControl(
    this.form,
    this.path,
    this.schema,
    this.hints,
    this.required,
  );

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldHints hints;
  final bool required;

  @override
  State<_TextControl> createState() => _TextControlState();
}

class _TextControlState extends State<_TextControl> {
  late final _sensitive =
      widget.schema.sensitive || widget.hints.widget == 'password';

  // A secret is never put back in a box: an empty one keeps what is held.
  late final _controller = TextEditingController(
    text: _sensitive ? '' : widget.form.model.text(widget.path) ?? '',
  );
  late final guatiao.Value? _stored = _sensitive
      ? widget.form.model.copyOf(widget.path)
      : null;
  late final _sync = _TextSync(_controller, _controller.text);

  @override
  void dispose() {
    _controller.dispose();
    _stored?.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final schema = widget.schema;
    final model = widget.form.model;
    final isNumber = schema.isInteger || schema.isNumber;
    final multiline = widget.hints.widget == 'textarea' && !_sensitive;
    final hint = _sensitive && _stored != null
        ? 'Set; type to replace it'
        : widget.hints.placeholder ?? _defaultText(schema);
    final editable = widget.form.canEdit(widget.path, schema);
    if (!_sensitive) _sync.sync(model.text(widget.path) ?? '');
    return TextField(
      key: Key('form-field-${widget.path}'),
      controller: _controller,
      enabled:
          widget.form.enabled &&
          widget.form.unavailableReason(widget.path) == null,
      readOnly: schema.readOnly,
      obscureText: _sensitive,
      enableSuggestions: !_sensitive,
      autocorrect: !_sensitive,
      minLines: multiline ? 3 : 1,
      maxLines: multiline ? 8 : 1,
      keyboardType: isNumber
          ? const TextInputType.numberWithOptions(decimal: true, signed: true)
          : multiline
          ? TextInputType.multiline
          : switch (schema.format) {
              'email' => TextInputType.emailAddress,
              'uri' || 'url' || 'iri' => TextInputType.url,
              _ => TextInputType.text,
            },
      decoration: _decoration(
        widget.form,
        widget.path,
        schema,
        required: widget.required,
        hint: hint,
        suffix: isNumber ? widget.hints.unit : null,
      ),
      onChanged: editable
          ? (text) {
              _sync.typed();
              final stored = _stored;
              if (_sensitive && text.isEmpty) {
                if (stored != null) {
                  model.setValue(widget.path, stored);
                } else {
                  model.remove(widget.path);
                }
              } else {
                model.setText(widget.path, text, at: schema);
              }
            }
          : null,
    );
  }
}

/// A secret that is only shown: masked, with a reveal and a copy.
class _SecretView extends StatefulWidget {
  const _SecretView(this.form, this.path, this.schema, this.hints);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldHints hints;

  @override
  State<_SecretView> createState() => _SecretViewState();
}

class _SecretViewState extends State<_SecretView> {
  var _revealed = false;
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.form.model;
    final held = model.holds(widget.path);
    // The plaintext is read only to show it on a reveal: hidden, the box
    // holds a fixed mask.
    final shown = !held
        ? ''
        : _revealed
        ? model.text(widget.path) ?? ''
        : _mask;
    if (_controller.text != shown) _controller.text = shown;
    return TextField(
      key: Key('form-field-${widget.path}'),
      controller: _controller,
      readOnly: true,
      obscureText: !_revealed,
      enableSuggestions: false,
      autocorrect: false,
      decoration: _decoration(
        widget.form,
        widget.path,
        widget.schema,
        hint: !held ? 'Not set' : null,
        suffixIcon: !held
            ? null
            : Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  IconButton(
                    key: Key('form-reveal-${widget.path}'),
                    tooltip: _revealed ? 'Hide' : 'Show',
                    icon: Icon(
                      _revealed ? Icons.visibility_off : Icons.visibility,
                    ),
                    onPressed: () => setState(() => _revealed = !_revealed),
                  ),
                  IconButton(
                    key: Key('form-copy-${widget.path}'),
                    tooltip: 'Copy',
                    icon: const Icon(Icons.copy),
                    onPressed: () => Clipboard.setData(
                      ClipboardData(text: model.text(widget.path) ?? ''),
                    ),
                  ),
                ],
              ),
      ),
    );
  }
}

/// A number between its `minimum` and `maximum`, by a slider.
class _SliderControl extends StatelessWidget {
  const _SliderControl(this.form, this.path, this.schema, this.hints);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldHints hints;

  @override
  Widget build(BuildContext context) {
    final low = FieldSchema.boundToDouble(schema.minimum!);
    final high = FieldSchema.boundToDouble(schema.maximum!);
    final held = num.tryParse(form.model.text(path) ?? '');
    final fallback = schema.defaultValue;
    final shown = (held ?? (fallback is num ? fallback : low)).toDouble().clamp(
      low,
      high,
    );
    final steps = high - low;
    final whole = schema.isInteger;
    final unit = hints.unit;
    final words = whole ? _wholeText(shown, schema) : shown.toStringAsFixed(2);
    final problem = form.problemAt(path);
    return _Titled(
      schema: schema,
      problem: problem,
      children: [
        Row(
          children: [
            Expanded(
              child: Slider(
                key: Key('form-field-$path'),
                min: low,
                max: high,
                divisions: whole && steps <= 1000 ? steps.round() : null,
                value: shown,
                label: unit == null ? words : '$words $unit',
                onChanged: form.canEdit(path, schema)
                    ? (value) => form.model.setText(
                        path,
                        whole ? _wholeText(value, schema) : '$value',
                        at: schema,
                      )
                    : null,
              ),
            ),
            Text(unit == null ? words : '$words $unit'),
          ],
        ),
      ],
    );
  }
}

/// A slider's position as the exact whole number it stands for, within the
/// field's bounds: a double cannot name every integer past 2^53, and its
/// `round()` is not exact past 2^63.
String _wholeText(double value, FieldSchema schema) {
  var whole = BigInt.from(value.roundToDouble());
  final low = schema.minimumWhole;
  final high = schema.maximumWhole;
  if (low != null && whole < low) whole = low;
  if (high != null && whole > high) whole = high;
  return whole.toString();
}

class _Switch extends StatelessWidget {
  const _Switch(this.form, this.path, this.schema, {required this.checkbox});

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final bool checkbox;

  @override
  Widget build(BuildContext context) {
    final held = form.model.read(path);
    final on = held is bool ? held : schema.defaultValue == true;
    final problem = form.problemAt(path);
    // A held unavailable choice is locked, and an unavailable one is not
    // newly chosen.
    final locked =
        form.choiceReason(path, !on) ??
        (held is bool ? form.choiceReason(path, held) : null);
    final editable = form.canEdit(path, schema) && locked == null;
    final theme = Theme.of(context);
    final subtitle = problem != null
        ? Text(problem, style: TextStyle(color: theme.colorScheme.error))
        : locked != null
        ? Text(locked, style: TextStyle(color: theme.disabledColor))
        : schema.description.isEmpty
        ? null
        : Text(schema.description);
    void set(bool? value) => form.model.setValue(path, value == true);
    final Widget tile = checkbox
        ? CheckboxListTile(
            key: Key('form-field-$path'),
            contentPadding: EdgeInsets.zero,
            controlAffinity: ListTileControlAffinity.leading,
            title: Text(schema.title),
            subtitle: subtitle,
            value: on,
            onChanged: editable ? set : null,
          )
        : SwitchListTile(
            key: Key('form-field-$path'),
            contentPadding: EdgeInsets.zero,
            title: Text(schema.title),
            subtitle: subtitle,
            value: on,
            onChanged: editable ? set : null,
          );
    return locked == null ? tile : Tooltip(message: locked, child: tile);
  }
}

/// The position of what [path] holds among [values], or `null`.
int? _heldIndex(Object? held, List<Object?> values) {
  if (held is guatiao.Absent) return null;
  for (var at = 0; at < values.length; at++) {
    if (values[at] == held) return at;
  }
  return null;
}

/// One of a closed set of values (`enum`, `const`, a `oneOf` of `const`s),
/// as a dropdown.
class _Choice extends StatelessWidget {
  const _Choice(this.form, this.path, this.schema, this.required);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final bool required;

  @override
  Widget build(BuildContext context) {
    final values = [...schema.choices!];
    final held = form.model.read(path);
    var current = _heldIndex(held, values);
    if (current == null && held is! guatiao.Absent) {
      values.add(held);
      current = values.length - 1;
    }
    final fallback = schema.defaultValue;
    final heldReason = current == null
        ? null
        : form.choiceReason(path, values[current]);
    final editable = form.canEdit(path, schema) && heldReason == null;
    final labels = [
      if (!required)
        schema.hasDefault ? 'Default: ${schema.labelOf(fallback)}' : 'Not set',
      for (final value in values) schema.labelOf(value),
    ];
    final dropdown = DropdownButtonFormField<int>(
      key: Key('form-field-$path'),
      isExpanded: true,
      initialValue: current ?? (required ? null : -1),
      decoration: _decoration(form, path, schema, required: required),
      selectedItemBuilder: (context) => [
        for (final label in labels)
          Text(label, overflow: TextOverflow.ellipsis),
      ],
      items: [
        if (!required) DropdownMenuItem(value: -1, child: Text(labels.first)),
        for (var at = 0; at < values.length; at++)
          () {
            final reason = form.choiceReason(path, values[at]);
            return DropdownMenuItem(
              value: at,
              enabled: reason == null,
              child: _choiceLabel(context, schema.labelOf(values[at]), reason),
            );
          }(),
      ],
      onChanged: editable
          ? (at) {
              if (at == null || at < 0) {
                form.model.remove(path);
              } else {
                form.model.setValue(path, values[at]);
              }
            }
          : null,
    );
    if (heldReason == null) return dropdown;
    return Tooltip(
      message: heldReason,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          dropdown,
          Padding(
            padding: const EdgeInsets.only(top: 4, left: 12),
            child: Text(
              heldReason,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: Theme.of(context).disabledColor,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// One of a closed set of values, as radio buttons (`widget: radio`).
class _RadioChoice extends StatelessWidget {
  const _RadioChoice(this.form, this.path, this.schema, this.required);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final bool required;

  @override
  Widget build(BuildContext context) {
    final values = [...schema.choices!];
    final held = form.model.read(path);
    var current = _heldIndex(held, values);
    if (current == null && held is! guatiao.Absent) {
      values.add(held);
      current = values.length - 1;
    }
    final heldReason = current == null
        ? null
        : form.choiceReason(path, values[current]);
    final editable = form.canEdit(path, schema) && heldReason == null;
    return _Titled(
      key: Key('form-field-$path'),
      schema: schema,
      problem: form.problemAt(path),
      children: [
        RadioGroup<int>(
          groupValue: current ?? -1,
          onChanged: (at) {
            if (!editable) return;
            if (at == null || at < 0) {
              form.model.remove(path);
            } else {
              form.model.setValue(path, values[at]);
            }
          },
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!required)
                RadioListTile<int>(
                  key: Key('form-choice-$path-none'),
                  value: -1,
                  enabled: editable,
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  title: Text(
                    schema.hasDefault
                        ? 'Default: ${schema.labelOf(schema.defaultValue)}'
                        : 'Not set',
                  ),
                ),
              for (var at = 0; at < values.length; at++)
                () {
                  final reason = form.choiceReason(path, values[at]);
                  final tile = RadioListTile<int>(
                    key: Key('form-choice-$path-$at'),
                    value: at,
                    enabled: editable && reason == null,
                    dense: true,
                    contentPadding: EdgeInsets.zero,
                    title: Text(schema.labelOf(values[at])),
                    subtitle: reason == null ? null : Text(reason),
                  );
                  return reason == null
                      ? tile
                      : Tooltip(message: reason, child: tile);
                }(),
            ],
          ),
        ),
      ],
    );
  }
}

/// A field that names something from a list the app supplies, or that
/// the schema offers (`examples`): a choice when the list is complete, a
/// box with suggestions when it cannot be.
class _Picker extends StatelessWidget {
  const _Picker(this.form, this.path, this.schema, this.choices);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldChoices choices;

  @override
  Widget build(BuildContext context) {
    final stored = form.model.text(path);
    final editable = form.canEdit(path, schema);
    // What is held, not what was typed and refused.
    final held = form.model.read(path);
    final heldReason = held is String ? form.choiceReason(path, held) : null;
    if (!choices.open) {
      final values = <String>[
        ...{...choices.values, ...?schema.suggestions, ?stored}..remove(''),
      ];
      String label(String value) => choices.labels[value] ?? value;
      final none = _defaultText(schema) ?? 'None';
      return DropdownButtonFormField<String>(
        key: Key('form-field-$path'),
        isExpanded: true,
        initialValue: stored ?? '',
        decoration: _decoration(form, path, schema),
        selectedItemBuilder: (context) => [
          Text(none, overflow: TextOverflow.ellipsis),
          for (final value in values)
            Text(label(value), overflow: TextOverflow.ellipsis),
        ],
        items: [
          DropdownMenuItem(value: '', child: Text(none)),
          for (final value in values)
            DropdownMenuItem(
              value: value,
              enabled: form.choiceReason(path, value) == null,
              child: _choiceLabel(
                context,
                label(value),
                form.choiceReason(path, value),
              ),
            ),
        ],
        onChanged: editable && heldReason == null
            ? (value) => form.model.setText(path, value, at: schema)
            : null,
      );
    }
    return Autocomplete<String>(
      key: Key('form-field-$path'),
      initialValue: TextEditingValue(text: stored ?? ''),
      optionsBuilder: (value) {
        // Nothing is offered where nothing may be picked.
        if (!editable || heldReason != null) return const <String>[];
        final text = value.text.toLowerCase();
        return choices.values.where(
          (choice) =>
              form.choiceReason(path, choice) == null &&
              (choice.toLowerCase().contains(text) ||
                  (choices.labels[choice]?.toLowerCase().contains(text) ??
                      false)),
        );
      },
      displayStringForOption: (value) => value,
      onSelected: editable && heldReason == null
          ? (value) => form.model.setText(path, value, at: schema)
          : null,
      fieldViewBuilder: (context, controller, focus, onSubmitted) =>
          ValueListenableBuilder<TextEditingValue>(
            valueListenable: controller,
            builder: (context, typed, _) {
              final refused = form.choiceReason(path, typed.text);
              return TextField(
                controller: controller,
                focusNode: focus,
                enabled: form.enabled && form.unavailableReason(path) == null,
                readOnly: schema.readOnly || heldReason != null,
                decoration: _decoration(
                  form,
                  path,
                  schema,
                  hint: _defaultText(schema),
                ).copyWith(errorText: refused ?? form.problemAt(path)),
                // A held unavailable choice is locked, and an unavailable
                // one typed in is not taken.
                onChanged: editable && heldReason == null
                    ? (text) {
                        final reason = form.choiceReason(path, text);
                        if (reason == null) {
                          form.model.setText(path, text, at: schema);
                        } else {
                          form.model.refuse(path, text, reason);
                        }
                      }
                    : null,
                onSubmitted: (_) => onSubmitted(),
              );
            },
          ),
    );
  }
}

/// A number, or a word that takes its place, such as no limit at all.
class _NumberOrWord extends StatefulWidget {
  const _NumberOrWord(this.form, this.path, this.schema, this.hints);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldHints hints;

  @override
  State<_NumberOrWord> createState() => _NumberOrWordState();
}

class _NumberOrWordState extends State<_NumberOrWord> {
  late final Object? _word = widget.schema.wordArm!.choices!.first;
  late final _controller = TextEditingController(
    text: widget.form.model.read(widget.path) == _word
        ? ''
        : widget.form.model.text(widget.path) ?? '',
  );
  late final _sync = _TextSync(_controller, _controller.text);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.form.model;
    final schema = widget.schema;
    final lifted = model.read(widget.path) == _word;
    final wordArm = schema.wordArm!;
    final editable = widget.form.canEdit(widget.path, schema);
    // The word is a choice like any other: held unavailable, it is
    // locked; unavailable, it is not newly chosen.
    final wordReason = widget.form.choiceReason(widget.path, _word);
    _sync.sync(lifted ? '' : model.text(widget.path) ?? '');
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          key: Key('form-field-${widget.path}'),
          controller: _controller,
          enabled: editable && !lifted,
          keyboardType: const TextInputType.numberWithOptions(decimal: true),
          decoration: _decoration(
            widget.form,
            widget.path,
            schema,
            hint: _defaultText(schema),
            suffix: widget.hints.unit,
          ),
          onChanged: (text) {
            _sync.typed();
            // A word typed in is the choice it names: one the person lacks
            // is not taken.
            final reason = widget.form.choiceReason(widget.path, text.trim());
            if (reason != null) {
              model.refuse(widget.path, text, reason);
            } else {
              model.setText(widget.path, text, at: schema);
            }
          },
        ),
        CheckboxListTile(
          key: Key('form-word-${widget.path}'),
          contentPadding: EdgeInsets.zero,
          controlAffinity: ListTileControlAffinity.leading,
          dense: true,
          title: Text(wordArm.labelOf(_word)),
          subtitle: wordReason == null ? null : Text(wordReason),
          value: lifted,
          onChanged: editable && wordReason == null
              ? (value) => value == true
                    ? model.setValue(widget.path, _word)
                    : model.setText(widget.path, _controller.text, at: schema)
              : null,
        ),
      ],
    );
  }
}

/// One of several kinds (`anyOf`): a few words, or a group of fields of
/// its own.
class _ArmChoice extends StatelessWidget {
  const _ArmChoice(this.form, this.path, this.schema);

  final _FormContext form;
  final String path;
  final FieldSchema schema;

  static const _memberValue = '\u0000members';

  @override
  Widget build(BuildContext context) {
    final model = form.model;
    final member = schema.memberArm!;
    final words = [
      for (final word in schema.wordArm?.choices ?? const <Object?>[])
        if (word is String) word,
    ];
    final usesMembers = model.usesMemberArm(path);
    final current = usesMembers ? _memberValue : model.text(path) ?? '';
    final heldReason = usesMembers ? null : form.choiceReason(path, current);
    final memberLabel = member.title.isEmpty ? 'Custom' : member.title;
    String label(String word) => schema.wordArm!.labelOf(word);
    // What is held but not offered (a word the schema does not list, a
    // number) is listed too, so the box can show it.
    final unlisted =
        current.isEmpty || current == _memberValue || words.contains(current)
        ? null
        : current;
    // Switching arms drops what is under the fields, so it waits while
    // something under them is unavailable.
    final dropsLocked = usesMembers ? form.unavailableUnder(path) : null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DropdownButtonFormField<String>(
          key: Key('form-field-$path'),
          isExpanded: true,
          initialValue: current,
          decoration: _decoration(form, path, schema),
          selectedItemBuilder: (context) => [
            const Text('Default'),
            for (final word in words) Text(label(word)),
            if (unlisted != null) Text(unlisted),
            Text(memberLabel),
          ],
          items: [
            const DropdownMenuItem(value: '', child: Text('Default')),
            for (final word in words)
              DropdownMenuItem(
                value: word,
                enabled: form.choiceReason(path, word) == null,
                child: _choiceLabel(
                  context,
                  label(word),
                  form.choiceReason(path, word),
                ),
              ),
            if (unlisted != null)
              DropdownMenuItem(value: unlisted, child: Text(unlisted)),
            DropdownMenuItem(value: _memberValue, child: Text(memberLabel)),
          ],
          onChanged:
              form.canEdit(path, schema) &&
                  heldReason == null &&
                  dropsLocked == null
              ? (value) =>
                    model.chooseArm(path, value == _memberValue ? null : value)
              : null,
        ),
        if (dropsLocked != null)
          Padding(
            padding: const EdgeInsets.only(top: 4, left: 12),
            child: Text(
              dropsLocked,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: Theme.of(context).disabledColor,
              ),
            ),
          ),
        if (usesMembers)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: _Fields(form: form, path: path, schema: member),
          ),
      ],
    );
  }
}

/// A tagged variant (`x-variant-tag` and `oneOf`): which arm, then that
/// arm's fields.
class _VariantChoice extends StatelessWidget {
  const _VariantChoice(this.control);

  final _Control control;

  @override
  Widget build(BuildContext context) {
    final form = control.form;
    final path = control.path;
    final schema = control.schema;
    final model = form.model;
    final arms = schema.variantArms;
    final name = model.variantName(path, schema);
    final at = arms.indexWhere((arm) => arm.name == name);
    final current = at < 0 ? null : at;
    final heldReason = current == null
        ? null
        : form.choiceReason(path, arms[current].name);
    final editable = form.canEdit(path, schema) && heldReason == null;
    final labels = [
      if (!control.required) 'Not set',
      for (final arm in arms) arm.label,
    ];
    // A new arm drops the fields it does not declare, so the choice waits
    // while something under the variant is unavailable.
    final dropsLocked = form.unavailableUnder(path);
    final fields = <Widget>[];
    if (current != null) {
      final arm = arms[current];
      for (final key in arm.memberKeys) {
        final relative = memberPath(control.relativeKey, key);
        if (!model.isVisible(
          control.scopePath,
          relative,
          control.scopeSchema,
        )) {
          continue;
        }
        final full = memberPath(path, key);
        fields.add(
          _Control(
            key: ValueKey('form-control-$full'),
            form: form,
            scopePath: control.scopePath,
            scopeSchema: control.scopeSchema,
            relativeKey: relative,
            path: full,
            schema: arm.schema.member(key)!,
            hints: model.hints(
              control.scopePath,
              relative,
              control.scopeSchema,
            ),
            required: arm.schema.requiredKeys.contains(key),
          ),
        );
      }
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DropdownButtonFormField<int>(
          key: Key('form-field-$path'),
          isExpanded: true,
          initialValue: current ?? (control.required ? null : -1),
          decoration: _decoration(
            form,
            path,
            schema,
            required: control.required,
          ),
          selectedItemBuilder: (context) => [
            for (final label in labels)
              Text(label, overflow: TextOverflow.ellipsis),
          ],
          items: [
            if (!control.required)
              const DropdownMenuItem(value: -1, child: Text('Not set')),
            for (var i = 0; i < arms.length; i++)
              DropdownMenuItem(
                value: i,
                enabled: form.choiceReason(path, arms[i].name) == null,
                child: _choiceLabel(
                  context,
                  arms[i].label,
                  form.choiceReason(path, arms[i].name),
                ),
              ),
          ],
          onChanged: editable && dropsLocked == null
              ? (i) => model.chooseVariant(
                  path,
                  schema,
                  i == null || i < 0 ? null : arms[i].name,
                )
              : null,
        ),
        if ((heldReason ?? dropsLocked) case final reason?)
          Padding(
            padding: const EdgeInsets.only(top: 4, left: 12),
            child: Text(
              reason,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: Theme.of(context).disabledColor,
              ),
            ),
          ),
        if (fields.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 12, left: 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: fields,
            ),
          ),
      ],
    );
  }
}

/// A field with members of its own, inline.
class _Group extends StatelessWidget {
  const _Group(this.form, this.path, this.schema);

  final _FormContext form;
  final String path;
  final FieldSchema schema;

  @override
  Widget build(BuildContext context) => _Titled(
    key: Key('form-field-$path'),
    schema: schema,
    problem: form.problemAt(path),
    children: [
      DecoratedBox(
        decoration: BoxDecoration(
          border: Border(
            left: BorderSide(color: Theme.of(context).dividerColor, width: 2),
          ),
        ),
        child: Padding(
          padding: const EdgeInsets.only(left: 12),
          child: _Fields(form: form, path: path, schema: schema),
        ),
      ),
    ],
  );
}

/// A field with members of its own, in a window of its own
/// (`widget: dialog`).
class _DialogGroup extends StatelessWidget {
  const _DialogGroup(this.form, this.path, this.schema);

  final _FormContext form;
  final String path;
  final FieldSchema schema;

  @override
  Widget build(BuildContext context) {
    final problem = form.problemAt(path);
    final theme = Theme.of(context);
    return ListTile(
      key: Key('form-field-$path'),
      contentPadding: EdgeInsets.zero,
      title: Text(schema.title),
      subtitle: problem != null
          ? Text(problem, style: TextStyle(color: theme.colorScheme.error))
          : schema.description.isEmpty
          ? null
          : Text(schema.description),
      trailing: TextButton(
        key: Key('form-open-$path'),
        onPressed: () => showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            title: Text(schema.title),
            content: SingleChildScrollView(
              child: ListenableBuilder(
                listenable: form.model,
                builder: (context, _) =>
                    _Fields(form: form, path: path, schema: schema),
              ),
            ),
            actions: [
              TextButton(
                key: Key('form-close-$path'),
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('Close'),
              ),
            ],
          ),
        ),
        child: Text(form.canEdit(path, schema) ? 'Edit' : 'View'),
      ),
    );
  }
}

/// Bytes (`{"type": "bytes"}`): their length and media type, filled and
/// saved through the app's callbacks.
class _BytesControl extends StatelessWidget {
  const _BytesControl(this.form, this.path, this.schema);

  final _FormContext form;
  final String path;
  final FieldSchema schema;

  @override
  Widget build(BuildContext context) {
    final secret = schema.sensitive;
    // A secret's bytes are copied out of the value only to hand them to
    // the save callback.
    final present = form.model.holds(path);
    final held = secret || !present ? null : form.model.read(path);
    final bytes = held is Uint8List ? held : null;
    final media = schema.contentMediaType;
    final editable = form.canEdit(path, schema);
    final pick = form.onPickBytes;
    final save = form.onSaveBytes;
    final size = !present
        ? 'Empty'
        : secret
        ? 'Set'
        : '${_grouped(bytes?.length ?? 0)} bytes';
    return _Titled(
      key: Key('form-field-$path'),
      schema: schema,
      problem: form.problemAt(path),
      children: [
        Wrap(
          spacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Text(size, key: Key('form-size-$path')),
            if (media != null)
              Text(media, style: Theme.of(context).textTheme.bodySmall),
            if (pick != null)
              TextButton.icon(
                key: Key('form-pick-$path'),
                onPressed: editable
                    ? () async {
                        final chosen = await pick(path, schema);
                        if (chosen != null) {
                          form.model.setValue(path, chosen);
                        }
                      }
                    : null,
                icon: const Icon(Icons.upload_file),
                label: const Text('Choose'),
              ),
            if (save != null && present)
              TextButton.icon(
                key: Key('form-save-$path'),
                onPressed: () {
                  final now = form.model.read(path);
                  if (now is Uint8List) save(path, now, schema);
                },
                icon: const Icon(Icons.download),
                label: const Text('Save'),
              ),
            if (present && editable)
              TextButton.icon(
                key: Key('form-clear-$path'),
                onPressed: () => form.model.remove(path),
                icon: const Icon(Icons.clear),
                label: const Text('Clear'),
              ),
          ],
        ),
      ],
    );
  }
}

class _ListOfValues extends StatefulWidget {
  const _ListOfValues(
    this.form,
    this.path,
    this.schema,
    this.items,
    this.choices,
  );

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldSchema items;
  final FieldChoices? choices;

  @override
  State<_ListOfValues> createState() => _ListOfValuesState();
}

class _ListOfValuesState extends State<_ListOfValues> {
  final _adding = TextEditingController();
  String? _refused;

  @override
  void dispose() {
    _adding.dispose();
    super.dispose();
  }

  void _addText(String text) {
    final value = text.trim();
    if (value.isEmpty) return;
    final unavailable = widget.form.choiceReason(widget.path, value);
    if (unavailable != null) {
      setState(() => _refused = unavailable);
      return;
    }
    final refused = widget.form.model.addTextToList(
      widget.path,
      value,
      at: widget.items,
    );
    setState(() => _refused = refused);
    if (refused == null) _adding.clear();
  }

  @override
  Widget build(BuildContext context) {
    final form = widget.form;
    final model = form.model;
    final path = widget.path;
    final items = widget.items;
    final held = model.read(path);
    final entries = held is List ? held : const <Object?>[];
    final closed = items.choices;
    final app = widget.choices;
    final offeredValues = <Object?>[...?closed, ...?app?.values];
    String label(Object? value) => closed != null
        ? items.labelOf(value)
        : app?.labels[value] ?? (value is String ? value : '$value');
    final theme = Theme.of(context);
    final offered = [
      for (final value in offeredValues)
        if (!entries.contains(value)) value,
    ];
    final fallback = _defaultText(widget.schema);
    final editable = form.canEdit(path, widget.schema);
    Widget withReason(Widget chip, String? reason) => reason == null
        ? chip
        : Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              chip,
              Text(
                reason,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.disabledColor,
                ),
              ),
            ],
          );
    return _Titled(
      key: Key('form-field-$path'),
      schema: widget.schema,
      problem: form.problemAt(path),
      children: [
        Wrap(
          spacing: 6,
          runSpacing: 6,
          children: [
            for (var at = 0; at < entries.length; at++)
              () {
                final reason =
                    form.choiceReason(path, entries[at]) ??
                    form.unavailable[entryPath(path, at)];
                final lock = form.removeLock(path, at, entries.length);
                return withReason(
                  InputChip(
                    key: Key('form-entry-$path-$at'),
                    label: Text(label(entries[at])),
                    tooltip: reason ?? lock,
                    onDeleted: editable && reason == null && lock == null
                        ? () => model.removeFromList(path, at)
                        : null,
                    deleteButtonTooltipMessage: 'Remove ${label(entries[at])}',
                  ),
                  reason,
                );
              }(),
            for (final value in offered)
              () {
                final reason = form.choiceReason(path, value);
                return withReason(
                  ActionChip(
                    key: Key('form-offer-$path-${label(value)}'),
                    avatar: const Icon(Icons.add, size: 16),
                    label: Text(label(value)),
                    tooltip: reason,
                    onPressed: editable && reason == null
                        ? () => model.addToList(path, value)
                        : null,
                  ),
                  reason,
                );
              }(),
          ],
        ),
        if (entries.isEmpty && fallback != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(fallback, style: theme.textTheme.bodySmall),
          ),
        if (closed == null && (app == null || app.open))
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: TextField(
              key: Key('form-add-$path'),
              controller: _adding,
              enabled: editable,
              decoration: InputDecoration(
                isDense: true,
                labelText: 'Add to ${widget.schema.title}',
                errorText: _refused,
                suffixIcon: IconButton(
                  tooltip: 'Add',
                  icon: const Icon(Icons.add),
                  onPressed: editable ? () => _addText(_adding.text) : null,
                ),
              ),
              onSubmitted: _addText,
            ),
          ),
      ],
    );
  }
}

class _ListOfGroups extends StatelessWidget {
  const _ListOfGroups(this.form, this.path, this.schema, this.items);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldSchema items;

  @override
  Widget build(BuildContext context) {
    final model = form.model;
    final ids = model.entryIds(path);
    final editable = form.canEdit(path, schema);
    return _Titled(
      key: Key('form-field-$path'),
      schema: schema,
      problem: form.problemAt(path),
      children: [
        for (var at = 0; at < ids.length; at++)
          _Entry(
            key: ValueKey('form-entry-$path-#${ids[at]}'),
            title: '${schema.title} ${at + 1}',
            reason: form.unavailable[entryPath(path, at)],
            lock: form.removeLock(path, at, ids.length),
            onRemove: editable ? () => model.removeFromList(path, at) : null,
            child: _Fields(
              form: form,
              path: entryPath(path, at),
              schema: items,
            ),
          ),
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: Key('form-add-$path'),
            onPressed: editable
                ? () => model.addToList(path, <String, Object?>{})
                : null,
            icon: const Icon(Icons.add),
            label: Text('Add to ${schema.title}'),
          ),
        ),
      ],
    );
  }
}

class _Entry extends StatelessWidget {
  const _Entry({
    required this.title,
    required this.child,
    this.onRemove,
    this.reason,
    this.lock,
    super.key,
  });

  final String title;
  final Widget child;
  final VoidCallback? onRemove;

  /// Why this entry may not be changed, shown with it.
  final String? reason;

  /// Why it may not be taken out: it, or something in it, is unavailable.
  final String? lock;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 8),
    child: DecoratedBox(
      decoration: BoxDecoration(
        border: Border(
          left: BorderSide(color: Theme.of(context).dividerColor, width: 2),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.only(left: 12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Expanded(child: Text(title)),
                IconButton(
                  tooltip: lock ?? 'Remove $title',
                  icon: const Icon(Icons.delete_outline),
                  onPressed: lock == null ? onRemove : null,
                ),
              ],
            ),
            if (reason != null)
              Text(
                reason!,
                key: Key('form-entry-reason-$title'),
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                  color: Theme.of(context).disabledColor,
                ),
              ),
            child,
          ],
        ),
      ),
    ),
  );
}

/// A box asking for a new key, for a map whose keys are data.
class _NewKey extends StatefulWidget {
  const _NewKey({required this.path, required this.label, this.onAdd});

  final String path;
  final String label;

  /// Adds the key; answers why it was refused, or `null` once added.
  final String? Function(String key)? onAdd;

  @override
  State<_NewKey> createState() => _NewKeyState();
}

class _NewKeyState extends State<_NewKey> {
  final _controller = TextEditingController();
  String? _refused;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _add() {
    final key = _controller.text.trim();
    if (key.isEmpty) return;
    final refused = widget.onAdd?.call(key);
    setState(() => _refused = refused);
    if (refused == null) _controller.clear();
  }

  @override
  Widget build(BuildContext context) => TextField(
    key: Key('form-add-${widget.path}'),
    controller: _controller,
    enabled: widget.onAdd != null,
    decoration: InputDecoration(
      isDense: true,
      labelText: widget.label,
      errorText: _refused,
      suffixIcon: IconButton(
        tooltip: 'Add',
        icon: const Icon(Icons.add),
        onPressed: widget.onAdd == null ? null : _add,
      ),
    ),
    onSubmitted: (_) => _add(),
  );
}

/// What a new entry of a map of [values] starts as.
Object? _blank(FieldSchema values) {
  if (values.hasMembers || values.isOpenMap) return <String, Object?>{};
  if (values.isList) return <Object?>[];
  if (values.isBool) {
    return values.defaultValue is bool ? values.defaultValue : false;
  }
  if (values.hasDefault) return values.defaultValue;
  if (values.isText || values.isAny) return '';
  return null;
}

class _MapOfValues extends StatelessWidget {
  const _MapOfValues(this.form, this.path, this.schema, this.values);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldSchema values;

  @override
  Widget build(BuildContext context) {
    final model = form.model;
    final editable = form.canEdit(path, schema);
    return _Titled(
      key: Key('form-field-$path'),
      schema: schema,
      problem: form.problemAt(path),
      children: [
        for (final key in model.mapKeys(path))
          Padding(
            key: ValueKey('form-entry-$path-$key'),
            padding: const EdgeInsets.only(bottom: 8),
            child: Row(
              children: [
                Expanded(child: Text(key, overflow: TextOverflow.ellipsis)),
                const SizedBox(width: 8),
                Expanded(
                  flex: 2,
                  child: _EntryText(
                    form: form,
                    path: keyedPath(path, key),
                    name: key,
                    schema: values,
                    editable:
                        editable && form.canEdit(keyedPath(path, key), values),
                    reason: form.unavailable[keyedPath(path, key)],
                  ),
                ),
                IconButton(
                  tooltip:
                      form.lockReason(keyedPath(path, key)) ?? 'Remove $key',
                  icon: const Icon(Icons.delete_outline),
                  onPressed:
                      editable && form.lockReason(keyedPath(path, key)) == null
                      ? () => model.removeFromMap(path, key)
                      : null,
                ),
              ],
            ),
          ),
        _NewKey(
          path: path,
          label: 'Add to ${schema.title}: name',
          onAdd: editable
              ? (key) => model.addToMap(path, key, _blank(values))
              : null,
        ),
      ],
    );
  }
}

class _EntryText extends StatefulWidget {
  const _EntryText({
    required this.form,
    required this.path,
    required this.name,
    required this.schema,
    required this.editable,
    this.reason,
  });

  final _FormContext form;
  final String path;
  final String name;
  final FieldSchema schema;
  final bool editable;

  /// Why this entry may not be changed, shown under it.
  final String? reason;

  @override
  State<_EntryText> createState() => _EntryTextState();
}

class _EntryTextState extends State<_EntryText> {
  // A secret is never put back in a box: an empty one keeps what is held.
  late final bool _secret = widget.schema.sensitive;
  late final _controller = TextEditingController(
    text: _secret ? '' : widget.form.model.text(widget.path) ?? '',
  );
  late final guatiao.Value? _stored = _secret
      ? widget.form.model.copyOf(widget.path)
      : null;
  late final _sync = _TextSync(_controller, _controller.text);

  @override
  void dispose() {
    _controller.dispose();
    _stored?.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.form.model;
    if (!_secret) _sync.sync(model.text(widget.path) ?? '');
    return TextField(
      key: Key('form-field-${widget.path}'),
      controller: _controller,
      enabled: widget.editable,
      obscureText: _secret,
      enableSuggestions: !_secret,
      autocorrect: !_secret,
      decoration: InputDecoration(
        isDense: true,
        labelText: 'Value of ${widget.name}',
        helperText: widget.reason,
        hintText: _secret && _stored != null ? 'Set; type to replace it' : null,
        errorText: widget.form.problemAt(widget.path),
      ),
      // An emptied entry is still an entry: it keeps its key.
      onChanged: (text) {
        _sync.typed();
        if (text.isNotEmpty) {
          model.setText(widget.path, text, at: widget.schema);
        } else if (_stored case final stored?) {
          model.setValue(widget.path, stored);
        } else {
          model.setValue(widget.path, _blank(widget.schema));
        }
      },
    );
  }
}

class _MapOfGroups extends StatelessWidget {
  const _MapOfGroups(this.form, this.path, this.schema, this.values);

  final _FormContext form;
  final String path;
  final FieldSchema schema;
  final FieldSchema values;

  @override
  Widget build(BuildContext context) {
    final model = form.model;
    final editable = form.canEdit(path, schema);
    return _Titled(
      key: Key('form-field-$path'),
      schema: schema,
      problem: form.problemAt(path),
      children: [
        for (final key in model.mapKeys(path))
          _Entry(
            key: ValueKey('form-entry-$path-$key'),
            title: key,
            reason: form.unavailable[keyedPath(path, key)],
            lock: form.lockReason(keyedPath(path, key)),
            onRemove: editable ? () => model.removeFromMap(path, key) : null,
            child: _Fields(
              form: form,
              path: keyedPath(path, key),
              schema: values,
            ),
          ),
        _NewKey(
          path: path,
          label: 'Add to ${schema.title}: name',
          onAdd: editable
              ? (key) => model.addToMap(path, key, <String, Object?>{})
              : null,
        ),
      ],
    );
  }
}
