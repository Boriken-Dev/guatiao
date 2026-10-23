// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'status.dart';

export 'status.dart';

/// Throws unless `code` is `GUATIAO_OK`, taking the provider's own words
/// from `err` first and freeing the owned string it built.
///
/// `err` is read and released whatever the status is, because a provider
/// may leave a message behind on a call that then reported success.
void checkProviderStatus(
  GuatiaoBindings bindings,
  int code,
  Pointer<guatiao_provider_error> err,
) {
  var message = '';
  final owned = err.ref.message;
  if (owned.len != 0 && owned.ptr != nullptr) {
    message = utf8.decode(
      Uint8List.fromList(owned.ptr.asTypedList(owned.len)),
      allowMalformed: true,
    );
  }
  if (owned.cap != 0) {
    // The message is an owned `guatiao_string`; freeing it means handing
    // the value model a string-tagged node that borrows those 32 bytes.
    final wrapper = calloc<guatiao_value>();
    try {
      wrapper.ref.tag = 4; // GUATIAO_STRING
      wrapper.ref.payload.text
        ..ptr = owned.ptr
        ..len = owned.len
        ..cap = owned.cap
        ..alloc = owned.alloc;
      bindings.guatiao_value_free(wrapper);
    } finally {
      calloc.free(wrapper);
    }
    err.ref.message
      ..ptr = nullptr
      ..len = 0
      ..cap = 0;
  }
  checkStatus(code, message);
}
