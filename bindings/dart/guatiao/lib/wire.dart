// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Channels: a schema announced before its values, so a receiver knows
/// what each value means and refuses what does not fit it.
///
/// Import it with a prefix: `import 'package:guatiao/wire.dart' as wire;`.
/// A frame is `kind:u8 channel:LEB128 payload` (kind 0 a schema, 1 a
/// value, 2 the channel's end); the transport says where one ends.
/// [announce], [send] and [close] write frames; a [Receiver] reads them
/// and answers an [Event], or throws a [Refused] that is also the status's
/// exception ([Malformed] and [Invalid] are `BadValue`s, [NoSchema] a
/// `NotFound`, [NotASchema] a `WrongKind`). One value in the wire encoding
/// is `Value.fromWire(bytes)` and `value.toWire()`. Resolution is lazy, so
/// importing touches no library.
library;

export 'src/wire.dart';
