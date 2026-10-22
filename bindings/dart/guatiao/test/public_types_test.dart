// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Every type in the public signatures is reachable from
/// `package:guatiao/guatiao.dart` alone: this file imports nothing else
/// from the package.
library;

import 'dart:ffi';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('the pointer types the public API names can be written out', () {
    final value = Value.fromDart({'a': 1});
    addTearDown(value.close);

    final Pointer<guatiao_value> root = value.pointer;
    final Pointer<guatiao_alloc> allocator = value.allocator;
    expect(root, isNot(nullptr));
    expect(allocator, isNot(nullptr));

    // And they go back in: a borrowed view, and a value built in an
    // allocator the caller names.
    expect(Ref.borrowed(root).asMap()['a']!.toDart(), 1);
    final again = Value.fromDart([1], alloc: allocator);
    addTearDown(again.close);
    expect(again.toDart(), [1]);
  });
}
