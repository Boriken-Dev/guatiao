// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What `dart run guatiao:kind` writes: the Dart for a document of kinds,
/// and a package's library identity from its `pubspec.yaml`. Plain text
/// in, plain text out; nothing native.
library;

import 'dart:convert';

/// A kind's name as a Dart type name: `session_backend` is
/// `SessionBackend`.
String typeNameOf(String kindName) {
  final name = kindName
      .split(RegExp('[^A-Za-z0-9]+'))
      .where((part) => part.isNotEmpty)
      .map((part) => part[0].toUpperCase() + part.substring(1))
      .join();
  if (name.isEmpty) {
    throw FormatException('the kind "$kindName" has no letters to name a type');
  }
  return RegExp('^[0-9]').hasMatch(name) ? 'Kind$name' : name;
}

/// A slot's or argument's name in Dart: `greet_into` is `greetInto`.
String memberNameOf(String name) {
  final parts = name.split(RegExp('_+')).where((p) => p.isNotEmpty).toList();
  if (parts.isEmpty) return name;
  return parts.first +
      parts.skip(1).map((p) => p[0].toUpperCase() + p.substring(1)).join();
}

const Set<String> _keywords = {
  'abstract',
  'as',
  'assert',
  'async',
  'await',
  'base',
  'break',
  'case',
  'catch',
  'class',
  'const',
  'continue',
  'covariant',
  'default',
  'deferred',
  'do',
  'dynamic',
  'else',
  'enum',
  'export',
  'extends',
  'extension',
  'external',
  'factory',
  'false',
  'final',
  'finally',
  'for',
  'Function',
  'get',
  'hide',
  'if',
  'implements',
  'import',
  'in',
  'interface',
  'is',
  'late',
  'library',
  'mixin',
  'new',
  'null',
  'of',
  'on',
  'operator',
  'part',
  'required',
  'rethrow',
  'return',
  'sealed',
  'set',
  'show',
  'static',
  'super',
  'switch',
  'sync',
  'this',
  'throw',
  'true',
  'try',
  'type',
  'typedef',
  'var',
  'void',
  'when',
  'while',
  'with',
  'yield',
};

/// What a remote answers itself, so no slot may be called it.
const Set<String> _reserved = {
  'kind',
  'table',
  'size',
  'ctx',
  'has',
  'slot',
  'close',
  'isClosed',
  'hashCode',
  'toString',
  'runtimeType',
  'noSuchMethod',
};

String _argName(Object? name, int i) {
  final camel = name is String ? memberNameOf(name) : 'arg$i';
  final plain = RegExp(r'^[A-Za-z_$][A-Za-z0-9_$]*$').hasMatch(camel)
      ? camel
      : 'arg$i';
  // These are the generated code's own locals.
  return _keywords.contains(plain) ||
          const {
            'c',
            's',
            'out',
            'err',
            'status',
            'ctx',
            'target',
            'result',
            'slot',
          }.contains(plain)
      ? '${plain}_'
      : plain;
}

const Map<String, (String, String)> _scalars = {
  'bool': ('Bool', 'bool'),
  'i8': ('Int8', 'int'),
  'i16': ('Int16', 'int'),
  'i32': ('Int32', 'int'),
  'i64': ('Int64', 'int'),
  'u8': ('Uint8', 'int'),
  'u16': ('Uint16', 'int'),
  'u32': ('Uint32', 'int'),
  'u64': ('Uint64', 'int'),
  'isize': ('IntPtr', 'int'),
  'usize': ('Size', 'int'),
  'f32': ('Float', 'double'),
  'f64': ('Double', 'double'),
};

/// One argument: its C type twice (the native typedef and the Dart one),
/// the types each side sees, and how each side crosses it.
final class _Arg {
  _Arg({
    required this.name,
    required this.native,
    required this.dart,
    required this.caller,
    required this.callee,
    required this.toC,
    required this.fromC,
    this.taken = false,
  });

  final String name;
  final String native;
  final String dart;
  final String caller;
  final String callee;
  final String toC;
  final String fromC;
  final bool taken;
}

