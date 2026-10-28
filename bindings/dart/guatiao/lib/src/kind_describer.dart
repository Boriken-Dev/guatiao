// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Reads kinds declared in Dart (`package:guatiao/describe.dart`) into the
/// description document. A reader of the few forms a declaration uses,
/// not of Dart: anything else is refused with its line and column.
library;

import 'dart:convert';

import 'kind_generator.dart'
    show generateKinds, memberNameOf, reservedMemberNames, typeNameOf;
import 'kind_hash.dart'
    if (dart.library.js_interop) 'web/kind_hash_web.dart'
    show kindFloorHash;
import 'status.dart' show BadValue;

/// The document of the kinds [source] declares: every `abstract interface
/// class`, in the order written, as `{"kinds": [...]}`. An
/// object kind a slot takes or answers is one of them.
/// [file] names the source in a refusal.
///
/// A [FormatException] names the place that is not a kind.
Map<String, Object?> describeDeclarations(String source, {String? file}) {
  final name = file ?? 'the declaration';
  final document = _Describer(
    _Parser(_lex(source, name), name).declarations(),
    name,
  ).document();
  // What the document is for: the Dart, and every other language, it writes.
  try {
    generateKinds(document);
  } on FormatException catch (e) {
    throw FormatException(
      '$name: these kinds would not be written as Dart: ${e.message}',
    );
  }
  return document;
}

/// [describeDeclarations] as the JSON text a kind crate commits: two
/// spaces of indent, and a newline at the end.
String describedText(Map<String, Object?> document) =>
    '${const JsonEncoder.withIndent('  ').convert(document)}\n';

/// The floor hash of a kind born outside Rust, computed by the library
/// (`guatiao_kind_floor_hash`): FNV-1a (32-bit) of `provider` or `object`
/// and every slot in table order, each as `name(argument types)->answer`
/// spelled as `#[guatiao::kind]` spells them, so a declaration repeating a
/// trait's slots hashes as the trait does. An object is spelled by its
/// kind's name in PascalCase, as its trait would be named. Loads the
/// library (the module, on the web).
int floorHashOf(bool object, List<Map<String, Object?>> slots) =>
    kindFloorHash(jsonEncode({'object': object, 'slots': slots}));

/// Has the library read each kind of [document] (`guatiao_kind_floor_hash`
/// reads one as its document reader does), so what no kind may declare, such
/// as `bytes_mut` on a kind that is not an object kind, is refused before
/// anything is written from it. Loads the library.
///
/// A [FormatException] names the kind and the place.
void checkKinds(Map<String, Object?> document) {
  final kinds = document['kinds'];
  if (kinds is! List) {
    throw const FormatException('a document is {"kinds": [...]}');
  }
  for (final (i, kind) in kinds.indexed) {
    try {
      kindFloorHash(jsonEncode(kind));
    } on BadValue catch (e) {
      throw FormatException('kinds[$i]: ${e.message}');
    }
  }
}

// ---- reading ---------------------------------------------------------------

enum _T { ident, punct, string, number, doc, end }

final class _Token {
  _Token(this.kind, this.text, this.line, this.column);

  final _T kind;
  final String text;
  final int line;
  final int column;

  String get where => '$line:$column';

  @override
  String toString() => kind == _T.end ? 'the end of the file' : '"$text"';
}

