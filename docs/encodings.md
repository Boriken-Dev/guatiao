# Encodings

A value crosses between languages as one of eight kinds: null, boolean,
number, string, bytes, list, map, and the absent marker. Everything else
a program holds, a duration, a timestamp, an address, a set, is written
as one of those kinds in a form fixed here. **The form is the contract**:
any language that reads and writes these forms interoperates, and a
library type in one language is only one implementation of a form.

Each entry gives the value form, a JSON-shaped example, what a reader
accepts, and the [JSON Schema](https://json-schema.org/draft/2020-12)
(2020-12) that describes the form. The examples are JSON because JSON's
shapes are the value kinds; a number is kept as the exact text that
wrote it, so `18446744073709551615` and `0.1` cross unrounded.

A **schema describes what is written.** Where a reader also accepts
another spelling (a duration as a number of seconds), the schema does not
declare it, and a validator holding the schema refuses it.

`format` is an annotation: it names what the text holds and no
validator in this project refuses a value for failing it. `pattern`,
lengths, bounds, `uniqueItems`, `prefixItems`, `propertyNames` and `not`
are enforced (`pattern` only where the build carries a regular
expression engine).

## Containers and any value

| form | example | schema |
| --- | --- | --- |
| any value | `{"a": [1, null]}` | `{}` |
| a map of any keys and values | `{"host": "a", "n": 1}` | `{"type": "object", "additionalProperties": {}}` |
| a list of anything | `[1, "a", null]` | `{"type": "array", "items": {}}` |
| text | `"héllo"` | `{"type": "string"}` |
| a number, its text kept | `123456789012345678901234567890.5` | `{"type": "number"}` |
| bytes | (no JSON spelling) | `{"type": "bytes"}`, this project's own type |

## A character

Text of exactly one Unicode code point.

```json
"é"
```

Schema: `{"type": "string", "minLength": 1, "maxLength": 1}`.
Lengths count code points, as JSON Schema does: `"🦀"` is one.

## Sequences, sets, fixed arrays, tuples, nothing

| form | example | schema |
| --- | --- | --- |
| a sequence | `[3, 1, 2]` | `{"type": "array", "items": T}` |
| a set: no element repeated | `["a", "b"]` | `{"type": "array", "items": T, "uniqueItems": true}` |
| an array of exactly N | `[10, 0, 0, 1]` | `{"type": "array", "items": T, "minItems": N, "maxItems": N}` |
| a tuple: a kind per position | `[7, "x"]` | see below |
| nothing (a unit) | `null` | `{"type": "null"}` |

A set is written in the order its implementation iterates: a sorted set
sorted, a hash set in no reproducible order. A reader **refuses** a set
with an element equal to an earlier one rather than dropping it. Numbers
are equal by what they denote (`1` and `1.0`), everything else
structurally.

A tuple of N positions:

```json
{
  "type": "array",
  "prefixItems": [{"type": "integer"}, {"type": "string"}],
  "items": false,
  "minItems": 2,
  "maxItems": 2
}
```

A reader refuses a list of any other length, and reports an element of
the wrong kind by its position (`[1]`). A record whose fields are
positions (a tuple struct) is written the same way.

## Maps keyed by something other than text

An object whose keys are each key's text. The schema states the key's
shape in `propertyNames`, applied to the key as text:

```json
{"2": "two", "10": "ten"}
```

| key | text | `propertyNames` |
| --- | --- | --- |
| text | itself | none |
| unsigned integer | decimal digits, no sign, no leading zero (`0`, `10`) | a `pattern`, below |
| signed integer | the same with an optional `-` | a `pattern`, below |
| character | the character | `{"type": "string", "minLength": 1, "maxLength": 1}` |
| boolean | `true` or `false` | `{"type": "string", "enum": ["false", "true"]}` |
| an address | as below | that address's schema |
| a path | as below | none |

The integer keys' `propertyNames`, unsigned then signed:

```json
{"type": "string", "pattern": "^(0|[1-9][0-9]*)$"}
{"type": "string", "pattern": "^-?(0|[1-9][0-9]*)$"}
```

The value schema is `additionalProperties`. A reader refuses a key that
does not read back (`01`, `x` for an integer), and two keys that read as
one key (`::1` and `0:0::1`).

## Paths

The path's text, `"/etc/ssl/ca.pem"`, schema `{"type": "string"}`. A
path that is not valid Unicode has no text and is refused when written,
never written with replacement characters, which would name another
path.

## Network addresses

The address's usual text form.

| address | example | schema |
| --- | --- | --- |
| IPv4 | `"10.0.0.1"` | `{"type": "string", "format": "ipv4"}` |
| IPv6 | `"::1"` | `{"type": "string", "format": "ipv6"}` |
| either | `"10.0.0.1"`, `"::1"` | `{"type": "string", "anyOf": [{"format": "ipv4"}, {"format": "ipv6"}]}` |
| IPv4 socket | `"10.0.0.1:5900"` | `{"type": "string", "pattern": "^([0-9]{1,3}\\.){3}[0-9]{1,3}:[0-9]{1,5}$"}` |
| IPv6 socket | `"[fe80::1%2]:443"` | `{"type": "string", "pattern": "^\\[[0-9A-Fa-f:.]+(%[0-9]+)?\\]:[0-9]{1,5}$"}` |
| either socket | both of the above | the two joined, below |

Either socket address:

```json
{"type": "string",
 "pattern": "^(([0-9]{1,3}\\.){3}[0-9]{1,3}|\\[[0-9A-Fa-f:.]+(%[0-9]+)?\\]):[0-9]{1,5}$"}
```

An IPv6 socket address is the address in brackets, an optional numeric
zone after `%`, a colon and the port.

## Non-zero integers

An integer, never zero.

```json
{"type": "integer", "minimum": 1, "maximum": 4294967295}
```

for an unsigned width, and for a signed one the width's bounds and
`"not": {"const": 0}`:

```json
{"type": "integer", "minimum": -9223372036854775808,
 "maximum": 9223372036854775807, "not": {"const": 0}}
```

## A point in time

RFC 3339 `date-time` text in UTC, schema
`{"type": "string", "format": "date-time"}`:

```json
"2026-10-21T13:45:00.5Z"
```

**Written** as `YYYY-MM-DDTHH:MM:SS`, then `.` and the fraction of a
second with trailing zeros removed (none when it is zero), then `Z`. The
years 0000 to 9999 are writable; a time outside them is refused. The
calendar is the proleptic Gregorian one, with no leap seconds.

**Read** with this grammar, and converted to the instant it names:

```text
date-time  = year "-" month "-" day ("T" / "t") hour ":" minute ":" second
             [ "." 1*9DIGIT ] ( "Z" / "z" / offset )
offset     = ("+" / "-") hh ":" mm          ; hh 00-23, mm 00-59
year       = 4DIGIT                         ; 0000-9999
month      = 2DIGIT                         ; 01-12
day        = 2DIGIT                         ; 01 to the month's last day
hour       = 2DIGIT                         ; 00-23
minute     = 2DIGIT                         ; 00-59
second     = 2DIGIT                         ; 00-59: a leap second is refused
```

`2026-10-21T19:15:00.5+05:30` reads as the instant above. A space in
place of the `T`, a missing offset, more than nine fractional digits and
anything after the offset are refused, each naming the byte offset it
stopped at. An implementation keeps the fraction to its clock's
resolution.

## An exact length of time

A list of two integers, `[seconds, nanoseconds]`, the order of C's
`struct timespec`: whole seconds (0 to 18446744073709551615), then the
nanoseconds below a second (0 to 999999999).

```json
[90, 500000000]
```

```json
{
  "type": "array",
  "prefixItems": [
    {"type": "integer", "minimum": 0, "maximum": 18446744073709551615},
    {"type": "integer", "minimum": 0, "maximum": 999999999}
  ],
  "items": false,
  "minItems": 2,
  "maxItems": 2
}
```

A reader **also accepts**, and the schema does not declare:

- a **number of seconds**, read exactly from its text in the JSON number
  grammar: `1.5`, `2e-3`, `90`. Refused when negative (zero excepted),
  finer than a nanosecond (`0.0000000001`), or past the largest number of
  seconds above.
- the **map** `{"secs": 90, "nanos": 500000000}`, both keys present, as
  Rust's serde writes a duration.

Anything else is refused.

## A calendar length: span

A length in **calendar units**: years, months, weeks, days, hours,
minutes, seconds and nanoseconds, each counted on its own, with one sign
for the whole span. Two spans are equal unit by unit: `P1D` is not
`PT24H`, because where a calendar applies a span a day is not always 24
hours. The text is an ISO 8601 duration, schema
`{"type": "string", "format": "duration"}`.

```json
"P1Y2M3W4DT5H6M7.5S"
```

**Written** (canonical): `-` for a negative span, `P`, then each non-zero
unit in this order with its designator: years `Y`, months `M`, weeks
`W`, days `D`; then, if any of hours, minutes, seconds or nanoseconds is
non-zero, `T` and hours `H`, minutes `M`, seconds `S`. The seconds carry
the nanoseconds as a fraction with trailing zeros removed (`7.5S`,
`0.000000001S`). The zero span is `PT0S`. Each number is decimal digits
with no leading zero, at most 9223372036854775807.

**Read**, either of two forms.

ISO 8601:

```text
span       = [ "+" / "-" ] ("P" / "p") date-part [ time-part ]    ; at least one unit
date-part  = [ n "Y" ] [ n "M" ] [ n "W" ] [ n "D" ]              ; in this order
time-part  = ("T" / "t") [ n "H" ] [ n "M" ] [ n [ "." 1*9DIGIT ] "S" ]   ; at least one unit
n          = 1*DIGIT                                              ; at most 9223372036854775807
```

Designators are read in either case. Only the seconds take a fraction.
`-PT0S` is the zero span, which has no sign.

Friendly:

```text
span       = [ "-" ] *SP item *( *SP item ) *SP
item       = n unit / n "." 1*9DIGIT "s"
unit       = "y" / "mo" / "w" / "d" / "h" / "m" / "min" / "s"
           / "ms" / "us" / "µs" / "ns"
```

`mo` is months and `m` (or `min`) minutes. Each unit appears at most
once, in any order; spaces between items are optional; units are
lowercase. `ms`, `us` and `ns` add into the nanoseconds, a whole second
or more of them carried into the seconds: `1500ms` is `PT1.5S`. A
leading `-` makes the whole span negative: `-1d 12h` is `-P1DT12H`.

```text
1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns   =   P1Y2M3W4DT5H6M7.500001001S
1h30m                                 =   PT1H30M
```

A refusal names the byte offset it stopped at and what it expected
there, and never quotes the text.

A span converts to an exact length only when it has no years or months
and is not negative; a week is then 7 days and a day 24 hours. Applying a
span to a date is the date library's job, in the program that has one.

## Records and variants

A record with named fields is a map, one key per field; a field that is
absent is omitted, and its schema leaves it out of `required`. A record
of one field written transparently is that field's value. A record with
no fields, written as a unit, is `null`.

A variant (a tagged enum) is a map holding its name under the tag key,
then its fields: `{"type": "Square", "side": 2}`, described with
`oneOf` arms and the tag under `x-variant-tag`. A variant holding one
record writes that record's fields beside the tag:
`{"type": "Circle", "a": 1}`.

## In Rust

The `guatiao` crate implements every form above for these types, under
`ToValue`, `FromValue` and `Schema` and the derives.

| form | Rust types |
| --- | --- |
| any value, map, list, text, number, bytes | `Value`, `Map`, `List`, `Text`, `Number`, `Buffer` |
| a character | `char` |
| a sequence | `Vec<T>`, `VecDeque<T>`, `[T]`, `Box<[T]>`, `Arc<[T]>`, `Rc<[T]>` |
| a set | `BTreeSet<T>`, `HashSet<T>` |
| a fixed array, a tuple, nothing | `[T; N]`, tuples of 1 to 12, `()` |
| maps keyed by data | `BTreeMap<K, V>`, `HashMap<K, V>` with `K: MapKey` |
| a path | `PathBuf`, `Path`, `Box<Path>`, `Arc<Path>`, `Rc<Path>` |
| addresses | `IpAddr`, `Ipv4Addr`, `Ipv6Addr`, `SocketAddr`, `SocketAddrV4`, `SocketAddrV6` |
| non-zero integers | every `NonZero*` |
| a point in time | `std::time::SystemTime` |
| an exact length | `std::time::Duration` |
| a span | `guatiao::Span` |
| the same as `T` | `Box<T>`, `Arc<T>`, `Rc<T>`, `Cow<'_, T>`, `&T` |