/// One answer: its out pointer's C type, the types each side sees, and how
/// each side reads or writes it.
final class _Answer {
  const _Answer({
    required this.pointee,
    required this.caller,
    required this.callee,
    required this.outSlot,
    required this.read,
    required this.write,
  });

  /// What the out pointer points at; empty for no answer.
  final String pointee;
  final String caller;
  final String callee;

  /// The caller's out slot, built in its scope.
  final String outSlot;

  /// The caller reads `out`.
  final String read;

  /// The callee writes `result` into `out`.
  final String write;
}

const _none = _Answer(
  pointee: '',
  caller: 'void',
  callee: 'void',
  outSlot: '',
  read: '',
  write: '',
);

final class _Generator {
  _Generator(this.document, this.kindImports);

  final Map<String, Object?> document;

  /// Object kinds another document describes, each with the library
  /// `dart run guatiao:kind` wrote for it.
  final Map<String, String> kindImports;
  final Map<String, String> names = {};
  final Set<String> imports = {};
  final Set<String> declared = {};

  List<Map<String, Object?>> get kinds => [
    for (final k in document['kinds']! as List)
      (k! as Map).cast<String, Object?>(),
  ];

  void claim(String name, String what) {
    if (!declared.add(name)) {
      throw FormatException('$what would declare $name twice');
    }
  }

  String kindRef(Map<String, Object?> type, String where) {
    final named = type['kind'];
    final found = named is String ? names[named] : null;
    if (found == null) {
      throw FormatException(
        '$where: the object kind $named is not in this document',
      );
    }
    return found;
  }

  _Arg arg(Map<String, Object?> type, int i, String where) {
    final name = _argName(type['name'], i);
    final t = type['type'];
    final scalar = _scalars[t];
    if (scalar != null) {
      return _Arg(
        name: name,
        native: scalar.$1,
        dart: scalar.$2,
        caller: scalar.$2,
        callee: scalar.$2,
        toC: name,
        fromC: name,
      );
    }
    final pass = type['pass'] ?? 'borrowed';
    switch (t) {
      case 'str':
        return _Arg(
          name: name,
          native: 'guatiao_str',
          dart: 'guatiao_str',
          caller: 'String',
          callee: 'String',
          toC: 'c.str($name)',
          fromC: 's.str($name)',
        );
      case 'bytes':
        imports.add('typed_data');
        return _Arg(
          name: name,
          native: 'guatiao_bytes',
          dart: 'guatiao_bytes',
          caller: 'List<int>',
          callee: 'Uint8List',
          toC: 'c.bytes($name)',
          fromC: 's.bytes($name)',
        );
      case 'bytes_mut':
        imports.add('typed_data');
        return _Arg(
          name: name,
          native: 'guatiao_bytes_mut',
          dart: 'guatiao_bytes_mut',
          caller: 'Uint8List',
          callee: 'Uint8List',
          toC: 'c.bytesMut($name)',
          fromC: 's.bytesMut($name)',
        );
      case 'value' || 'map' || 'list':
        final c = 'guatiao_$t';
        final up = t == 'value'
            ? 'Value'
            : t == 'map'
            ? 'Map'
            : 'List';
        final view = t == 'value'
            ? 'Ref'
            : t == 'map'
            ? 'MapRef'
            : 'ListRef';
        switch (pass) {
          case 'taken':
            imports.addAll(['Value']);
            return _Arg(
              name: name,
              native: 'Pointer<$c>',
              dart: 'Pointer<$c>',
              caller: t == 'value' ? 'Object?' : 'Object',
              callee: 'Value',
              toC: 'c.take$up($name)',
              fromC: 's.taken$up($name)',
              taken: true,
            );
          case 'lent':
            imports.addAll(['Ref', view]);
            return _Arg(
              name: name,
              native: 'Pointer<$c>',
              dart: 'Pointer<$c>',
              caller: 'Ref',
              callee: view,
              toC: 'c.lend$up($name)',
              fromC: 's.lent$up($name)',
            );
          case 'borrowed' when t == 'value':
            final nullable = type['nullable'] == true;
            imports.add('Ref');
            return _Arg(
              name: name,
              native: 'Pointer<$c>',
              dart: 'Pointer<$c>',
              caller: 'Object?',
              callee: nullable ? 'Ref?' : 'Ref',
              toC: 'c.borrowValue($name)',
              fromC: nullable
                  ? 's.borrowedValue($name, nullable: true)'
                  : 's.borrowedValue($name)!',
            );
          case 'borrowed' when t == 'map':
            imports.add('MapRef');
            return _Arg(
              name: name,
              native: 'Pointer<$c>',
              dart: 'Pointer<$c>',
              caller: 'Object',
              callee: 'MapRef',
              toC: 'c.borrowMap($name)',
              fromC: 's.borrowedMap($name)',
            );
        }
        throw FormatException('$where: a $t cannot be passed $pass');
      case 'object':
        final k = kindRef(type, where);
        if (pass == 'taken') {
          return _Arg(
            name: name,
            native: 'guatiao_object',
            dart: 'guatiao_object',
            caller: k,
            callee: '${k}Remote',
            toC: 'c.takeObject($k.kind, $name)',
            fromC: 's.takenObject($k.kind, $name)',
            taken: true,
          );
        }
        if (pass == 'lent') {
          return _Arg(
            name: name,
            native: 'Pointer<guatiao_object>',
            dart: 'Pointer<guatiao_object>',
            caller: k,
            callee: '${k}Remote',
            toC: 'c.lendObject($k.kind, $name)',
            fromC: 's.lentObject($k.kind, $name)',
          );
        }
        throw FormatException('$where: an object is passed taken or lent');
    }
    throw FormatException('$where: $t is not a type an argument can have');
  }