List<_Token> _lex(String s, String file) {
  final out = <_Token>[];
  var i = s.startsWith('\uFEFF') ? 1 : 0, line = 1, lineStart = i;
  Never refuse(String what) =>
      throw FormatException('$file:$line:${i - lineStart + 1}: $what');
  bool isIdent(int c) =>
      c == 0x24 ||
      c == 0x5f ||
      (c >= 0x30 && c <= 0x39) ||
      (c | 0x20) >= 0x61 && (c | 0x20) <= 0x7a;
  while (i < s.length) {
    final c = s.codeUnitAt(i);
    final column = i - lineStart + 1;
    if (c == 0x0a) {
      i++;
      line++;
      lineStart = i;
    } else if (c == 0x20 || c == 0x09 || c == 0x0d) {
      i++;
    } else if (s.startsWith('///', i)) {
      var end = s.indexOf('\n', i);
      if (end < 0) end = s.length;
      var text = s.substring(i + 3, end);
      text = text.trimRight();
      if (text.startsWith(' ')) text = text.substring(1);
      out.add(_Token(_T.doc, text, line, column));
      i = end;
    } else if (s.startsWith('//', i)) {
      final end = s.indexOf('\n', i);
      i = end < 0 ? s.length : end;
    } else if (s.startsWith('/*', i)) {
      var depth = 0;
      while (i < s.length) {
        if (s.startsWith('/*', i)) {
          depth++;
          i += 2;
        } else if (s.startsWith('*/', i)) {
          depth--;
          i += 2;
          if (depth == 0) break;
        } else {
          if (s.codeUnitAt(i) == 0x0a) {
            line++;
            lineStart = i + 1;
          }
          i++;
        }
      }
      if (depth != 0) refuse('a comment that never ends');
    } else if (c == 0x27 ||
        c == 0x22 ||
        (c == 0x72 &&
            i + 1 < s.length &&
            (s[i + 1] == "'" || s[i + 1] == '"'))) {
      final raw = c == 0x72;
      if (raw) i++;
      final quote = s[i];
      if (s.startsWith(quote * 3, i)) refuse('a multi-line string');
      final buf = StringBuffer();
      i++;
      while (true) {
        if (i >= s.length || s[i] == '\n') refuse('a string that never ends');
        final ch = s[i];
        if (ch == quote) break;
        if (!raw && ch == r'$') refuse('interpolation in a string');
        if (!raw && ch == r'\') {
          i++;
          if (i >= s.length) refuse('a string that never ends');
          final e = s[i];
          buf.write(switch (e) {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            _ when "\\'\"\$".contains(e) => e,
            _ => refuse('the escape \\$e'),
          });
          i++;
          continue;
        }
        buf.write(ch);
        i++;
      }
      i++;
      out.add(_Token(_T.string, buf.toString(), line, column));
    } else if (c >= 0x30 && c <= 0x39) {
      final start = i;
      while (i < s.length && (isIdent(s.codeUnitAt(i)))) {
        i++;
      }
      out.add(_Token(_T.number, s.substring(start, i), line, column));
    } else if (isIdent(c)) {
      final start = i;
      while (i < s.length && isIdent(s.codeUnitAt(i))) {
        i++;
      }
      out.add(_Token(_T.ident, s.substring(start, i), line, column));
    } else if ('{}()<>[],;?@.=:'.contains(s[i])) {
      out.add(_Token(_T.punct, s[i], line, column));
      i++;
    } else {
      refuse('"${s[i]}" is not part of a kind declaration');
    }
  }
  out.add(_Token(_T.end, '', line, i - lineStart + 1));
  return out;
}

final class _Type {
  _Type(this.name, this.args, this.nullable, this.at);

  final String name;
  final List<_Type> args;
  final bool nullable;
  final _Token at;

  @override
  String toString() =>
      '$name${args.isEmpty ? '' : '<${args.join(', ')}>'}${nullable ? '?' : ''}';
}

final class _Annotation {
  _Annotation(this.name, this.argument, this.at);

  final String name;
  final _Token? argument;
  final _Token at;
}

final class _Member {
  _Member(this.name, this.doc, this.marks, this.answer, this.params, this.at);

  final String name;
  final String? doc;
  final List<_Annotation> marks;
  final _Type answer;
  final List<(String, _Type)> params;
  final _Token at;
}

final class _Decl {
  _Decl(
    this.className,
    this.doc,
    this.marks,
    this.object,
    this.members,
    this.at,
  );

  final String className;
  final String? doc;
  final List<_Annotation> marks;
  final bool object;
  final List<_Member> members;
  final _Token at;
}

