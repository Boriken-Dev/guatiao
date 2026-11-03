// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A secret in Dart: built in the wiping allocator, never shown by
/// `toString`, read as a view over the tree's own memory, wiped on
/// demand, and answered by a slot its kind marks `@sensitive` as a
/// [Value] kept there rather than a `String`.
///
/// That the wiping allocator overwrites what it frees is the library's,
/// and is proven by its own tests; these check that Dart puts a secret
/// there and keeps it there.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/kind_describer.dart';
import 'package:guatiao/src/kind_generator.dart';
import 'package:guatiao/src/kind_runtime.dart' show hold, release;
import 'package:guatiao/src/library.dart' as native;
import 'package:test/test.dart';

import 'fixtures/vault_kind.dart';
import 'support.dart';

const _secret = 'hunter2-SECRET';

bool _livesInWiping(Pointer<guatiao_value> node) {
  // Every owning arm is `{ptr, len, cap, alloc}`, after the tag and its
  // padding.
  final recorded = (node.cast<Uint8>() + 8 + 3 * sizeOf<IntPtr>())
      .cast<Pointer<guatiao_alloc>>()
      .value;
  return native.core().bindings.guatiao_alloc_is_wiping(recorded);
}

/// Blocks [_watching] freed while they still held the secret.
int _held = 0;

Pointer<Void> _watchAlloc(Pointer<Void> ctx, int size, int align) =>
    malloc<Uint8>(size).cast();

void _watchFree(Pointer<Void> ctx, Pointer<Void> ptr, int size, int align) {
  if (ptr != nullptr) {
    final text = latin1.decode(ptr.cast<Uint8>().asTypedList(size));
    if (text.contains(_secret)) _held++;
  }
  malloc.free(ptr);
}

/// `malloc`, looking at every block as it is freed.
final Pointer<guatiao_alloc> _watching = () {
  final a = calloc<guatiao_alloc>();
  a.ref.struct_size = sizeOf<guatiao_alloc>();
  a.ref.alloc = Pointer.fromFunction(_watchAlloc);
  a.ref.free = Pointer.fromFunction(_watchFree);
  return a;
}();

final class Keeper implements DartVault {
  @override
  Object? read(String key) => {'key': key, 'secret': _secret};

  @override
  Object every() => {'one': _secret};

  @override
  Object pin() => _secret;

  @override
  String label() => 'keeper';
}

/// The bytes a [ref] holds, read through its view and compared there.
bool _holds(Ref ref, String text) =>
    ref.reveal((bytes) => _same(bytes, utf8.encode(text)));

