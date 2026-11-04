// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A view `at()` answers borrows from its value, so a call cannot lend the
/// value and borrow the view: the callee would get a `&mut` and a `&` into
/// one tree.
@TestOn('vm')
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kind_glue.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
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

  test('refuses a value lent beside a view at() borrows from it', () {
    final v = Value({
      'a': {'k': 'v'},
    });
    addTearDown(v.close);
    final view = intake.at(v, 'a')!;
    _inOneCall((c) {
      c.lendMap(v.asMap());
      expect(() => c.borrowMap(view.asMap()), throwsStateError);
    });
    _inOneCall((c) {
      c.borrowMap(view.asMap());
      expect(() => c.lendMap(v.asMap()), throwsStateError);
      expect(() => v.asMap()['more'] = 1, throwsStateError);
    });
    v.asMap()['after'] = 1;
  });
}