final class _Parser {
  _Parser(this.tokens, this.file);

  final List<_Token> tokens;
  final String file;
  var _at = 0;

  _Token get peek => tokens[_at];

  _Token next() => tokens[_at++];

  Never refuse(_Token at, String what) =>
      throw FormatException('$file:${at.where}: $what');

  bool isPunct(String p) => peek.kind == _T.punct && peek.text == p;

  bool isWord(String w) => peek.kind == _T.ident && peek.text == w;

  _Token expect(String p) {
    if (!isPunct(p)) refuse(peek, '"$p" was expected, not $peek');
    return next();
  }

  String ident(String what) {
    if (peek.kind != _T.ident) refuse(peek, '$what was expected, not $peek');
    return next().text;
  }

  /// A name, an import prefix dropped: `k.U32` is `U32`.
  String name(String what) {
    var n = ident(what);
    if (isPunct('.')) {
      next();
      n = ident(what);
    }
    return n;
  }

  /// The `///` lines and annotations before a declaration, in any order.
  (String?, List<_Annotation>) docAndMarks() {
    final lines = <String>[];
    final marks = <_Annotation>[];
    while (peek.kind == _T.doc || isPunct('@')) {
      if (peek.kind == _T.doc) {
        lines.add(next().text);
      } else {
        marks.add(annotation());
      }
    }
    while (lines.isNotEmpty && lines.last.isEmpty) {
      lines.removeLast();
    }
    while (lines.isNotEmpty && lines.first.isEmpty) {
      lines.removeAt(0);
    }
    return (lines.isEmpty ? null : lines.join('\n'), marks);
  }

  _Annotation annotation() {
    {
      final at = next();
      final n = name('an annotation');
      _Token? argument;
      if (isPunct('(')) {
        next();
        if (peek.kind != _T.string && peek.kind != _T.number) {
          refuse(peek, '@$n takes a literal, not $peek');
        }
        argument = next();
        expect(')');
      }
      return _Annotation(n, argument, at);
    }
  }

  _Type type() {
    final at = peek;
    final n = name('a type');
    final args = <_Type>[];
    if (isPunct('<')) {
      next();
      args.add(type());
      while (isPunct(',')) {
        next();
        args.add(type());
      }
      expect('>');
    }
    var nullable = false;
    if (isPunct('?')) {
      next();
      nullable = true;
    }
    return _Type(n, args, nullable, at);
  }

