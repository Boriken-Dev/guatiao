// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Paths in guatiao_intake's grammar (`agent[1].name`, `env[PATH]`,
/// `env["a.b"]`), spelled as that grammar writes them.
library;

/// Whether [name] can be written as a member segment as it stands: not
/// empty, and holding no `.`, `[` or `]`.
bool isBareName(String name) =>
    name.isNotEmpty &&
    !name.contains('.') &&
    !name.contains('[') &&
    !name.contains(']');

/// Whether a bracket key written bare would read back as something else:
/// empty, a leading `"`, a bracket, or a position (digits, no leading zero).
bool needsQuoting(String key) =>
    key.isEmpty ||
    key.startsWith('"') ||
    key.contains('[') ||
    key.contains(']') ||
    _positionOf(key) != null;

int? _positionOf(String text) {
  if (text.isEmpty || !RegExp(r'^[0-9]+$').hasMatch(text)) return null;
  if (text.length > 1 && text.startsWith('0')) return null;
  return int.tryParse(text);
}

String _bracket(String key) => needsQuoting(key)
    ? '["${key.replaceAll(r'\', r'\\').replaceAll('"', r'\"')}"]'
    : '[$key]';

/// The path of the member [key] of what is at [owner] (`''` for the root).
String memberPath(String owner, String key) {
  if (isBareName(key)) return owner.isEmpty ? key : '$owner.$key';
  return '$owner${_bracket(key)}';
}

/// The path of the entry at [position] of the list at [owner].
String entryPath(String owner, int position) => '$owner[$position]';

/// The path of the entry under [key] of the map at [owner].
String keyedPath(String owner, String key) => '$owner${_bracket(key)}';

/// Whether [path] is [owner] or lies under it.
bool isAtOrUnder(String path, String owner) =>
    path == owner ||
    (owner.isEmpty) ||
    path.startsWith('$owner.') ||
    path.startsWith('$owner[');

/// One step of a path: a member or map key (`String`), or a bracket that
/// may be a position (`int`) or the key its digits spell.
sealed class Step {
  const Step();
}

/// A member, or a quoted or non-numeric bracket key.
final class KeyStep extends Step {
  const KeyStep(this.key);
  final String key;
}

/// A bracket holding a position; on a map it is the key its digits spell.
final class PositionStep extends Step {
  const PositionStep(this.position);
  final int position;
  String get key => '$position';
}

/// The steps of [path]; throws [FormatException] for text off the grammar.
List<Step> parsePath(String path) {
  final steps = <Step>[];
  var at = 0;
  var first = true;
  while (at < path.length) {
    final c = path[at];
    if (c == '[') {
      at += 1;
      if (at < path.length && path[at] == '"') {
        at += 1;
        final key = StringBuffer();
        while (true) {
          if (at >= path.length) {
            throw FormatException('an unclosed quote', path, at);
          }
          final d = path[at];
          if (d == '"') break;
          if (d == r'\') {
            at += 1;
            if (at >= path.length) {
              throw FormatException('a bad escape', path, at);
            }
          }
          key.write(path[at]);
          at += 1;
        }
        at += 1;
        if (at >= path.length || path[at] != ']') {
          throw FormatException('text after a quoted key', path, at);
        }
        steps.add(KeyStep(key.toString()));
      } else {
        final end = path.indexOf(']', at);
        if (end < 0) throw FormatException('an unclosed bracket', path, at);
        final text = path.substring(at, end);
        if (text.isEmpty || text.contains('[')) {
          throw FormatException('a bad bracket', path, at);
        }
        final position = _positionOf(text);
        steps.add(position == null ? KeyStep(text) : PositionStep(position));
        at = end;
      }
      at += 1;
    } else {
      if (!first) {
        if (c != '.') throw FormatException('a missing separator', path, at);
        at += 1;
      }
      var end = at;
      while (end < path.length && !'.[]'.contains(path[end])) {
        end += 1;
      }
      if (end == at) throw FormatException('an empty member', path, at);
      steps.add(KeyStep(path.substring(at, end)));
      at = end;
    }
    first = false;
  }
  return steps;
}