  _Answer answer(Object? given, String where) {
    if (given == null) return _none;
    final type = (given as Map).cast<String, Object?>();
    final t = type['type'];
    final scalar = _scalars[t];
    if (scalar != null) {
      return _Answer(
        pointee: scalar.$1,
        caller: scalar.$2,
        callee: scalar.$2,
        outSlot: 'c.arena<${scalar.$1}>()',
        read: 'out.value',
        write: 'out.value = result;',
      );
    }
    switch (t) {
      case 'text':
        return const _Answer(
          pointee: 'guatiao_string',
          caller: 'String',
          callee: 'String',
          outSlot: 'c.textOut()',
          read: 'c.textAnswer(out)',
          write: 's.textAnswer(out, result);',
        );
      case 'value' || 'map' || 'list':
        imports.add('Value');
        final up = t == 'value'
            ? 'Value'
            : t == 'map'
            ? 'Map'
            : 'List';
        return _Answer(
          pointee: 'guatiao_$t',
          caller: 'Value',
          callee: t == 'value' ? 'Object?' : 'Object',
          outSlot: 'c.${t}Out()',
          read: 'c.${up.toLowerCase()}Answer(out)',
          write: 's.${up.toLowerCase()}Answer(out, result);',
        );
      case 'object':
        final k = kindRef(type, where);
        return _Answer(
          pointee: 'guatiao_object',
          caller: '${k}Remote',
          callee: k,
          outSlot: 'c.objectOut()',
          read: 'c.objectAnswer($k.kind, out)',
          write: 's.objectAnswer($k.kind, out, result);',
        );
    }
    throw FormatException('$where: an answer cannot be $t');
  }

