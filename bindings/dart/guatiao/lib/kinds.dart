// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Kinds and providers as classes, on a native platform.
///
/// A kind is never declared by hand: `dart run guatiao:kind` writes, from
/// the description a kind crate commits beside its C header, an abstract
/// interface class per kind (its required slots) and per appended slot,
/// a `<Kind>Remote` a caller holds, and a [Kind] reached as the
/// interface's static `kind`. A provider is a class extending [Provider]
/// and implementing the interfaces of what it serves; a [Library] offers
/// it, and `Registry.registerLibrary` files the library.
///
/// ```dart
/// class Hello extends Provider implements Greeter, GreeterShout {
///   @override
///   Map<String, Object?> greet(String name) => {'greeting': 'hello, $name'};
///   @override
///   String shout(String name) => 'HELLO, ${name.toUpperCase()}';
/// }
///
/// final library = Library.of(packageLibrary, offers: [
///   Offer.single(Hello(), named: 'hello', kinds: [Greeter.kind]),
/// ]);
/// ```
///
/// Not on the web: a kind table is native code's.
library;

export 'src/bindings.g.dart' show guatiao_host_info, guatiao_library_info;
export 'src/kind_runtime.dart'
    show Disposable, Kind, KindRemote, KindSlot, liveObjects;
export 'src/kinds.dart' show FloorMismatch, kindTable;
export 'src/provide.dart' show Library, LibraryIdentity, Offer, Provider;
