// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `greeter_vtable` as dart:ffi, written from the greeter kind's own C
/// header (`examples/greeter_kind/include/greeter_kind.h`).
library;

import 'dart:ffi';

import 'package:guatiao/src/bindings.g.dart';

typedef GreetNative = Uint32 Function(
  Pointer<Void> ctx,
  guatiao_str name,
  Pointer<guatiao_map> out,
  Pointer<guatiao_provider_error> err,
);

typedef GreetDart = int Function(
  Pointer<Void> ctx,
  guatiao_str name,
  Pointer<guatiao_map> out,
  Pointer<guatiao_provider_error> err,
);

typedef ShoutNative = Uint32 Function(
  Pointer<Void> ctx,
  guatiao_str name,
  Pointer<guatiao_string> out,
);

typedef StartNative = Uint32 Function(
  Pointer<Void> ctx,
  guatiao_object listener,
  Pointer<guatiao_object> out,
  Pointer<guatiao_provider_error> err,
);

final class GreeterVtable extends Struct {
  external guatiao_kind_header header;

  external Pointer<NativeFunction<GreetNative>> greet;

  external Pointer<NativeFunction<ShoutNative>> shout;

  external Pointer<NativeFunction<StartNative>> start;
}

/// `examples/hello_library`'s own vtable, written by hand and unrelated
/// to `#[guatiao::kind]` -- hence no `guatiao_kind_header`.
final class HelloGreeterVtable extends Struct {
  /// The C field is `struct_size`; only the layout has to match.
  @Uint32()
  external int structSize;

  external Pointer<
      NativeFunction<
          Uint32 Function(
            Pointer<Void> ctx,
            Pointer<guatiao_value> config,
            Pointer<guatiao_value> out,
          )>> greet;

  external Pointer<NativeFunction<Int64 Function(Pointer<Void> ctx)>>
      outstanding;
}
