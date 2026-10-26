// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `dart run guatiao:kind DOCUMENT -o FILE [--import KIND=URI]... [--check]`
/// writes the Dart for a document of kinds, the `<crate>.kinds.json` a kind
/// crate commits beside its C header; `--import` names an object kind a slot
/// takes or answers that another document describes, with the library
/// written for that document.
///
/// `dart run guatiao:kind identity [-o FILE] [--pubspec FILE] [--check]`
/// writes this package's library identity from its `pubspec.yaml`
/// (default `lib/src/library_identity.g.dart`).
///
/// `dart run guatiao:kind describe FILE.dart -o DOCUMENT [--check]` writes
/// the document of the kinds a Dart file declares
/// (`package:guatiao/describe.dart`), which the first form then reads.
///
/// `--check` writes nothing and exits 1 when the file is not what it would
/// write.
library;

import 'dart:convert';
import 'dart:io';

import 'package:guatiao/src/kind_describer.dart';
import 'package:guatiao/src/kind_generator.dart';

const _usage =
    'usage: dart run guatiao:kind DOCUMENT -o FILE [--import KIND=URI]... [--check]\n'
    '       dart run guatiao:kind identity [-o FILE] [--pubspec FILE] [--check]\n'
    '       dart run guatiao:kind describe FILE.dart -o DOCUMENT [--check]';

List<String> _segments(String path) =>
    File(path).absolute.uri.pathSegments.where((s) => s.isNotEmpty).toList();

/// [to] from the directory [from], with forward slashes.
String _relative(List<String> from, String to) {
  final b = _segments(to);
  var i = 0;
  while (i < from.length &&
      i < b.length &&
      from[i].toLowerCase() == b[i].toLowerCase()) {
    i++;
  }
  return [for (var j = i; j < from.length; j++) '..', ...b.skip(i)].join('/');
}

/// The directory of [file]'s package: the nearest one above it holding a
/// `pubspec.yaml`, else its own.
List<String> _packageOf(String file) {
  var dir = File(file).absolute.parent;
  while (!File('${dir.path}/pubspec.yaml').existsSync()) {
    if (dir.parent.path == dir.path) return _segments(file)..removeLast();
    dir = dir.parent;
  }
  return _segments('${dir.path}/pubspec.yaml')..removeLast();
}

void main(List<String> argv) {
  final args = [...argv];
  String? option(String name) {
    final at = args.indexOf(name);
    if (at < 0) return null;
    if (at + 1 >= args.length) {
      stderr.writeln(_usage);
      exit(2);
    }
    final value = args[at + 1];
    args.removeRange(at, at + 2);
    return value;
  }

  final check = args.remove('--check');
  final kindImports = <String, String>{};
  for (String? one; (one = option('--import')) != null;) {
    final at = one!.indexOf('=');
    if (at <= 0 || at == one.length - 1) {
      stderr.writeln('guatiao:kind: --import is KIND=URI, not $one\n$_usage');
      exit(2);
    }
    kindImports[one.substring(0, at)] = one.substring(at + 1);
  }
  final out = option('-o') ?? option('--out');
  final pubspec = option('--pubspec');
  final describing = args.length == 2 && args.first == 'describe';
  if (describing) args.removeAt(0);
  if (args.length != 1) {
    stderr.writeln(_usage);
    exit(2);
  }
  final String target;
  final String text;
  try {
    if (describing) {
      if (out == null) {
        stderr.writeln(_usage);
        exit(2);
      }
      target = out;
      text = describedText(
        describeDeclarations(
          File(args.single).readAsStringSync(),
          file: args.single,
        ),
      );
    } else if (args.single == 'identity') {
      target = out ?? 'lib/src/library_identity.g.dart';
      final from = pubspec ?? 'pubspec.yaml';
      final here = _segments(target)..removeLast();
      final root = _packageOf(target);
      text = generateIdentity(
        File(from).readAsStringSync(),
        source: _relative(here, from),
        command: [
          'dart run guatiao:kind identity',
          if (out != null) '-o ${_relative(root, target)}',
          if (pubspec != null) '--pubspec ${_relative(root, from)}',
        ].join(' '),
      );
    } else {
      if (out == null) {
        stderr.writeln(_usage);
        exit(2);
      }
      target = out;
      final document = jsonDecode(File(args.single).readAsStringSync());
      final here = _segments(target)..removeLast();
      final root = _packageOf(target);
      text = generateKinds(
        (document as Map).cast<String, Object?>(),
        source: _relative(here, args.single),
        command: [
          'dart run guatiao:kind ${_relative(root, args.single)}',
          '-o ${_relative(root, target)}',
          for (final MapEntry(:key, :value) in kindImports.entries)
            '--import $key=$value',
        ].join(' '),
        imports: kindImports,
      );
    }
  } on Object catch (e) {
    stderr.writeln('guatiao:kind: $e');
    exit(1);
  }
  final file = File(target);
  if (check) {
    if (!file.existsSync() || file.readAsStringSync() != text) {
      stderr.writeln('guatiao:kind: $target is not what it would write');
      exit(1);
    }
    return;
  }
  file.parent.createSync(recursive: true);
  file.writeAsStringSync(text);
}