  String generate({String? source, String? command}) {
    if (document['format'] != 1 || document['kinds'] is! List) {
      throw const FormatException(
        'a kinds document is {"format": 1, "kinds": [...]}',
      );
    }
    for (final kind in kinds) {
      final name = kind['name'];
      if (name is! String) throw const FormatException('a kind has a name');
      final type = typeNameOf(name);
      if (names.containsValue(type)) {
        throw FormatException('two kinds are $type in Dart');
      }
      names[name] = type;
    }
    for (final MapEntry(key: kind, value: uri) in kindImports.entries) {
      if (names.containsKey(kind)) {
        throw FormatException(
          '$kind is in this document and imported from $uri',
        );
      }
      final type = typeNameOf(kind);
      if (names.containsValue(type)) {
        throw FormatException('two kinds are $type in Dart');
      }
      names[kind] = type;
    }
    final body = <String>[for (final kind in kinds) ...declareKind(kind)];
    final libraries = kindImports.values.toSet().toList()..sort();
    final text = body.join('\n');
    final structs = {
      for (final m in RegExp(r'\bguatiao_[a-z_]+\b').allMatches(text)) m[0]!,
    }.toList()..sort();
    final shown = imports.where((i) => i != 'typed_data').toList()..sort();
    final lines = [
      '// GENERATED by `dart run guatiao:kind`'
          '${source == null ? '' : ' from $source'}. DO NOT EDIT.',
      if (command != null) '// Regenerate: $command',
      '',
      '// The C structs and slot typedefs keep the header\'s spelling.',
      '// ignore_for_file: camel_case_types',
      '',
      "import 'dart:ffi';",
      if (imports.contains('typed_data')) "import 'dart:typed_data';",
      '',
      "import 'package:guatiao/ffi.dart'",
      '    show',
      for (final (i, s) in structs.indexed)
        '        $s${i == structs.length - 1 ? ';' : ','}',
      if (shown.isNotEmpty)
        "import 'package:guatiao/guatiao.dart' show ${shown.join(', ')};",
      "import 'package:guatiao/kind_glue.dart';",
      "import 'package:guatiao/kinds.dart';",
      if (libraries.isNotEmpty) '',
      for (final uri in libraries) "import '$uri';",
      '',
      ...body,
    ];
    return '${lines.join('\n').trimRight()}\n';
  }

  List<String> doc(
    Object? text, [
    String indent = '',
    List<String> extra = const [],
  ]) {
    final lines = [
      if (text is String && text.isNotEmpty) ...text.split('\n'),
      if (text is String && text.isNotEmpty && extra.isNotEmpty) '',
      ...extra,
    ];
    return [
      for (final line in lines)
        line.isEmpty ? '$indent///' : '$indent/// $line',
    ];
  }

