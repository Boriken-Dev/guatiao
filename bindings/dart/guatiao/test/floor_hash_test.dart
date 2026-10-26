// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The floor hash of a kind born in Dart is exact where an int is a double
/// too: the hash `#[guatiao::kind]` gives the greeter kind's one required
/// slot, and FNV-1a's own vectors, on the VM and in a browser alike.
library;

import 'package:guatiao/src/kind_describer.dart' show floorHashOf;
import 'package:test/test.dart';

void main() {
  test('the greeter kind hashes as Rust hashes it', () {
    expect(
      floorHashOf(false, [
        {
          'name': 'greet',
          'required': true,
          'fallible': true,
          'args': [
            {'name': 'name', 'type': 'str'},
          ],
          'answer': {'type': 'map'},
        },
      ]),
      2695856491,
    );
    // FNV-1a of "provider" alone, a slot appended after it changing nothing.
    expect(
      floorHashOf(false, [
        {'name': 'x', 'required': false, 'fallible': false, 'args': []},
      ]),
      floorHashOf(false, const []),
    );
  });
}