  List<_Decl> declarations() {
    final decls = <_Decl>[];
    while (peek.kind != _T.end) {
      final (d, marks) = docAndMarks();
      if (isWord('import') ||
          isWord('export') ||
          isWord('library') ||
          isWord('part')) {
        while (!isPunct(';')) {
          if (peek.kind == _T.end) refuse(peek, 'a directive with no ";"');
          next();
        }
        next();
        continue;
      }
      if (peek.kind == _T.end) {
        if (marks.isNotEmpty) {
          refuse(marks.first.at, 'an annotation on nothing');
        }
        break;
      }
      final at = peek;
      for (final w in ['abstract', 'interface', 'class']) {
        if (!isWord(w)) {
          refuse(
            peek,
            'a kinds file declares abstract interface classes, not $peek',
          );
        }
        next();
      }
      final className = ident('the kind\'s class name');
      var object = false;
      if (isWord('implements')) {
        next();
        final n = name('ObjectKind');
        if (n != 'ObjectKind' || isPunct(',')) {
          refuse(
            at,
            '$className implements ObjectKind or nothing: a kind\'s slots are its own methods',
          );
        }
        object = true;
      }
      if (isWord('extends') || isWord('with')) {
        refuse(
          peek,
          '$className extends nothing: a kind\'s slots are its own methods',
        );
      }
      expect('{');
      final members = <_Member>[];
      while (true) {
        final (memberDoc, memberMarks) = docAndMarks();
        if (isPunct('}')) {
          if (memberMarks.isNotEmpty) {
            refuse(memberMarks.first.at, 'an annotation on nothing');
          }
          next();
          break;
        }
        final memberAt = peek;
        if (isWord('static') ||
            isWord('final') ||
            isWord('const') ||
            isWord('get') ||
            isWord('set') ||
            isWord('var')) {
          refuse(
            peek,
            'a kind\'s class has methods alone, not ${peek.text} members',
          );
        }
        final answer = isWord('void')
            ? _Type(next().text, const [], false, memberAt)
            : type();
        if ((isWord('get') || isWord('set') || isWord('operator')) &&
            tokens[_at + 1].kind == _T.ident) {
          refuse(
            peek,
            'a kind\'s class has methods alone, not ${peek.text} members',
          );
        }
        final memberName = ident('a method name');
        expect('(');
        final params = <(String, _Type)>[];
        while (!isPunct(')')) {
          if (isPunct('[') || isPunct('{')) {
            refuse(
              peek,
              '$memberName takes plain arguments, not optional or named ones',
            );
          }
          if (isPunct('@')) {
            refuse(
              peek,
              'an argument carries no annotation: its type says how it passes',
            );
          }
          final t = type();
          params.add((ident('an argument name'), t));
          if (!isPunct(',')) break;
          next();
        }
        expect(')');
        if (isPunct('{') || isPunct('=')) {
          refuse(peek, '$memberName is declared, not defined: end it with ";"');
        }
        expect(';');
        members.add(
          _Member(memberName, memberDoc, memberMarks, answer, params, memberAt),
        );
      }
      decls.add(_Decl(className, d, marks, object, members, at));
    }
    return decls;
  }
}

// ---- describing ------------------------------------------------------------

/// `GreetInto` / `greetInto` as `greet_into`; capitals in a row refused, as
/// no name written from a description comes back to them.
String _snake(String name, Never Function(String) refuse) {
  final b = StringBuffer();
  for (var i = 0; i < name.length; i++) {
    final c = name[i];
    final upper = c != c.toLowerCase();
    if (upper) {
      if (i > 0 && name[i - 1] != name[i - 1].toLowerCase()) {
        refuse(
          '$name has capitals in a row, so no name comes back to it: spell it parseHttp for parse_http',
        );
      }
      if (i > 0) b.write('_');
      b.write(c.toLowerCase());
    } else {
      b.write(c);
    }
  }
  return b.toString();
}

const _scalars = {
  'bool': 'bool',
  'int': 'i64',
  'double': 'f64',
  'I8': 'i8',
  'I16': 'i16',
  'I32': 'i32',
  'I64': 'i64',
  'Isize': 'isize',
  'U8': 'u8',
  'U16': 'u16',
  'U32': 'u32',
  'U64': 'u64',
  'Usize': 'usize',
  'F32': 'f32',
  'F64': 'f64',
};

const _views = {'Ref': 'value', 'MapRef': 'map', 'ListRef': 'list'};

/// The names a declaration's types are read by, which no kind may take.
final _marked = {
  ..._scalars.keys,
  'Ref',
  'MapRef',
  'ListRef',
  'Value',
  'Lent',
  'Taken',
  'ValueAnswer',
  'MapAnswer',
  'ListAnswer',
  'BytesMut',
  'ObjectKind',
  'String',
  'Uint8List',
};

final class _Describer {
  _Describer(this.decls, this.file) {
    for (final d in decls) {
      if (_marked.contains(d.className)) {
        refuse(
          d.at,
          '${d.className} is one of the marks a declaration is spelled with: name the kind otherwise',
        );
      }
      final name = kindName(d);
      if (byClass.containsKey(d.className) || names.contains(name)) {
        refuse(d.at, 'two kinds are ${d.className} ($name)');
      }
      byClass[d.className] = d;
      names.add(name);
    }
  }

  final List<_Decl> decls;
  final String file;
  final Map<String, _Decl> byClass = {};
  final Set<String> names = {};

