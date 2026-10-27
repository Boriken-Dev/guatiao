// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import '../status.dart';
import 'module.dart';
import 'value_web.dart';

/// The floor hash of the kind whose description [kind] holds as JSON; see
/// the VM's `kindFloorHash`, which this matches.
int kindFloorHash(String kind) => withScratch((s) {
  final m = s.module;
  final out = s.alloc(4, 4);
  final why = s.value();
  try {
    final status = m.call('guatiao_kind_floor_hash', [
      s.str(kind),
      m.defaultAlloc,
      out,
      why,
    ]);
    checkStatus(status, Ref.borrowed(why).stringOr(''));
    return m.u32(out);
  } finally {
    m.call('guatiao_value_free', [why]);
  }
});
