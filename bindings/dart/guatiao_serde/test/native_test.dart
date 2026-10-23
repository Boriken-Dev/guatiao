// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What only a native build has: the library file serde opened, the
/// parse error's layout, and the constants against the generated bindings.
@TestOn('vm')
library;

import 'dart:ffi';

import 'package:ffi/ffi.dart';
import 'package:guatiao/ffi.dart' show openSurface, strOf;
import 'package:guatiao/guatiao.dart';
import 'package:guatiao_serde/guatiao_serde.dart' as serde;
import 'package:guatiao_serde/src/bindings.g.dart'
    hide guatiao_alloc, guatiao_str, guatiao_value;
import 'package:guatiao_serde/src/serde.dart' as inner;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('the library is serde\'s own, and carries the core too', () {
    final lib = serde.library();
    expect(
        lib.path, namedLibraryFile() ?? endsWith(dllFilename('guatiao_serde')));
    expect(lib.provides('guatiao_value_free'), isTrue);
  });

  test('a parse error is laid out as the header says', () {
    // Three uint32_t, padding, then the message value the reader strides to.
    expect(sizeOf<guatiao_parse_error>(), 56);
  });

  test('a refusal that is not a syntax error keeps its message', () {
    final error = calloc<guatiao_parse_error>();
    addTearDown(() => calloc.free(error));
    final core = openSurface('guatiao');
    // `message` is the last field of the struct.
    final message = (error.cast<Uint8>() +
            (sizeOf<guatiao_parse_error>() - sizeOf<guatiao_value>()))
        .cast<guatiao_value>();
    using((arena) {
      expect(
        core.bindings.guatiao_value_string(
          core.alloc,
          strOf(arena, 'the library words this itself').ref,
          message,
        ),
        0,
      );
    });
    expect(
      () => inner.finishParse(Status.wrongKind.code, error),
      throwsA(
        isA<WrongKind>().having(
          (e) => e.message,
          'message',
          'the library words this itself',
        ),
      ),
    );
  });

  test('the flags are the header\'s', () {
    expect(serde.bytesDataUri, GUATIAO_BYTES_DATA_URI);
    expect(serde.bytesBase64, GUATIAO_BYTES_BASE64);
    expect(serde.bytesArray, GUATIAO_BYTES_ARRAY);
    expect(serde.bytesRefuse, GUATIAO_BYTES_REFUSE);
    expect(serde.readDataUris, GUATIAO_READ_DATA_URIS);
    expect(serde.prettyFlag, GUATIAO_PRETTY);
  });
}
