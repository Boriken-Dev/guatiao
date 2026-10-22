// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Dates, times of day, civil date-times, instants, spans and exact
/// lengths of time, read and written by the library's own functions.
///
/// Import it with a prefix: `import 'package:guatiao/time.dart' as time;`.
/// A date, a time of day and a civil date and time are [PlainDate],
/// [PlainTime] and [PlainDateTime]; an instant is a UTC `DateTime`; an
/// exact length is a `Duration`; a calendar length is a [Span].
/// Resolution is lazy, so importing touches no library.
library;

export 'src/time.dart';
