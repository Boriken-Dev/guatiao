// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The floor hash of a kind born in Dart comes from the library, on the
/// VM and in a browser alike: the hash `#[guatiao::kind]` gives a slot,
/// an optional slot hashed with the rest, and a refusal naming where.
library;

import 'package:guatiao/guatiao.dart' show BadValue;
import 'package:guatiao/src/kind_describer.dart' show floorHashOf;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(setUpLibrary);

  const greet = {
    'name': 'greet',
    'required': true,
    'fallible': true,
    'args': [
      {'name': 'name', 'type': 'str'},
    ],
    'answer': {'type': 'map'},
  };

  test('the greeter kind hashes as Rust hashes it', () {
    expect(floorHashOf(false, [greet]), 2695856491);
  });

  test('an optional slot is hashed with the rest', () {
    final shout = {
      'name': 'shout',
      'required': false,
      'fallible': false,
      'args': [
        {'name': 'name', 'type': 'str'},
      ],
      'answer': {'type': 'text'},
    };
    expect(floorHashOf(false, [greet, shout]), 1283548087);
    expect(floorHashOf(false, const []), isNot(floorHashOf(false, [shout])));
  });

  test('slots that are not a description are refused, naming where', () {
    expect(
      () => floorHashOf(false, [
        {
          'name': 'x',
          'required': true,
          'fallible': false,
          'args': [
            {'name': 'a', 'type': 'u256'},
          ],
        },
      ]),
      throwsA(
        isA<BadValue>().having(
          (e) => e.toString(),
          'message',
          contains('slots[0].args[0].type'),
        ),
      ),
    );
  });
}
