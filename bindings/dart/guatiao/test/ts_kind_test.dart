// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A kind written as TypeScript interfaces, served from Dart: the
/// description `guatiao-kind` writes from them is the one `dart run
/// guatiao:kind` writes the committed package from, and a provider written
/// in Dart serves every slot of it.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kinds.dart';
import 'package:guatiao/src/kind_generator.dart';
import 'package:test/test.dart';

// The generated kinds of the example package, which this package cannot
// depend on: it depends on this one.
// ignore: avoid_relative_lib_imports
import '../../../../examples/tally_kind/dart/lib/tally_kind.dart';
import 'support.dart';

final class Counts extends Provider
    implements
        Tally,
        TallyAll,
        TallyKnown,
        TallyChecksum,
        TallyReport,
        TallyLoad {
  final Map<String, int> counts = {};

  @override
  int add(String name, int by) {
    if (name.isEmpty) {
      throw BadValue(Status.badValue.code, 'a count has a name');
    }
    return counts.update(name, (n) => n + by, ifAbsent: () => by);
  }

  @override
  int total(String name) => counts[name] ?? 0;

  @override
  Object all() => Map<String, Object?>.of(counts);

  @override
  bool known(String name) => counts.containsKey(name);

  @override
  int checksum(Uint8List data) =>
      data.fold(0, (sum, b) => (sum + b) & 0xffffffff);

  @override
  void report(WatcherRemote watcher) =>
      counts.forEach((name, total) => watcher.changed(name, total));

  @override
  int load(Value taken) {
    try {
      final map = taken.asMap();
      for (final name in map.keys) {
        counts.update(
          name,
          (n) => n + map[name]!.intOr(0),
          ifAbsent: () => map[name]!.intOr(0),
        );
      }
      return map.length;
    } finally {
      taken.close();
    }
  }
}

final class Board implements Watcher {
  final List<(String, int)> told = [];

  @override
  void changed(String name, int total) => told.add((name, total));
}

void main() {
  setUpAll(libraryDir);

  test('the committed package is what the description writes', () {
    final document =
        (jsonDecode(
                  File(
                    repoPath([
                      'examples',
                      'tally_kind',
                      'include',
                      'tally_kind.kinds.json',
                    ]),
                  ).readAsStringSync(),
                )
                as Map)
            .cast<String, Object?>();
    expect(
      generateKinds(
        document,
        source: '../../include/tally_kind.kinds.json',
        command:
            'dart run guatiao:kind ../include/tally_kind.kinds.json -o '
            'lib/tally_kind.dart',
      ),
      File(
        repoPath(['examples', 'tally_kind', 'dart', 'lib', 'tally_kind.dart']),
      ).readAsStringSync(),
      reason:
          'regenerate: in examples/tally_kind/dart, dart run guatiao:kind '
          '../include/tally_kind.kinds.json -o lib/tally_kind.dart',
    );
  });

  test('the kind is what the interfaces say', () {
    expect(Tally.kind.name, 'tally');
    expect(Tally.kind.floorHash, 1286177969);
    expect(Watcher.kind.isObject, isTrue);
    expect(Watcher.kind.floorHash, 297588550);
  });

  group('a provider written in Dart', () {
    late Registry reg;
    late Library library;
    late Counts counts;
    setUp(() {
      counts = Counts();
      library = Library(
        'dart_tally',
        '1.0.0',
        offers: [
          Offer.single(counts, named: 'counts', kinds: [Tally.kind]),
        ],
      );
      reg = Registry('dart-tally', '0.1')..registerLibrary(library);
      addTearDown(() {
        reg.close();
        library.close();
      });
    });

    test('serves every slot of the kind', () {
      final tally = reg.bind('dart_tally_counts', Tally.kind);
      expect(tally.add('a', 2), 2);
      expect(tally.add('a', 3), 5);
      expect(tally.total('a'), 5);
      expect(tally.total('zzz'), 0);
      expect(
        () => tally.add('', 1),
        throwsA(
          isA<BadValue>().having(
            (e) => e.message,
            'message',
            contains('a count has a name'),
          ),
        ),
      );
      expect(tally.known('a'), isTrue);
      expect(tally.known('zzz'), isFalse);
      expect(tally.checksum(Uint8List.fromList([1, 2, 3, 250])), 256);
      final all = tally.all();
      addTearDown(all.close);
      expect(all.toDart(), {'a': 5});
      final loaded = Value.fromDart({'a': 1, 'b': 7});
      expect(tally.load(loaded), 2);
      expect(loaded.tag, Tag.absent, reason: 'a Value handed over is moved');
      final board = Board();
      tally.report(board);
      expect(board.told, [('a', 6), ('b', 7)]);
      expect(counts.counts, {'a': 6, 'b': 7});
    });
  });
}
