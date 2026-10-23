// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `package:guatiao/hook.dart`: the target table, the switch that builds
/// nothing, the sentence each missing prerequisite fails with, and real
/// builds of the core and of an application's own crate for this machine.
@TestOn('vm')
library;

import 'dart:ffi';
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:guatiao/hook.dart';
import 'package:hooks/hooks.dart';
import 'package:test/test.dart';

/// Runs the core package's hook in this process with [environment], and
/// answers its output with which of [symbols] its library exports: the
/// hook's directories are gone once it returns.
Future<(BuildOutput, Set<String>)> _run({
  List<String> symbols = const <String>[],
  Map<String, String>? environment,
  Map<String, Object?>? defines,
  OS? targetOS,
  Architecture? targetArchitecture,
}) async {
  late BuildOutput result;
  final exported = <String>{};
  await testCodeBuildHook(
    mainMethod: (arguments) => build(
      arguments,
      (input, output) => buildLibrary(
        input,
        output,
        crate: 'guatiao',
        features: const <String>['load'],
        environment: environment,
      ),
    ),
    check: (input, output) {
      result = output;
      for (final asset in output.assets.code) {
        final library = DynamicLibrary.open(asset.file!.toFilePath());
        exported.addAll(symbols.where(library.providesSymbol));
        library.close();
      }
    },
    targetOS: targetOS,
    targetArchitecture: targetArchitecture,
    userDefines: defines == null
        ? null
        : PackageUserDefines(
            workspacePubspec: PackageUserDefinesSource(
              defines: defines,
              basePath: Directory.current.uri,
            ),
          ),
  );
  return (result, exported);
}

Matcher _failsWith(String start) => throwsA(
  isA<HookFailure>().having((f) => f.message, 'message', startsWith(start)),
);

/// The directory holding cargo in this process's environment.
String _cargoDir() {
  final separator = Platform.isWindows ? ';' : ':';
  final exe = Platform.isWindows ? 'cargo.exe' : 'cargo';
  final home =
      Platform.environment['HOME'] ?? Platform.environment['USERPROFILE']!;
  for (final dir in <String>[
    ...(Platform.environment['PATH'] ?? '').split(separator),
    [home, '.cargo', 'bin'].join(Platform.pathSeparator),
  ]) {
    if (dir.isNotEmpty &&
        File('$dir${Platform.pathSeparator}$exe').existsSync()) {
      return dir;
    }
  }
  fail('no cargo on PATH or in ~/.cargo/bin');
}

String _metadata(BuildOutput output) => output.assets.encodedAssetsForBuild
    .where((asset) => asset.type == 'hooks/metadata')
    .map((asset) => '${asset.encoding['key']}=${asset.encoding['value']}')
    .join(',');

