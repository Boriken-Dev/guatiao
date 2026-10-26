// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The trusted level's proof of a pre-shared key: HMAC-SHA256 over a label
// and both nonces, each side proving it to the other.

import 'dart:convert';
import 'dart:math';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';

/// What a server proves with. Protocol bytes, never renamed.
const String serverLabel = 'guatiao-remote server';

/// What a client proves with.
const String clientLabel = 'guatiao-remote client';

/// The proof of [key] for [label] over the client's nonce and the
/// server's.
Uint8List proofOf(
  List<int> key,
  String label,
  List<int> client,
  List<int> server,
) {
  final mac = Hmac(sha256, key);
  final digest = mac.convert([...utf8.encode(label), ...client, ...server]);
  return Uint8List.fromList(digest.bytes);
}

/// Whether [given] is the proof, compared in time that does not depend on
/// where it first differs.
bool proves(
  List<int> key,
  String label,
  List<int> client,
  List<int> server,
  List<int> given,
) {
  final expected = proofOf(key, label, client, server);
  if (given.length != expected.length) return false;
  var diff = 0;
  for (var i = 0; i < expected.length; i++) {
    diff |= expected[i] ^ given[i];
  }
  return diff == 0;
}

/// Sixteen bytes from the platform's secure random source.
Uint8List nonce() {
  final random = Random.secure();
  return Uint8List.fromList([for (var i = 0; i < 16; i++) random.nextInt(256)]);
}
