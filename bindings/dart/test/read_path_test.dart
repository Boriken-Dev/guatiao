// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The reader keeps no top-level state, checked in the source.
///
/// A behavioural test cannot hold this line. In an `isolateGroupBound`
/// callback a top-level `final` here read as 0 after an earlier callback
/// had read it as 72, in a consumer's suite and only in one test order;
/// no standalone program reproduced it. So the rule is checked where it
/// is written: `value.dart` declares nothing at the top level but a
/// `const`.
library;

import 'dart:io';

import 'package:test/test.dart';

/// A declaration at the start of a line that is not a `const`: a `final`,
/// a `late`, a `var`, or a typed variable with an initialiser.
final _topLevelVariable = RegExp(
  r'^(?:final|late|var)\b|^(?!const\b)[A-Za-z_][\w<>?, ]*\s+_?\w+\s*=[^=>]',
);

void main() {
  test('the read path declares no top-level state', () {
    final lines = File('lib/src/value.dart').readAsLinesSync();
    final offending = <String>[
      for (var i = 0; i < lines.length; i++)
        if (_topLevelVariable.hasMatch(lines[i])) '${i + 1}: ${lines[i]}',
    ];
    expect(offending, isEmpty,
        reason: 'value.dart must declare only consts at the top level');
  });

  test('the check sees a top-level final when there is one', () {
    expect(_topLevelVariable.hasMatch('final int _x = sizeOf<T>();'), isTrue);
    expect(_topLevelVariable.hasMatch('int _count = 0;'), isTrue);
    expect(_topLevelVariable.hasMatch('const Absent absent = Absent._();'),
        isFalse);
    expect(_topLevelVariable.hasMatch('  final x = 1;'), isFalse,
        reason: 'indented is local');
  });
}
