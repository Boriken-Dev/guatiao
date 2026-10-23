# Providers written in Dart

`dart_greeter/` is a guatiao library written in Dart: a package whose
`greeterLibrary()` offers

- `Hello`, its one instance, serving the `greeter` and `counter` kinds;
  its `start` hands back a conversation, a Dart object that tells the
  listener it was handed;
- `Echo`, serving the `wide` kind;
- `Shouter`, an instance per configuration the host checked against its
  schema.

Each provider is a class extending `Provider` and implementing the
interfaces of the kinds it serves, from
[`../greeter_kind/dart`](../greeter_kind/dart/), which
`dart run guatiao:kind` wrote from the kinds' description; nothing here
says what a slot takes, and the analyzer checks every method. The
library's id and version are the package's: `lib/src/library_identity.g.dart`
is what `dart run guatiao:kind identity` wrote from `pubspec.yaml`.

```dart
import 'package:dart_greeter/dart_greeter.dart';
import 'package:guatiao/guatiao.dart';

final library = greeterLibrary();
final reg = Registry('my-host', '1.0')..registerLibrary(library);
```

A host in the same process, written in any language, then calls these
greeters through the registry's tables or its host services. The example
host, [`../dart_host`](../dart_host/), does both.