void main() {
  test('each Dart target has its Rust target', () {
    expect(rustTarget(OS.android, Architecture.arm64), 'aarch64-linux-android');
    expect(rustTarget(OS.android, Architecture.arm), 'armv7-linux-androideabi');
    expect(rustTarget(OS.android, Architecture.x64), 'x86_64-linux-android');
    expect(rustTarget(OS.linux, Architecture.x64), 'x86_64-unknown-linux-gnu');
    expect(
      rustTarget(OS.windows, Architecture.arm64),
      'aarch64-pc-windows-msvc',
    );
    expect(rustTarget(OS.macOS, Architecture.arm64), 'aarch64-apple-darwin');
    expect(rustTarget(OS.iOS, Architecture.arm64), 'aarch64-apple-ios');
    expect(
      rustTarget(OS.iOS, Architecture.arm64, iOSSdk: IOSSdk.iPhoneSimulator),
      'aarch64-apple-ios-sim',
    );
    // 32-bit x86 lays a 64-bit integer out on four bytes, which the ABI's
    // structs are not written for.
    expect(rustTarget(OS.android, Architecture.ia32), isNull);
  });

  test('build: false builds nothing and tells the packages over it', () async {
    final (output, _) = await _run(defines: <String, Object?>{'build': false});
    expect(output.assets.code, isEmpty);
    expect(_metadata(output), 'library=none');
  });

  group('a missing prerequisite is named in one sentence', () {
    late Directory empty;
    setUp(() => empty = Directory.systemTemp.createTempSync('guatiao_hook'));
    tearDown(() => empty.deleteSync(recursive: true));

    test('cargo', () async {
      await expectLater(
        _run(
          environment: <String, String>{
            'PATH': '',
            'HOME': empty.path,
            'USERPROFILE': empty.path,
          },
        ),
        _failsWith('cargo was not found on PATH or in ~/.cargo/bin'),
      );
    });

    test('the Android NDK', () async {
      await expectLater(
        _run(
          environment: <String, String>{
            'PATH': _cargoDir(),
            'HOME': empty.path,
            'USERPROFILE': empty.path,
          },
          targetOS: OS.android,
          targetArchitecture: Architecture.arm64,
        ),
        _failsWith('The Android NDK was not found'),
      );
    });

    test('Xcode', () async {
      await expectLater(
        _run(
          environment: <String, String>{
            'PATH': _cargoDir(),
            'HOME': empty.path,
            'USERPROFILE': empty.path,
          },
          targetOS: OS.iOS,
          targetArchitecture: Architecture.arm64,
        ),
        _failsWith('Xcode was not found'),
      );
    });

    test('the Rust target', () async {
      const triple = 'riscv64gc-unknown-linux-gnu';
      final sysroot = Process.runSync('rustc', <String>[
        '--print',
        'sysroot',
      ], runInShell: Platform.isWindows).stdout.toString().trim();
      final installed = Directory(
        [sysroot, 'lib', 'rustlib', triple].join(Platform.pathSeparator),
      ).existsSync();
      if (installed) {
        markTestSkipped('$triple is installed here');
        return;
      }
      await expectLater(
        _run(targetOS: OS.linux, targetArchitecture: Architecture.riscv64),
        throwsA(
          isA<HookFailure>().having(
            (f) => f.message,
            'message',
            missingTarget(triple),
          ),
        ),
      );
    });
  });

  test(
    'builds the core for this machine, with the sources it depends on',
    () async {
      final (output, exported) = await _run(
        symbols: const <String>['guatiao_value_free', 'guatiao_registry_new'],
      );
      expect(_metadata(output), 'library=guatiao');
      final asset = output.assets.code.single;
      expect(asset.id, 'package:guatiao/guatiao');
      expect(exported, {'guatiao_value_free', 'guatiao_registry_new'});
      expect(
        output.dependencies.map((uri) => uri.toFilePath()),
        contains(
          endsWith(
            ['crates', 'guatiao', 'src', 'lib.rs'].join(Platform.pathSeparator),
          ),
        ),
      );
    },
    timeout: const Timeout(Duration(minutes: 10)),
  );

  test(
    "builds the application's own crate when told, one file for the surfaces",
    () async {
      final crate = Directory.systemTemp.createTempSync('guatiao_app_crate');
      addTearDown(() => crate.deleteSync(recursive: true));
      final core = Directory.current.uri
          .resolve('../../../crates/guatiao/')
          .toFilePath()
          .replaceAll('\\', '/');
      File('${crate.path}/Cargo.toml').writeAsStringSync('''
[package]
name = "hook_app"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]
path = "lib.rs"

[dependencies]
guatiao = { path = '$core' }

[workspace]
''');
      File('${crate.path}/lib.rs').writeAsStringSync('''
use guatiao as _;

#[no_mangle]
pub extern "C" fn hook_app_version() -> u32 {
    1
}
''');
      final (output, exported) = await _run(
        symbols: const <String>['hook_app_version', 'guatiao_value_free'],
        defines: <String, Object?>{'crate': crate.path},
      );
      expect(_metadata(output), 'library=app');
      final asset = output.assets.code.single;
      expect(asset.id, 'package:guatiao/guatiao');
      expect(exported, {'hook_app_version', 'guatiao_value_free'});
    },
    timeout: const Timeout(Duration(minutes: 10)),
  );
}