bool _same(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

void main() {
  test('the wiping allocator is the library\'s and the default is not', () {
    final core = native.core();
    expect(core.bindings.guatiao_alloc_is_wiping(core.wipingAlloc), isTrue);
    expect(core.bindings.guatiao_alloc_is_wiping(core.alloc), isFalse);
    expect(identical(core.wipingAlloc, core.wipingAlloc), isTrue);
  });

  test('a value built as a secret is never shown', () {
    final secret = Value.sensitive({'user': 'alice', 'password': _secret});
    addTearDown(secret.close);
    expect(secret.isSensitive, isTrue);
    expect(_livesInWiping(secret.pointer), isTrue);
    expect(secret.toString(), 'Value([sensitive])');
    expect(secret.toString(), isNot(contains('hunter2')));
    final copy = Value.fromDart(secret);
    addTearDown(copy.close);
    final plain = Value({'user': 'alice'});
    addTearDown(plain.close);
    expect(plain.isSensitive, isFalse);
    expect(plain.toString(), contains('alice'));
  });

  test('a secret is read where it lives, through a view that cannot write', () {
    final secret = Value.sensitive(_secret);
    addTearDown(secret.close);
    final at = secret.pointer.ref.payload.text.ptr.address;
    secret.reveal((bytes) {
      expect(bytes.length, _secret.length);
      expect(_same(bytes, utf8.encode(_secret)), isTrue);
      expect(() => bytes[0] = 0, throwsUnsupportedError);
      // The tree's own bytes, not a copy: a byte changed where they live
      // reads back changed.
      final raw = Pointer<Uint8>.fromAddress(at);
      final was = raw.value;
      raw.value = 0x58;
      expect(bytes[0], 0x58);
      raw.value = was;
    });
    expect(() => Value({'a': 1}).reveal((_) {}), throwsStateError);
  });

  test('wipe overwrites a tree in an allocator that does not', () {
    _held = 0;
    Value.fromDart({'password': _secret, _secret: 1}, alloc: _watching).close();
    expect(_held, greaterThanOrEqualTo(2), reason: 'the control');
    _held = 0;
    final secret = Value.fromDart({
      'password': _secret,
      _secret: 1,
    }, alloc: _watching);
    secret.wipe();
    expect(_held, 0);
    expect(secret.isNull, isTrue);
    secret.close();
  });

  test('a secret copied into a plain tree stays in the wiping allocator', () {
    final secret = Value.sensitive(_secret);
    addTearDown(secret.close);
    final plain = Value({'user': 'alice'});
    addTearDown(plain.close);
    plain.asMap()['token'] = secret;
    final items = Value(['a']);
    addTearDown(items.close);
    items.asList()[0] = secret;
    final built = Value.fromDart(secret);
    addTearDown(built.close);
    for (final holder in [plain, items, built]) {
      expect(holder.isSensitive, isTrue, reason: '$holder');
      expect(holder.toString(), isNot(contains('hunter2')));
    }
  });

  test(
    'revealed bytes are read only during the call, which the value waits for',
    () {
      final v = Value.sensitive({'token': _secret, 'user': 'alice'});
      late List<int> kept;
      v.asMap()['token']!.reveal((bytes) {
        kept = bytes;
        expect(bytes.length, _secret.length);
        expect(() => v.close(), throwsStateError);
        expect(() => v.wipe(), throwsStateError);
        expect(() => v.asMap()['more'] = 'x', throwsStateError);
        expect(() => v.asMap().remove('user'), throwsStateError);
      });
      expect(() => kept[0], throwsStateError);
      expect(() => kept.length, throwsStateError);
      v.asMap()['more'] = 'x';
      v.close();
    },
  );

  group('a slot its kind marks sensitive', () {
    late DartVaultRemote vault;
    late int ctx;
    setUp(() {
      final keeper = Keeper();
      ctx = hold(keeper);
      vault = DartVault.kind.remote(
        DartVault.kind.tableFor(keeper),
        DartVault.kind.tableSize,
        Pointer<Void>.fromAddress(ctx),
      );
    });
    tearDown(() => release(ctx));

    test('answers a tree kept in the wiping allocator', () {
      final read = vault.read('k');
      addTearDown(read.close);
      expect(read.isSensitive, isTrue);
      expect(_livesInWiping(read.pointer), isTrue);
      expect(read.toString(), isNot(contains('hunter2')));
      expect(_holds(read.asMap()['secret']!, _secret), isTrue);
      final every = vault.every();
      addTearDown(every.close);
      expect(every.isSensitive, isTrue);
    });

    test('answers text as a Value, never a String', () {
      final Value pin = vault.pin();
      addTearDown(pin.close);
      expect(pin.isSensitive, isTrue);
      expect(_holds(pin, _secret), isTrue);
      expect(vault.label(), 'keeper');
    });
  });

  test('the description marks the sensitive slots, and the hash does not', () {
    final declarations = File(
      repoPath([
        'bindings',
        'dart',
        'guatiao',
        'test',
        'fixtures',
        'vault_declarations.dart',
      ]),
    ).readAsStringSync();
    final described = describeDeclarations(declarations);
    final committed =
        jsonDecode(
              File(
                repoPath([
                  'bindings',
                  'dart',
                  'guatiao',
                  'test',
                  'fixtures',
                  'vault.kinds.json',
                ]),
              ).readAsStringSync(),
            )
            as Map<String, Object?>;
    expect(described, committed);
    final slots = ((committed['kinds']! as List).first as Map)['slots'] as List;
    expect(
      [for (final s in slots) (s as Map)['sensitive'] == true],
      [true, true, true, false],
    );
    final unmarked = declarations.replaceAll('  @sensitive\n', '');
    final plain = describeDeclarations(unmarked);
    int hashOf(Map<String, Object?> d) =>
        ((d['kinds']! as List).first as Map)['floor_hash'] as int;
    expect(hashOf(plain), hashOf(committed));
    expect(jsonEncode(plain), isNot(contains('sensitive')));
    // Committed formatted, as the package's format check wants it.
    final generated = generateKinds(
      committed,
      source: 'vault.kinds.json',
      command:
          'dart run guatiao:kind test/fixtures/vault.kinds.json -o '
          'test/fixtures/vault_kind.dart',
    );
    final scratch = Directory.systemTemp.createTempSync('vault_kind');
    addTearDown(() => scratch.deleteSync(recursive: true));
    final written = File('${scratch.path}/vault_kind.dart')
      ..writeAsStringSync(generated);
    final formatted = Process.runSync(Platform.resolvedExecutable, [
      'format',
      '--output=show',
      written.path,
    ]);
    expect(formatted.exitCode, 0, reason: '${formatted.stderr}');
    expect(
      (formatted.stdout as String).replaceAll('\r\n', '\n'),
      startsWith(
        File(
          repoPath([
            'bindings',
            'dart',
            'guatiao',
            'test',
            'fixtures',
            'vault_kind.dart',
          ]),
        ).readAsStringSync(),
      ),
    );
  });
}
