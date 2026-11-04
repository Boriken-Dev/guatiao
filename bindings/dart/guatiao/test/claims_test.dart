// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// One call never has a value lent and borrowed, or lent twice: the
/// callee would get a `&mut` and another reference onto one tree.
@TestOn('vm')
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kind_glue.dart';
import 'package:test/test.dart';

import 'support.dart';

/// Runs [build] over one call's scope, closed whatever it threw.
void _inOneCall(void Function(CallScope c) build) {
  final c = CallScope();
  try {
    build(c);
  } finally {
    c.close();
  }
}

void main() {
  setUpAll(setUpLibrary);

  test('refuses a value lent and borrowed by one call, in either order', () {
    final v = Value({'k': 'v'});
    addTearDown(v.close);
    _inOneCall((c) {
      c.lendMap(v.asMap());
      expect(() => c.borrowMap(v.asMap()), throwsStateError);
      expect(() => c.borrowValue(v), throwsStateError);
    });
    _inOneCall((c) {
      c.borrowMap(v.asMap());
      expect(() => c.lendMap(v.asMap()), throwsStateError);
      expect(() => c.lendValue(v), throwsStateError);
    });
    _inOneCall((c) {
      c.lendValue(v);
      expect(() => c.lendMap(v.asMap()), throwsStateError);
    });
    // Every mark the calls set is lifted: it changes and frees.
    v.asMap()['after'] = 1;
    expect(v.toDart(), {'k': 'v', 'after': 1});
  });

  test('refuses a node of a value lent while the value is borrowed', () {
    final v = Value({
      'inner': {'k': 'v'},
    });
    addTearDown(v.close);
    _inOneCall((c) {
      c.lendMap(v.asMap()['inner']!.asMap());
      expect(() => c.borrowValue(v), throwsStateError);
    });
    _inOneCall((c) {
      c.borrowValue(v);
      expect(() => c.lendMap(v.asMap()['inner']!.asMap()), throwsStateError);
    });
  });

  test('lets one value be borrowed twice, and two be lent and borrowed', () {
    final a = Value({'k': 'v'});
    final b = Value({'x': 1});
    addTearDown(a.close);
    addTearDown(b.close);
    _inOneCall((c) {
      c.borrowMap(b.asMap());
      c.borrowValue(b);
      c.lendMap(a.asMap());
      c.borrowMap(b.asMap());
    });
  });

  test('refuses a change or a free of a value while a call borrows it', () {
    final v = Value({'k': 'v'});
    addTearDown(v.close);
    _inOneCall((c) {
      c.borrowMap(v.asMap());
      expect(() => v.asMap()['more'] = 1, throwsStateError);
      expect(v.close, throwsStateError);
      expect(v.toDart(), {'k': 'v'});
    });
    v.asMap()['more'] = 1;
  });
}