  List<String> declareKind(Map<String, Object?> kind) {
    final kindName = kind['name']! as String;
    final n = names[kindName]!;
    final object = kind['object'] == true;
    final floorHash = kind['floor_hash'];
    if (floorHash is! int) throw FormatException('$kindName: a floor_hash');
    final slots = [
      for (final s in kind['slots']! as List)
        (s! as Map).cast<String, Object?>(),
    ];
    claim(n, kindName);
    claim('${n}Remote', kindName);
    claim('${n}Kind', kindName);
    final methods = <String>{};
    final required = <String>[];
    final appended = <String>[];
    final remote = <String>[];
    final kindCase = <String>[];
    final typeCase = <String>[];
    final shimCase = <String>[];
    final shims = <String>[];
    final typedefs = <String>[];
    final slotSpecs = <String>[];
    for (var i = 0; i < slots.length; i++) {
      final slot = slots[i];
      final slotName = slot['name']! as String;
      final where = '$kindName.$slotName';
      final m = memberNameOf(slotName);
      if (_reserved.contains(m) || _keywords.contains(m) || !methods.add(m)) {
        throw FormatException(
          '$where: $m is taken in Dart (a remote member, a keyword, or '
          'another slot)',
        );
      }
      final isRequired = slot['required'] == true;
      final fallible = slot['fallible'] == true;
      final args = [
        for (final (j, a) in (slot['args'] as List? ?? const []).indexed)
          arg((a! as Map).cast<String, Object?>(), j, '$where argument $j'),
      ];
      final ans = answer(slot['answer'], '$where answer');
      final iface = isRequired ? n : '$n${m[0].toUpperCase()}${m.substring(1)}';
      final c = '_$n${m[0].toUpperCase()}${m.substring(1)}';
      final cParams = [
        'Pointer<Void> ctx',
        for (final a in args) '${a.dart} ${a.name}',
        if (ans.pointee.isNotEmpty) 'Pointer<${ans.pointee}> out',
        if (fallible) 'Pointer<guatiao_provider_error> err',
      ];
      final nativeTypes = [
        'Pointer<Void>',
        for (final a in args) a.native,
        if (ans.pointee.isNotEmpty) 'Pointer<${ans.pointee}>',
        if (fallible) 'Pointer<guatiao_provider_error>',
      ];
      final dartTypes = [
        'Pointer<Void>',
        for (final a in args) a.dart,
        if (ans.pointee.isNotEmpty) 'Pointer<${ans.pointee}>',
        if (fallible) 'Pointer<guatiao_provider_error>',
      ];
      typedefs
        ..add('typedef ${c}C = Uint32 Function(${nativeTypes.join(', ')});')
        ..add('typedef ${c}D = int Function(${dartTypes.join(', ')});');
      slotSpecs.add(
        "KindSlot('$slotName'${isRequired ? ', required: true' : ''}"
        "${fallible ? ', fallible: true' : ''})",
      );

      // The method a provider implements.
      final calleeSig =
          '${ans.callee} $m(${args.map((a) => '${a.callee} ${a.name}').join(', ')});';
      if (isRequired) {
        required
          ..addAll(doc(slot['doc'], '  '))
          ..add('  $calleeSig');
      } else {
        claim(iface, where);
        appended
          ..addAll(
            doc(slot['doc'], '', [
              '`$slotName` of the `$kindName` kind: a slot appended after its first ones, which an older table may lack.',
            ]),
          )
          ..add('abstract interface class $iface {')
          ..add('  $calleeSig')
          ..add('}')
          ..add('');
      }

      // The caller's method: arguments built, handed-over ones last.
      final ordered = [
        ...args.where((a) => !a.taken),
        ...args.where((a) => a.taken),
      ];
      remote
        ..addAll(
          doc(slot['doc'], '  ', [
            if (!isRequired)
              'Throws `Unsupported` when the table lacks it: `has(\'$slotName\')` asks first.',
          ]),
        )
        ..add('  @override')
        ..add(
          '  ${ans.caller} $m(${args.map((a) => '${a.caller} ${a.name}').join(', ')}) {',
        )
        ..add('    final c = CallScope();')
        ..add('    try {')
        ..addAll([
          for (final a in ordered) '      final ${a.name}\$ = ${a.toC};',
        ])
        ..addAll([
          if (ans.pointee.isNotEmpty) '      final out = ${ans.outSlot};',
        ])
        ..addAll([if (fallible) '      final err = c.errorOut();'])
        ..add('      final status = slot($i)')
        ..add('          .cast<NativeFunction<${c}C>>()')
        ..add('          .asFunction<${c}D>()(')
        ..add(
          '            ${['ctx', for (final a in args) '${a.name}\$', if (ans.pointee.isNotEmpty) 'out', if (fallible) 'err'].join(', ')},',
        )
        ..add('          );')
        ..add(
          "      c.check(status, ${fallible ? 'err' : 'null'}, '$slotName');",
        )
        ..addAll([if (ans.pointee.isNotEmpty) '      return ${ans.read};'])
        ..add('    } finally {')
        ..add('      c.close();')
        ..add('    }')
        ..add('  }')
        ..add('');

      kindCase.add('    $i => target is $iface,');
      typeCase.add('    $i => <T>[] is List<$iface>,');
      shimCase.add('    $i => _serve$c.cast(),');

      // The shim a C caller reaches: handed-over arguments taken first, so
      // a refusal still frees them.
      final taken = args.where((a) => a.taken);
      final others = args.where((a) => !a.taken);
      final call = 'target.$m(${args.map((a) => '${a.name}\$').join(', ')})';
      shims
        ..add(
          'final Pointer<NativeFunction<${c}C>> _serve$c = Pointer.fromFunction<${c}C>(',
        )
        ..add('  _serve${c}Body,')
        ..add('  100,')
        ..add(');')
        ..add('')
        ..add('int _serve${c}Body(${cParams.join(', ')}) {')
        ..add("  final s = ServeScope('$where', ${fallible ? 'err' : 'null'});")
        ..add('  try {')
        ..addAll([for (final a in taken) '    final ${a.name}\$ = ${a.fromC};'])
        ..add('    final target = s.held<$iface>(ctx);')
        ..addAll([
          for (final a in others) '    final ${a.name}\$ = ${a.fromC};',
        ])
        ..add('    s.running();')
        ..addAll(
          ans.pointee.isEmpty
              ? ['    $call;']
              : ['    final result = $call;', '    ${ans.write}'],
        )
        ..add('  } on Object catch (e) {')
        ..add('    s.fail(e);')
        ..add('  }')
        ..add('  return s.close();')
        ..add('}')
        ..add('');
    }
    final optional = [
      for (final s in slots)
        if (s['required'] != true)
          '$n${memberNameOf(s['name']! as String)[0].toUpperCase()}${memberNameOf(s['name']! as String).substring(1)}',
    ];
    return [
      ...doc(kind['doc'], '', [
        'The `$kindName` kind: what ${object ? 'an object of it' : 'a provider of it'} '
            'implements${optional.isEmpty ? '.' : ', and each appended slot an interface of its own (${optional.map((o) => '[$o]').join(', ')}).'}',
      ]),
      'abstract interface class $n {',
      '  /// The kind: what a provider lists and a host binds with.',
      '  static final ${n}Kind kind = ${n}Kind._();',
      if (required.isNotEmpty) '',
      ...required,
      '}',
      '',
      ...appended,
      '/// A caller\'s handle on a `$kindName` ${object ? 'object' : 'table'}: a method per slot.',
      'final class ${n}Remote extends KindRemote',
      '    implements ${[n, ...optional].join(', ')} {',
      '  ${n}Remote._(',
      '    ${n}Kind super.kind,',
      '    super.table,',
      '    super.size,',
      '    super.ctx, {',
      '    super.scope,',
      '    super.owned,',
      '  });',
      '',
      ...remote,
      '}',
      '',
      '/// The `$kindName` kind as data: its slots, and the glue that calls and serves them.',
      'final class ${n}Kind extends Kind<${n}Remote> {',
      '  ${n}Kind._()',
      '    : super(',
      "        name: '$kindName',",
      '        floorHash: $floorHash,',
      '        isObject: $object,',
      '        slots: const [',
      ...[for (final s in slotSpecs) '          $s,'],
      '        ],',
      '      );',
      '',
      '  @override',
      '  ${n}Remote remote(',
      '    Pointer<Void> table,',
      '    int size,',
      '    Pointer<Void> ctx, {',
      '    SlotCall? scope,',
      '    bool owned = false,',
      '  }) => ${n}Remote._(this, table, size, ctx, scope: scope, owned: owned);',
      '',
      '  @override',
      '  bool slotServedBy(Object target, int index) => switch (index) {',
      ...kindCase,
      '    _ => false,',
      '  };',
      '',
      '  @override',
      '  bool slotServedByType<T>(int index) => switch (index) {',
      ...typeCase,
      '    _ => false,',
      '  };',
      '',
      '  @override',
      '  Pointer<Void> shim(int index) => switch (index) {',
      ...shimCase,
      '    _ => throw RangeError.index(index, slots),',
      '  };',
      '}',
      '',
      ...typedefs,
      '',
      ...shims,
    ];
  }
}