  Never refuse(_Token at, String what) =>
      throw FormatException('$file:${at.where}: $what');

  /// The one annotation of [d] named [name], or null.
  _Annotation? mark(_Decl d, String name) {
    final found = d.marks.where((m) => m.name == name).toList();
    if (found.length > 1) refuse(found[1].at, '@$name is written twice');
    return found.isEmpty ? null : found.single;
  }

  String kindName(_Decl d) {
    final named = mark(d, 'KindName');
    if (named != null) {
      final arg = named.argument;
      if (arg == null || arg.kind != _T.string || arg.text.isEmpty) {
        refuse(named.at, "@KindName takes the kind's name as a string");
      }
      return arg.text;
    }
    final name = _snake(d.className, (what) => refuse(d.at, what));
    if (typeNameOf(name) != d.className) {
      refuse(
        d.at,
        '${d.className} is not a kind\'s name in PascalCase: name it as $name is written (${typeNameOf(name)}), or say @KindName',
      );
    }
    return name;
  }

  Map<String, Object?> document() => {
    'kinds': [for (final d in decls) describe(d)],
  };

  Map<String, Object?> describe(_Decl d) {
    for (final m in d.marks) {
      if (m.name != 'KindName' && m.name != 'FloorHash') {
        refuse(
          m.at,
          '@${m.name} is not a mark of a kind (@KindName, @FloorHash)',
        );
      }
    }
    final slots = <Map<String, Object?>>[];
    for (final m in d.members) {
      final where = '${d.className}.${m.name}';
      final slotName = _snake(m.name, (what) => refuse(m.at, what));
      if (memberNameOf(slotName) != m.name) {
        refuse(
          m.at,
          '$where: no slot\'s name comes back to ${m.name}: name it as $slotName is written (${memberNameOf(slotName)})',
        );
      }
      if (slotName == 'destroy') {
        refuse(
          m.at,
          '$where: destroy is the slot an object kind\'s table starts with, and no other',
        );
      }
      if (reservedMemberNames.contains(m.name)) {
        refuse(
          m.at,
          '$where: ${m.name} is a remote\'s own, so no slot may be called it',
        );
      }
      if (slots.any((s) => s['name'] == slotName)) {
        refuse(m.at, '$where: two slots of ${d.className} are $slotName');
      }
      final argNames = <String>{};
      for (final (name, t) in m.params) {
        if (!argNames.add(_snake(name, (what) => refuse(t.at, what)))) {
          refuse(t.at, '$where: two arguments are $name');
        }
      }
      var fallibleMark = false, optionalMark = false;
      for (final a in m.marks) {
        if (a.argument != null) refuse(a.at, '@${a.name} takes no argument');
        switch (a.name) {
          case 'fallible':
            fallibleMark = true;
          case 'optional':
            optionalMark = true;
          default:
            refuse(
              a.at,
              '@${a.name} is not a mark of a slot (@fallible, @optional)',
            );
        }
      }
      final slot = <String, Object?>{
        'name': slotName,
        'required': !optionalMark,
        'fallible': fallibleMark,
        'args': [
          for (final (name, t) in m.params) arg(name, t, '$where($name)'),
        ],
      };
      final answer = this.answer(m.answer, where);
      if (answer != null) slot['answer'] = answer;
      if (m.doc != null) slot['doc'] = m.doc;
      slots.add(slot);
    }
    // The library reads the kind whether or not its hash is pinned: what no
    // kind may declare is refused there.
    int floor;
    try {
      floor = floorHashOf(d.object, slots);
    } on BadValue catch (e) {
      refuse(d.at, '${d.className}: ${e.message}');
    }
    final pinned = mark(d, 'FloorHash');
    if (pinned != null) {
      final arg = pinned.argument;
      final n = arg == null || arg.kind != _T.number
          ? null
          : int.tryParse(arg.text);
      if (n == null || n < 0 || n > 0xffffffff) {
        refuse(pinned.at, '@FloorHash takes a 32-bit unsigned integer');
      }
      floor = n;
    }
    return {
      'name': kindName(d),
      'floor_hash': floor,
      'object': d.object,
      if (d.doc != null) 'doc': d.doc,
      'slots': slots,
    };
  }

