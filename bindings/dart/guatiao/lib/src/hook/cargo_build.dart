// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';

/// What the core package's hook built, for the hooks of the packages over
/// it: `guatiao` (its own crate), `app` (the application's crate, which
/// carries every surface) or `none`.
const String _builtKey = 'library';

/// A missing prerequisite or a failed build, in one sentence.
final class HookFailure implements Exception {
  HookFailure(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A build hook's `main`: [buildLibrary] for [crate], with a failure
/// reported as the hook's error.
Future<void> buildHook(
  List<String> arguments, {
  required String crate,
  List<String> features = const <String>[],
}) => build(arguments, (input, output) async {
  try {
    await buildLibrary(input, output, crate: crate, features: features);
  } on HookFailure catch (failure) {
    throw BuildError(message: failure.message);
  }
});

/// Builds [crate]'s C library for the target [input] names and adds it to
/// [output] as the package's code asset, its id `package:p/p` for the
/// package `p`.
///
/// [features] are the crate's own, or `dependency/feature` for one of its
/// dependencies. The core package (`guatiao`) reads two user defines:
/// `build` set to `false` builds nothing, and `crate`, a path, builds the
/// application's crate there instead of [crate]. Either way it tells the
/// hooks of the packages over it, which then build nothing of their own.
///
/// The crate's source is the repository checkout the package sits in
/// (`bindings/dart/` and the package's name), else crates.io at the
/// package's own version. Cargo's target directory lives in the hook's
/// shared output directory, so a second build reuses it.
///
/// [environment] is where cargo, rustup's toolchains and the Android NDK
/// are looked for, and what cargo runs with; the process's own by default
/// (a hook sees PATH, HOME and the Android variables, not the rest).
///
/// Throws [HookFailure] for a missing prerequisite (cargo, the Android NDK,
/// Xcode, the Rust target) or a failed build.
Future<void> buildLibrary(
  BuildInput input,
  BuildOutputBuilder output, {
  required String crate,
  List<String> features = const <String>[],
  Map<String, String>? environment,
}) async {
  if (!input.config.buildCodeAssets) return;
  final package = input.packageName;
  Uri? appCrate;
  if (package == 'guatiao') {
    final build = input.userDefines['build'];
    if (build == false || build == 'false') {
      output.metadata[_builtKey] = 'none';
      return;
    }
    appCrate = input.userDefines.path('crate');
    output.metadata[_builtKey] = appCrate == null ? 'guatiao' : 'app';
  } else {
    final built = input.metadata['guatiao'][_builtKey];
    if (built == 'none' || built == 'app') return;
  }

  final code = input.config.code;
  final triple = rustTarget(
    code.targetOS,
    code.targetArchitecture,
    iOSSdk: code.targetOS == OS.iOS ? code.iOS.targetSdk : null,
  );
  if (triple == null) {
    throw HookFailure(
      'guatiao builds for no ${code.targetOS} target on '
      '${code.targetArchitecture}: its C ABI is laid out for 32- and 64-bit '
      'ARM, x86-64 and RISC-V.',
    );
  }

  final host = _Host(environment ?? Platform.environment);
  final cargo = host.findCargo();
  final linking = host.targetEnvironment(code, triple);

  final shared = Directory.fromUri(input.outputDirectoryShared);
  final Directory workDir;
  final List<String> selection;
  final bool Function(Map<String, Object?> artifact) isOurs;
  if (appCrate == null) {
    workDir = _writeWrapper(input, shared, crate, features);
    selection = <String>['-p', crate];
    final libName = crate.replaceAll('-', '_');
    isOurs = (artifact) =>
        (artifact['target'] as Map<String, Object?>?)?['name'] == libName;
  } else {
    final manifest = _manifestOf(appCrate);
    workDir = manifest.parent;
    selection = <String>['--manifest-path', manifest.path];
    final wanted = _canonical(manifest.path);
    isOurs = (artifact) =>
        artifact['manifest_path'] is String &&
        _canonical(artifact['manifest_path'] as String) == wanted;
  }
  host.requireTarget(host.findRustc(cargo), triple, workDir);

  final targetDir = Directory.fromUri(
    shared.uri.resolve(appCrate == null ? 'target/' : 'app-target/'),
  );
  final arguments = <String>[
    'build',
    '--release',
    '--lib',
    ...selection,
    '--target',
    triple,
    '--target-dir',
    targetDir.path,
    '--message-format=json-render-diagnostics',
  ];
  stdout.writeln(
    'guatiao: in ${workDir.path}: ${cargo.path} ${arguments.join(' ')}',
  );
  final process = await Process.start(
    cargo.path,
    arguments,
    workingDirectory: workDir.path,
    environment: <String, String>{
      ...host.environment,
      'PATH': host.pathWith(cargo.parent.path),
      'CARGO_TERM_COLOR': 'never',
      ...linking,
    },
    includeParentEnvironment: false,
  );
  final tail = <String>[];
  final errors = process.stderr
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .forEach((line) {
        stderr.writeln(line);
        tail.add(line);
        if (tail.length > 40) tail.removeAt(0);
      });
  final suffix = _dynamicSuffix(code.targetOS);
  String? library;
  final manifests = <String>{};
  await process.stdout
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .forEach((line) {
        if (!line.startsWith('{')) return;
        final Object? message;
        try {
          message = jsonDecode(line);
        } on FormatException {
          return;
        }
        if (message is! Map<String, Object?>) return;
        if (message['reason'] != 'compiler-artifact') return;
        final packageId = message['package_id'];
        final manifest = message['manifest_path'];
        // A crate from a path (the checkout, the application's own): its
        // manifest is a source the build depends on too.
        if (packageId is String &&
            manifest is String &&
            packageId.startsWith('path+')) {
          manifests.add(manifest);
        }
        if (!isOurs(message)) return;
        for (final file
            in (message['filenames'] as List<Object?>? ?? const [])) {
          if (file is String && file.endsWith(suffix)) library = file;
        }
      });
  await errors;
  final what = appCrate == null ? crate : appCrate.toFilePath();
  final exit = await process.exitCode;
  if (exit != 0) {
    throw HookFailure(
      'cargo could not build $what for $triple (exit $exit); its last '
      'lines:\n${tail.join('\n')}',
    );
  }
  final built = library;
  if (built == null) {
    throw HookFailure(
      '$what built no shared library for $triple: its [lib] needs '
      'crate-type = ["cdylib"].',
    );
  }

  final bundled = File.fromUri(
    input.outputDirectory.resolve(File(built).uri.pathSegments.last),
  );
  File(built).copySync(bundled.path);
  output.assets.code.add(
    CodeAsset(
      package: package,
      name: package,
      linkMode: DynamicLoadingBundled(),
      file: bundled.uri,
    ),
  );
  output.dependencies.addAll(<Uri>{
    // A build script's `rerun-if-changed` may name a directory, which the
    // runner hashes by its listing only when the URI says it is one.
    for (final source in {..._depInfo(built), ...manifests})
      Directory(source).existsSync() ? Uri.directory(source) : Uri.file(source),
    input.packageRoot.resolve('pubspec.yaml'),
  });
}

/// The Rust target triple for a Dart target, or `null` where guatiao has
/// none.
String? rustTarget(OS os, Architecture architecture, {IOSSdk? iOSSdk}) =>
    switch ((os, architecture)) {
      (OS.android, Architecture.arm64) => 'aarch64-linux-android',
      (OS.android, Architecture.arm) => 'armv7-linux-androideabi',
      (OS.android, Architecture.x64) => 'x86_64-linux-android',
      (OS.android, Architecture.riscv64) => 'riscv64-linux-android',
      (OS.linux, Architecture.arm64) => 'aarch64-unknown-linux-gnu',
      (OS.linux, Architecture.arm) => 'armv7-unknown-linux-gnueabihf',
      (OS.linux, Architecture.x64) => 'x86_64-unknown-linux-gnu',
      (OS.linux, Architecture.riscv64) => 'riscv64gc-unknown-linux-gnu',
      (OS.windows, Architecture.arm64) => 'aarch64-pc-windows-msvc',
      (OS.windows, Architecture.x64) => 'x86_64-pc-windows-msvc',
      (OS.macOS, Architecture.arm64) => 'aarch64-apple-darwin',
      (OS.macOS, Architecture.x64) => 'x86_64-apple-darwin',
      (OS.iOS, Architecture.arm64) =>
        iOSSdk == IOSSdk.iPhoneSimulator
            ? 'aarch64-apple-ios-sim'
            : 'aarch64-apple-ios',
      (OS.iOS, Architecture.x64) => 'x86_64-apple-ios',
      _ => null,
    };

/// The sentence a build missing the Rust target [triple] fails with.
String missingTarget(String triple) =>
    'The Rust target $triple is not installed: run '
    '`rustup target add $triple`.';

/// Android's clang names its 32-bit ARM target after the instruction set.
String _clangTriple(String triple) =>
    triple == 'armv7-linux-androideabi' ? 'armv7a-linux-androideabi' : triple;

String _dynamicSuffix(OS os) => switch (os) {
  OS.windows => '.dll',
  OS.macOS || OS.iOS => '.dylib',
  _ => '.so',
};

String _exe(String name) => Platform.isWindows ? '$name.exe' : name;

/// A crate of no code of its own, depending on [crate] from the checkout
/// or from crates.io, so that `cargo build -p` builds the crate's library
/// outside any workspace the application sits in.
Directory _writeWrapper(
  BuildInput input,
  Directory shared,
  String crate,
  List<String> features,
) {
  final dir = Directory.fromUri(shared.uri.resolve('crate/'))
    ..createSync(recursive: true);
  final repository = input.packageRoot.resolve('../../../');
  final inCheckout = File.fromUri(
    repository.resolve('crates/$crate/Cargo.toml'),
  ).existsSync();
  final String? version;
  if (inCheckout) {
    version = null;
  } else {
    final pubspec = File.fromUri(
      input.packageRoot.resolve('pubspec.yaml'),
    ).readAsStringSync();
    version = RegExp(
      r'^version:\s*(\S+)',
      multiLine: true,
    ).firstMatch(pubspec)?.group(1);
    if (version == null) {
      throw HookFailure(
        '${input.packageName} names no version in its pubspec.yaml, so the '
        'crate to build from crates.io is unknown.',
      );
    }
  }

  final byDependency = <String, List<String>>{crate: <String>[]};
  for (final feature in features) {
    final slash = feature.indexOf('/');
    if (slash < 0) {
      byDependency[crate]!.add(feature);
    } else {
      (byDependency[feature.substring(0, slash)] ??= <String>[]).add(
        feature.substring(slash + 1),
      );
    }
  }
  String source(String name) {
    if (!inCheckout) return 'version = "=$version"';
    final path = repository.resolve('crates/$name/').toFilePath();
    return "path = '${path.replaceAll('\\', '/')}'";
  }

  final manifest = StringBuffer()
    ..writeln("# Written by guatiao's build hook: builds $crate's C library.")
    ..writeln('[package]')
    ..writeln('name = "guatiao-hook-build"')
    ..writeln('version = "0.0.0"')
    ..writeln('edition = "2021"')
    ..writeln('publish = false')
    ..writeln()
    ..writeln('[lib]')
    ..writeln('path = "lib.rs"')
    ..writeln()
    ..writeln('[dependencies]');
  for (final MapEntry(key: name, value: wanted) in byDependency.entries) {
    final list = wanted.map((f) => '"$f"').join(', ');
    manifest.writeln('$name = { ${source(name)}, features = [$list] }');
  }
  manifest
    ..writeln()
    ..writeln('[workspace]');
  _writeIfChanged(
    File.fromUri(dir.uri.resolve('Cargo.toml')),
    manifest.toString(),
  );
  _writeIfChanged(File.fromUri(dir.uri.resolve('lib.rs')), '');
  final lock = File.fromUri(repository.resolve('Cargo.lock'));
  if (inCheckout && lock.existsSync()) {
    // The checkout's own resolution: the dependency versions its tests ran
    // against.
    _writeIfChanged(
      File.fromUri(dir.uri.resolve('Cargo.lock')),
      lock.readAsStringSync(),
    );
  }
  return dir;
}

void _writeIfChanged(File file, String contents) {
  if (file.existsSync() && file.readAsStringSync() == contents) return;
  final temporary = File('${file.path}.$pid.tmp')..writeAsStringSync(contents);
  temporary.renameSync(file.path);
}

File _manifestOf(Uri crate) {
  final path = crate.toFilePath();
  final asFile = File(path);
  if (asFile.existsSync() && path.endsWith('Cargo.toml')) return asFile;
  final inside = File(
    '${Directory(path).path}${Platform.pathSeparator}Cargo.toml',
  );
  if (inside.existsSync()) return inside;
  throw HookFailure(
    'The crate guatiao was told to build, $path, has no Cargo.toml.',
  );
}

String _canonical(String path) {
  final resolved = File(path).absolute.path;
  return Platform.isWindows
      ? resolved.replaceAll('/', '\\').toLowerCase()
      : resolved;
}

/// The machine as [environment] describes it: where cargo, the NDK and
/// Xcode are.
final class _Host {
  _Host(this.environment);

  final Map<String, String> environment;

  String get _separator => Platform.isWindows ? ';' : ':';

  String get _home => environment['HOME'] ?? environment['USERPROFILE'] ?? '';

  File? _onPath(String name) {
    for (final dir in (environment['PATH'] ?? '').split(_separator)) {
      if (dir.isEmpty) continue;
      final candidate = File('$dir${Platform.pathSeparator}${_exe(name)}');
      if (candidate.existsSync()) return candidate;
    }
    return null;
  }

  String pathWith(String dir) {
    final path = environment['PATH'] ?? '';
    return path.isEmpty ? dir : '$dir$_separator$path';
  }

  /// cargo from PATH, else rustup's default place: an app launched from a
  /// desktop (Xcode, Android Studio) often has no `~/.cargo/bin` on PATH.
  File findCargo() {
    final found = _onPath('cargo');
    if (found != null) return found;
    final rustup = File(
      [_home, '.cargo', 'bin', _exe('cargo')].join(Platform.pathSeparator),
    );
    if (_home.isNotEmpty && rustup.existsSync()) return rustup;
    throw HookFailure(
      'cargo was not found on PATH or in ~/.cargo/bin: install Rust with '
      'rustup from https://rustup.rs.',
    );
  }

  File findRustc(File cargo) {
    final beside = File(
      '${cargo.parent.path}${Platform.pathSeparator}${_exe('rustc')}',
    );
    return beside.existsSync() ? beside : (_onPath('rustc') ?? beside);
  }

  /// Fails with [missingTarget] unless the toolchain cargo will use in
  /// [workDir] has the standard library for [triple].
  void requireTarget(File rustc, String triple, Directory workDir) {
    final ProcessResult result;
    try {
      result = Process.runSync(
        rustc.path,
        <String>['--print', 'sysroot'],
        workingDirectory: workDir.path,
        environment: environment,
        includeParentEnvironment: false,
      );
    } on ProcessException {
      throw HookFailure(
        'rustc was not found beside ${rustc.parent.path}: install Rust with '
        'rustup from https://rustup.rs.',
      );
    }
    if (result.exitCode != 0) {
      final first = '${result.stderr}'.trim().split('\n').first;
      throw HookFailure(
        'rustc did not run ($first): install a toolchain with '
        '`rustup default stable`.',
      );
    }
    final sysroot = '${result.stdout}'.trim();
    final lib = Directory(
      [sysroot, 'lib', 'rustlib', triple, 'lib'].join(Platform.pathSeparator),
    );
    if (!lib.existsSync()) throw HookFailure(missingTarget(triple));
  }

  /// The variables cargo needs to link for [triple]: the NDK's clang for
  /// Android, the deployment target for Apple's platforms, the build's own
  /// C compiler on Linux; nothing on Windows, where rustc finds MSVC.
  Map<String, String> targetEnvironment(CodeConfig code, String triple) {
    final key = triple.toUpperCase().replaceAll('-', '_');
    final ccKey = triple.replaceAll('-', '_');
    switch (code.targetOS) {
      case OS.android:
        final api = code.android.targetNdkApi;
        final (clang, archiver) = _androidClang(code);
        final target = '--target=${_clangTriple(triple)}$api';
        final flags = <String>[
          '-C',
          'link-arg=$target',
          // 16 KB pages, which Android 15 devices may use; 32-bit has none.
          if (triple != 'armv7-linux-androideabi') ...[
            '-C',
            'link-arg=-Wl,-z,max-page-size=16384',
          ],
        ];
        return <String, String>{
          'CARGO_TARGET_${key}_LINKER': clang,
          'CARGO_TARGET_${key}_RUSTFLAGS': flags.join(' '),
          'CC_$ccKey': clang,
          'CFLAGS_$ccKey': target,
          'AR_$ccKey': archiver,
        };
      case OS.iOS:
        final sdk = code.iOS.targetSdk == IOSSdk.iPhoneSimulator
            ? 'iphonesimulator'
            : 'iphoneos';
        _requireXcode(sdk);
        return <String, String>{
          'IPHONEOS_DEPLOYMENT_TARGET': '${code.iOS.targetVersion}.0',
        };
      case OS.macOS:
        _requireXcode('macosx');
        return <String, String>{
          'MACOSX_DEPLOYMENT_TARGET': '${code.macOS.targetVersion}.0',
        };
      case OS.linux:
        // The compiler Flutter's Linux setup installs (clang): `cc`, rustc's
        // default linker, need not exist beside it.
        final given = code.cCompiler;
        if (given == null) return const <String, String>{};
        return <String, String>{
          'CARGO_TARGET_${key}_LINKER': given.compiler.toFilePath(),
          'CC_$ccKey': given.compiler.toFilePath(),
          'AR_$ccKey': given.archiver.toFilePath(),
        };
      default:
        return const <String, String>{};
    }
  }

  void _requireXcode(String sdk) {
    ProcessResult? result;
    final xcrun = _onPath('xcrun');
    if (xcrun != null) {
      try {
        result = Process.runSync(
          xcrun.path,
          <String>['--sdk', sdk, '--show-sdk-path'],
          environment: environment,
          includeParentEnvironment: false,
        );
      } on ProcessException {
        result = null;
      }
    }
    if (result == null || result.exitCode != 0) {
      throw HookFailure(
        'Xcode was not found (xcrun has no $sdk SDK): install Xcode and '
        'select it with `sudo xcode-select --switch /Applications/Xcode.app`.',
      );
    }
  }

  /// The clang and archiver to link Android code with: the ones the build
  /// was handed (Flutter passes the NDK's), else the newest NDK found
  /// through the usual variables and SDK locations.
  (String, String) _androidClang(CodeConfig code) {
    final given = code.cCompiler;
    if (given != null) {
      return (given.compiler.toFilePath(), given.archiver.toFilePath());
    }
    final ndk = _findNdk();
    if (ndk == null) {
      throw HookFailure(
        "The Android NDK was not found: install it with Android Studio's "
        'SDK Manager (or `sdkmanager --install "ndk;27.0.12077973"`), or set '
        'ANDROID_NDK_HOME.',
      );
    }
    final prebuilt = Directory(
      [ndk.path, 'toolchains', 'llvm', 'prebuilt'].join(Platform.pathSeparator),
    );
    final hosts = prebuilt.existsSync()
        ? prebuilt.listSync().whereType<Directory>().toList()
        : const <Directory>[];
    if (hosts.isEmpty) {
      throw HookFailure(
        'The Android NDK at ${ndk.path} has no LLVM toolchain for this host: '
        'install a current NDK with Android Studio.',
      );
    }
    final bin =
        '${hosts.first.path}${Platform.pathSeparator}bin'
        '${Platform.pathSeparator}';
    return ('$bin${_exe('clang')}', '$bin${_exe('llvm-ar')}');
  }

  Directory? _findNdk() {
    for (final name in const <String>[
      'ANDROID_NDK_HOME',
      'ANDROID_NDK_ROOT',
      'ANDROID_NDK',
      'ANDROID_NDK_LATEST_HOME',
    ]) {
      final value = environment[name];
      if (value != null && value.isNotEmpty && Directory(value).existsSync()) {
        return Directory(value);
      }
    }
    final separator = Platform.pathSeparator;
    final sdks = <String>[
      ?environment['ANDROID_HOME'],
      if (_home.isNotEmpty && Platform.isWindows)
        [_home, 'AppData', 'Local', 'Android', 'Sdk'].join(separator),
      if (_home.isNotEmpty && Platform.isMacOS)
        [_home, 'Library', 'Android', 'sdk'].join(separator),
      if (_home.isNotEmpty && Platform.isLinux)
        [_home, 'Android', 'Sdk'].join(separator),
    ];
    for (final sdk in sdks) {
      if (sdk.isEmpty) continue;
      final ndks = Directory('$sdk${separator}ndk');
      if (!ndks.existsSync()) continue;
      final versions = ndks.listSync().whereType<Directory>().toList()
        ..sort((a, b) => _compareVersions(a.path, b.path));
      if (versions.isNotEmpty) return versions.last;
    }
    return null;
  }
}

int _compareVersions(String a, String b) {
  List<int> parts(String path) => path
      .split(RegExp(r'[\\/]'))
      .last
      .split('.')
      .map((p) => int.tryParse(p) ?? 0)
      .toList();
  final x = parts(a);
  final y = parts(b);
  for (var i = 0; i < x.length && i < y.length; i++) {
    if (x[i] != y[i]) return x[i].compareTo(y[i]);
  }
  return x.length.compareTo(y.length);
}

/// Every source file cargo's dep-info lists for [artifact], so the hook
/// runs again when one changes.
List<String> _depInfo(String artifact) {
  final dot = artifact.lastIndexOf('.');
  final file = File('${artifact.substring(0, dot)}.d');
  if (!file.existsSync()) return const <String>[];
  final first = file.readAsLinesSync().firstWhere(
    (line) => line.contains(': '),
    orElse: () => '',
  );
  final colon = first.indexOf(': ');
  if (colon < 0) return const <String>[];
  final sources = <String>[];
  final current = StringBuffer();
  final text = first.substring(colon + 2);
  for (var i = 0; i < text.length; i++) {
    final c = text[i];
    if (c == '\\' && i + 1 < text.length && text[i + 1] == ' ') {
      current.write(' ');
      i++;
    } else if (c == ' ') {
      if (current.isNotEmpty) sources.add(current.toString());
      current.clear();
    } else {
      current.write(c);
    }
  }
  if (current.isNotEmpty) sources.add(current.toString());
  return sources;
}