/// The Dart for [document], a decoded kinds description: per kind an
/// abstract interface class of its required slots with the kind as its
/// static `kind`, one per appended slot, a `<Kind>Remote` a caller holds,
/// and the `<Kind>Kind` that calls and serves a table. [FormatException]
/// for a document it cannot write.
///
/// [imports] names an object kind a slot takes or answers that another
/// document describes, with the library `dart run guatiao:kind` wrote for
/// that document (`{'listener': 'listener_kind.dart'}`): it is imported,
/// not written again.
String generateKinds(
  Map<String, Object?> document, {
  String? source,
  String? command,
  Map<String, String> imports = const {},
}) => _Generator(document, imports).generate(source: source, command: command);

/// The top-level scalars of a `pubspec.yaml` this needs: `name`,
/// `version`, `description` (folded or literal blocks too), and the URLs.
Map<String, String> _pubspec(String yaml) {
  final out = <String, String>{};
  final lines = const LineSplitter().convert(yaml);
  for (var i = 0; i < lines.length; i++) {
    final m = RegExp(r'^([A-Za-z_]+):\s*(.*)$').firstMatch(lines[i]);
    if (m == null) continue;
    var value = m.group(2)!.trim();
    if (value.startsWith('#')) value = '';
    if (value.isEmpty || RegExp(r'^[>|][-+]?$').hasMatch(value)) {
      final literal = value.startsWith('|');
      final block = <String>[];
      while (i + 1 < lines.length &&
          (lines[i + 1].startsWith(' ') || lines[i + 1].trim().isEmpty)) {
        block.add(lines[++i].trim());
      }
      value = literal ? block.join('\n').trim() : block.join(' ').trim();
      if (!literal) value = value.replaceAll(RegExp(' +'), ' ');
    } else if (value.length >= 2 &&
        (value.startsWith("'") && value.endsWith("'") ||
            value.startsWith('"') && value.endsWith('"'))) {
      value = value.substring(1, value.length - 1);
    } else {
      value = value.replaceFirst(RegExp(r'\s+#.*$'), '');
    }
    if (value.isNotEmpty) out[m.group(1)!] = value;
  }
  return out;
}