  _Decl objectKind(_Type t, String where) {
    final d = byClass[t.name]!;
    if (!d.object) {
      refuse(
        t.at,
        '$where: ${t.name} is a kind but no object kind (it does not implement ObjectKind)',
      );
    }
    return d;
  }

  Map<String, Object?> arg(String name, _Type t, String where) {
    final out = <String, Object?>{
      'name': _snake(name, (what) => refuse(t.at, '$where: $what')),
    };
    void plain() {
      if (t.args.isNotEmpty || t.nullable) {
        refuse(t.at, '$where: $t is not a type an argument can have');
      }
    }

    final scalar = _scalars[t.name];
    if (scalar != null) {
      plain();
      return out..['type'] = scalar;
    }
    switch (t.name) {
      case 'String':
        plain();
        return out..['type'] = 'str';
      case 'Uint8List':
        plain();
        return out..['type'] = 'bytes';
      case 'BytesMut':
        plain();
        return out..['type'] = 'bytes_mut';
      case 'Ref' || 'MapRef' || 'ListRef':
        if (t.args.isNotEmpty || (t.nullable && t.name != 'Ref')) {
          refuse(t.at, '$where: only a Ref may be null');
        }
        out['type'] = _views[t.name];
        out['pass'] = 'borrowed';
        if (t.nullable) out['nullable'] = true;
        return out;
      case 'Lent' || 'Taken':
        final pass = t.name == 'Lent' ? 'lent' : 'taken';
        if (t.args.length != 1 ||
            t.nullable ||
            t.args.single.args.isNotEmpty ||
            t.args.single.nullable) {
          refuse(
            t.at,
            '$where: ${t.name}<T> takes a Ref, MapRef, ListRef or an object kind',
          );
        }
        final inner = t.args.single;
        final view = _views[inner.name];
        if (view != null) return out..addAll({'type': view, 'pass': pass});
        if (byClass.containsKey(inner.name)) {
          final d = objectKind(inner, where);
          return out
            ..addAll({'type': 'object', 'kind': kindName(d), 'pass': pass});
        }
        refuse(
          inner.at,
          '$where: ${t.name}<${inner.name}>: ${inner.name} is no tree view or kind of this file',
        );
      case 'Value':
        refuse(
          t.at,
          '$where: a tree handed over is Taken<Ref>, Taken<MapRef> or Taken<ListRef>',
        );
    }
    if (byClass.containsKey(t.name)) {
      refuse(
        t.at,
        '$where: an object is passed Taken<${t.name}> or Lent<${t.name}>',
      );
    }
    refuse(t.at, '$where: $t is not a type an argument can have');
  }

  Map<String, Object?>? answer(_Type t, String where) {
    if (t.nullable) {
      refuse(
        t.at,
        '$where: an answer is not nullable: ValueAnswer for a tree that may be null',
      );
    }
    if (t.args.isNotEmpty) {
      refuse(t.at, '$where: $t is not a type a slot can answer');
    }
    final scalar = _scalars[t.name];
    if (scalar != null) return {'type': scalar};
    switch (t.name) {
      case 'void':
        return null;
      case 'String':
        return {'type': 'text'};
      case 'ValueAnswer':
        return {'type': 'value'};
      case 'MapAnswer':
        return {'type': 'map'};
      case 'ListAnswer':
        return {'type': 'list'};
    }
    if (byClass.containsKey(t.name)) {
      final d = objectKind(t, where);
      return {'type': 'object', 'kind': kindName(d)};
    }
    refuse(
      t.at,
      '$where: $t is not a type a slot can answer (a tree is ValueAnswer, MapAnswer or ListAnswer)',
    );
  }
}
