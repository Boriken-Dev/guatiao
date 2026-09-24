# Dart API

The `guatiao` package on pub.dev: `dart:ffi` bindings over the C ABI,
with `package:ffi` as the only dependency and nothing to compile. It is a
plain Dart package and does **not** depend on Flutter, which is what lets
it serve a Flutter app and a command-line program alike.

Importing it never loads a native library; the first call that needs one
resolves it from `GUATIAO_LIBRARY`, then `libraryDirectory`, then the
platform's own search.

**[Generated API reference →](https://boriken-dev.github.io/guatiao/dart/index.html)**,
built by `dart doc`.

## Libraries

| Library | Purpose |
| --- | --- |
| `package:guatiao/guatiao.dart` | `Value`, `Ref`, `MapRef`, `ListRef`, `absent`, `Registry`, `Instance`, `kindTable`, the error classes |
| `package:guatiao/serde.dart` | `loads` / `dumps` for JSON, TOML and YAML |
| `package:guatiao/intake.dart` | `check`, `layout`, `isVisible` |

## Values

```dart
import 'package:guatiao/guatiao.dart';

final v = Value.fromDart({'host': '10.0.0.1', 'port': 5900});
final m = v.asMap();
m['port'] = 5901;
m['port']!.toDart();                 // 5901
v.toDart();                          // back to a plain Map
v.close();
```

A `Value` owns its tree and `close()` is required. `m['port']` is a
`Ref`: a path into the tree, walked again on every access, so it stays
valid while the tree grows around it.

A number keeps its exact text. `Value.number('1.10').toDart(numbers:
Numbers.text)` is `'1.10'`, and an integer of any size survives as a
`BigInt`.

## Hosting plugins

```dart
final reg = Registry('my-host', '1.0');
reg.scanDir('/opt/myapp/plugins', rules: 'kind=greeter');
for (final p in reg.providers('greeter')) {
  print('${p['key']} ${p['version']}');
}
final instance = reg.create('acme_shouter', {'prefix': 'hey'});
instance.close();
reg.close();
```

Every answer that is data comes back as a plain `Map` or `List`, already
freed. To call a provider, describe its table with the `Struct` the
kind's C header declares and hand it to `kindTable`, which checks the
size and the floor hash before casting.

## JSON, TOML, YAML and forms

```dart
import 'package:guatiao/serde.dart' as serde;
import 'package:guatiao/intake.dart' as form;

final value = serde.loads('{"port": 5900}');
final text = serde.dumps(value, format: Format.yaml);
final problem = form.check(schema, formDoc);   // null when the form fits
```

A format the loaded library does not export throws `UnsupportedError`
naming it.

The package's own
[`README`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/README.md)
and
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/AGENTS.md)
carry the rest. The C ABI is described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).