String _quoted(String text) =>
    "'${text.replaceAll(r'\', r'\\').replaceAll("'", r"\'").replaceAll(r'$', r'\$').replaceAll('\n', r'\n')}'";

/// The Dart for a package's library identity, from its `pubspec.yaml`:
/// a `const LibraryIdentity` named [constant], its id the package's
/// `name`, its version its `version`, its meta the `description` (as
/// `summary`) and the package's URLs.
String generateIdentity(
  String pubspecYaml, {
  String constant = 'packageLibrary',
  String? source,
  String? command,
}) {
  final pubspec = _pubspec(pubspecYaml);
  final name = pubspec['name'];
  final version = pubspec['version'];
  if (name == null) throw const FormatException('the pubspec names no package');
  if (version == null) {
    throw const FormatException(
      "the pubspec's version is the library's version, and it has none",
    );
  }
  final urls = <String, String>{
    if (pubspec['homepage'] != null) 'Homepage': pubspec['homepage']!,
    if (pubspec['repository'] != null) 'Repository': pubspec['repository']!,
    if (pubspec['issue_tracker'] != null) 'Issues': pubspec['issue_tracker']!,
    if (pubspec['documentation'] != null)
      'Documentation': pubspec['documentation']!,
  };
  final meta = <String>[
    if (pubspec['description'] != null)
      "    'summary': ${_quoted(pubspec['description']!)},",
    if (urls.isNotEmpty) "    'urls': <String, Object?>{",
    for (final e in urls.entries)
      '      ${_quoted(e.key)}: ${_quoted(e.value)},',
    if (urls.isNotEmpty) '    },',
  ];
  return [
    '// GENERATED by `dart run guatiao:kind identity`'
        '${source == null ? '' : ' from $source'}. DO NOT EDIT.',
    if (command != null) '// Regenerate: $command',
    '',
    "import 'package:guatiao/kinds.dart';",
    '',
    '/// This package as a guatiao library: the id and version its',
    '/// `pubspec.yaml` names, and what it says about itself.',
    'const LibraryIdentity $constant = LibraryIdentity(',
    '  ${_quoted(name)},',
    '  ${_quoted(version)},',
    if (meta.isNotEmpty) ...['  meta: <String, Object?>{', ...meta, '  },'],
    ');',
    '',
  ].join('\n');
}
