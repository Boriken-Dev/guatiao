// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The struct sizes are the ABI; a wrong one is silent corruption.
///
/// The tag and status numbers are read straight out of the committed
/// header, so the Dart enums cannot drift from it unnoticed.
library;

import 'dart:ffi';
import 'dart:io';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/bindings.g.dart';
import 'package:test/test.dart';

import 'support.dart';

const _tagNames = <Tag, String>{
  Tag.absent: 'GUATIAO_ABSENT',
  Tag.nullValue: 'GUATIAO_NULL',
  Tag.boolean: 'GUATIAO_BOOL',
  Tag.number: 'GUATIAO_NUMBER',
  Tag.string: 'GUATIAO_STRING',
  Tag.bytes: 'GUATIAO_BYTES',
  Tag.list: 'GUATIAO_LIST',
  Tag.map: 'GUATIAO_MAP',
};

const _statusNames = <Status, String>{
  Status.ok: 'GUATIAO_OK',
  Status.badValue: 'GUATIAO_ERR_BAD_VALUE',
  Status.allocFailed: 'GUATIAO_ERR_ALLOC',
  Status.wrongKind: 'GUATIAO_ERR_WRONG_KIND',
  Status.notFound: 'GUATIAO_ERR_NOT_FOUND',
  Status.nullArgument: 'GUATIAO_ERR_NULL',
  Status.gone: 'GUATIAO_ERR_GONE',
  Status.unsupported: 'GUATIAO_ERR_UNSUPPORTED',
  Status.busy: 'GUATIAO_ERR_BUSY',
  Status.internalError: 'GUATIAO_ERR_INTERNAL',
};

Map<String, int> _headerConstants() {
  final text = File(
    repoPath(['crates', 'guatiao', 'include', 'guatiao.h']),
  ).readAsStringSync();
  final pattern = RegExp(r'^\s*GUATIAO_([A-Z_0-9]+) = (\d+),', multiLine: true);
  return <String, int>{
    for (final match in pattern.allMatches(text))
      'GUATIAO_${match.group(1)}': int.parse(match.group(2)!),
  };
}

void main() {
  test('view and owned sizes are what the crate pins', () {
    expect(sizeOf<guatiao_str>(), 16);
    expect(sizeOf<guatiao_bytes>(), 16);
    expect(sizeOf<guatiao_values>(), 16);
    expect(sizeOf<guatiao_entries>(), 16);
    expect(sizeOf<guatiao_string>(), 32);
    expect(sizeOf<guatiao_buffer>(), 32);
    expect(sizeOf<guatiao_list>(), 32);
    expect(sizeOf<guatiao_map>(), 32);
    expect(sizeOf<guatiao_payload>(), 32);
    expect(sizeOf<guatiao_value>(), 40);
    expect(sizeOf<guatiao_entry>(), 72);
  });

  test('the offsets the readers do arithmetic with hold', () {
    // `guatiao_value.payload` sits at 8 and `guatiao_entry.value` at 32;
    // neither field's offset is readable from Dart, so each is pinned by
    // the sizes around it.
    expect(sizeOf<guatiao_value>() - sizeOf<guatiao_payload>(), 8);
    expect(sizeOf<guatiao_entry>() - sizeOf<guatiao_value>(), 32);
    expect(sizeOf<guatiao_string>(), 32);
  });

  test('every tag matches the committed header', () {
    final constants = _headerConstants();
    for (final entry in _tagNames.entries) {
      expect(constants[entry.value], entry.key.code, reason: entry.value);
    }
  });

  test('every status matches the committed header', () {
    final constants = _headerConstants();
    for (final entry in _statusNames.entries) {
      expect(constants[entry.value], entry.key.code, reason: entry.value);
    }
  });

  test('the header declares no status this binding is missing', () {
    final constants = _headerConstants();
    final statuses = _statusNames.values.toSet();
    for (final name in constants.keys) {
      if (name == 'GUATIAO_OK' || name.startsWith('GUATIAO_ERR_')) {
        expect(statuses, contains(name), reason: name);
      }
    }
  });

  test('every tag this binding names is declared in the header', () {
    final constants = _headerConstants();
    for (final name in _tagNames.values) {
      expect(constants, contains(name), reason: name);
    }
  });
}
