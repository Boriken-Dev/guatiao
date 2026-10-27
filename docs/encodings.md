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

## Dates and times

Four forms, each a string and none of them with a time zone: a calendar
date, a time of day, the two together (a **civil** date and time, what a
wall clock shows), and an **instant**. The calendar is the proleptic
Gregorian one with no leap seconds, and every form covers the years 0000
to 9999, which is what four digits write; a value outside them is
refused, written or read.

| form | example | schema |
| --- | --- | --- |
| a date | `"2026-10-21"` | `{"type": "string", "format": "date"}` |
| a time of day | `"13:45:00.5"` | a `pattern`, below |
| a civil date and time | `"2026-10-21T13:45:00.5"` | a `pattern`, below |
| an instant | `"2026-10-21T13:45:00.5Z"` | `{"type": "string", "format": "date-time"}` |

The time of day and the civil date and time state a `pattern` rather
than a `format`, because JSON Schema's `time` and `date-time` formats
require an offset and these forms have none. The time of day, then the
civil date and time:

```json
{"type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{1,9})?$"}
{"type": "string",
 "pattern": "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{1,9})?$"}
```

A pattern cannot know how long a month is; the reader refuses
`2026-02-30` as well.

**Written**: a date as `YYYY-MM-DD`; a time as `HH:MM:SS`, then `.` and
the fraction of a second with trailing zeros removed, none when it is
zero; a civil date and time as the date, `T` and the time; an instant as
its UTC date and time followed by `Z`.

**Read** with this grammar, RFC 3339's (section 5.6) with the offset
left out of the forms that have none:

```text
date       = year "-" month "-" day
time       = hour ":" minute ":" second [ "." 1*9DIGIT ]
date-time  = date ("T" / "t") time                    ; a civil date and time
instant    = date-time ( "Z" / "z" / offset )
offset     = ("+" / "-") hh ":" mm                    ; hh 00-23, mm 00-59
year       = 4DIGIT                                   ; 0000-9999
month      = 2DIGIT                                   ; 01-12
day        = 2DIGIT                                   ; 01 to the month's last day
hour       = 2DIGIT                                   ; 00-23
minute     = 2DIGIT                                   ; 00-59
second     = 2DIGIT                                   ; 00-59: a leap second is refused
```

An instant is read with any offset and converted to the instant it
names: `2026-10-21T19:15:00.5+05:30` is `2026-10-21T13:45:00.5Z`, and
`-00:00` (RFC 3339's "offset unknown") reads as UTC. An offset that moves
the instant outside the years 0000 to 9999
(`0000-01-01T00:30:00+01:00`) is refused at the offset. A date, a time of
day and a civil date and time take no offset: a `Z` or a `+05:30` after
one is refused. A space in place of the `T`, more than nine fractional
digits and anything after the form are refused too, each naming the byte
offset it stopped at, and no refusal quotes the text.

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
and is not negative; a week is then 7 days and a day 24 hours.

## Applying a span to a date

A span moves a date, or a civil date and time, in three steps:

1. **Years and months**, together, as a number of months. A day past the
   end of the month it lands in becomes that month's last day:
   `2026-01-31` plus `P1M` is `2026-02-28`, and `2024-02-29` plus `P1Y` is
   `2025-02-28`.
2. **Weeks and days**, a week being 7 days.
3. **Hours, minutes, seconds and nanoseconds**, on a civil date and time
   only, carried across midnight: `2026-10-21T23:30:00` plus `PT1H` is
   `2026-10-22T00:30:00`. On a date, a span with any of them is refused:
   a date has no time of day to carry them into.

The steps run in that order whatever order the text wrote the units in,
and the month-end clamp happens once, after the first step: `2026-01-31`
plus `P1M1D` is `2026-03-01`. A negative span runs each step backwards:
`2026-03-31` plus `-P1M1D` is `2026-02-27`. A step that lands outside
the years 0000 to 9999 is refused.

No time zone takes part, so a day on a civil date and time is always 24
hours. An instant takes no span; a program that wants "one month after
this instant" applies the span to the instant's civil date and time in
the zone it means.

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

## Variants told apart by kind

A variant can also store no name at all, when every alternative is a
different **kind** of value: a string or a map, a number or a list. The
value is the alternative's own value, and a reader picks the
alternative by the value's kind, the same way in every language.

```json
"identity"
{"certificate": "/etc/ssl/c.pem", "key": "/etc/ssl/k.pem"}
```

**No two alternatives are of one kind** (null, boolean, number, string,
bytes, list, map). Alternatives with no value of their own are one
string alternative together, its text one of their names. The schema
is an `anyOf` of the alternatives, so at most one accepts a value:

```json
{"anyOf": [
  {"type": "string", "enum": ["identity", "off"]},
  {"type": "object",
   "properties": {"certificate": {"type": "string"}, "key": {"type": "string"}},
   "required": ["certificate", "key"], "additionalProperties": false}
]}
```

A reader takes the value's kind, finds the one alternative of that
kind and reads the value as it. A value of a kind no alternative has is
refused, naming the kinds that would be accepted, and text naming no
alternative is refused naming the names; neither quotes the value.

**In a form**, the field is one entry. An alternative that is a
string, a number or a boolean is typed as its text; one that is a list
or a map has no one-line spelling, so a flat store cannot hold it
(`flatten` answers false) and a form takes it as a whole value.

## A secret

A secret is **not a form of its own**. It crosses as the value it holds,
a string, a number or a map, and its schema marks it: the holder's own
schema with two keywords added.

```json
{"type": "string", "writeOnly": true, "x-sensitive": true}
```

`writeOnly` is JSON Schema's "accepted, never handed back", which any
JSON Schema tool understands; `x-sensitive` is this project's "never
print it". A reader takes the value as it would without them.

What honours the marks:

- **Redaction.** `guatiao_schema_redact` in C, and `redact` in Rust,
  Python, Dart, Node, Julia and R, `Redact` in Go and `guatiao.redact` in
  MATLAB and Octave, copy a value with every marked field replaced by a
  placeholder, at any depth: inside a nested record, in every element of
  a list, in a variant's arm. Where the schema stops describing the
  value (a key it does not declare, say), the value is hidden too,
  since nothing says it is not a secret.
- **A form.** The form a schema implies shows a marked text field with
  the `password` widget, masked while it is typed.

What a mark does not do: encrypt the value, compare it in constant time,
or wipe it from memory. It says who may see it, and the code that shows,
logs or returns values honours it.

## Values on the wire

The forms above are trees of the eight kinds. To carry a tree as bytes,
over a socket, in a file or through a message queue, it is written in
the **wire encoding**: exact, the same bytes on every platform and
pointer width, and needing nothing but the bytes to read.

```text
value := tag:u8 body
body  := (nothing)                ABSENT 0, NULL 1
       | 0 | 1                    BOOL 2
       | len text                 NUMBER 3 (the JSON number grammar), STRING 4 (UTF-8)
       | len bytes                BYTES 5
       | count value*             LIST 6
       | count (len key value)*   MAP 7 (keys UTF-8, none repeated, in order)
len, count := unsigned LEB128, the shortest form, at most 64 bits
```

A number crosses as its own text, so `1.10` stays `1.10`; bytes cross
as bytes and are never confused with text. A reader refuses, naming the
byte it went wrong at, an unknown tag, a body cut short, a length not in
its shortest form or past 64 bits, text that is not UTF-8, a number
outside the grammar, a bool byte other than 0 or 1, a key repeated in
its map, and anything after the value. Nothing is repaired.

**Channels** give values a meaning: a schema is announced on a channel
before the values sent on it, and a receiver checks every value against
its channel's schema before handing it over.

```text
frame := kind:u8 channel:LEB128 payload
kind  := 0 schema   (payload: a schema, wire-encoded; it replaces any earlier one)
       | 1 value    (payload: a value, wire-encoded)
       | 2 close    (no payload: the channel's schema is forgotten)
```

A frame does not say how long it is: the transport does (a length prefix
on a stream, one datagram, one WebSocket message). A receiver refuses a
frame without changing what it keeps, for one of four reasons, each with
its status:

| refusal | status | what it carries |
| --- | --- | --- |
| malformed: the bytes are not a frame | `GUATIAO_ERR_BAD_VALUE` | the offset of the byte in the frame |
| no schema: a value on a channel nothing was announced on | `GUATIAO_ERR_NOT_FOUND` | the channel |
| not a schema: a schema frame whose value is not a map | `GUATIAO_ERR_WRONG_KIND` | the channel |
| invalid: a value its channel's schema refuses | `GUATIAO_ERR_BAD_VALUE` | the channel, the field and what it accepts |

A refusal never quotes what it was given. The vectors every
implementation runs, values with their bytes, malformed inputs, frames
and a conversation with every refusal, are
[`crates/guatiao/tests/vectors/wire.json`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/tests/vectors/wire.json).

| | one value | frames | receiving |
| --- | --- | --- | --- |
| C | `guatiao_wire_encode`, `guatiao_wire_decode` | `guatiao_wire_announce`, `_send`, `_close` | `guatiao_wire_receiver_new`, `_accept` (a `guatiao_wire_event`), `_free` |
| Rust | `wire::to_bytes`, `wire::decode` | `wire::channel::{announce, send, close}` | `Receiver::accept`, `Received`, `WireError` |
| Python | `ref.to_wire()`, `Value.from_wire(data)` | `guatiao.wire.{announce, send, close}` | `wire.Receiver().accept(frame)`; `Malformed`, `NoSchema`, `NotASchema`, `Invalid` |
| Dart | `ref.toWire()`, `Value.fromWire(data)` | `package:guatiao/wire.dart`: `announce`, `send`, `close` | `Receiver().accept(frame)`; the same four, each a `Refused` |
| Node | `ref.toWire()`, `Value.fromWire(data)` | `guatiao/wire`: `announce`, `send`, `close` | `new Receiver().accept(frame)`; the same four |
| Go | `ref.ToWire()`, `FromWire(data)` | `WireAnnounce`, `WireSend`, `WireClose` | `NewReceiver()`, `Accept(frame)`; a `*Refused` |
| Julia | `to_wire(v)`, `from_wire(data)` | `Wire.announce`, `Wire.send`, `Wire.close` | `Wire.Receiver()`, `Wire.accept(r, frame)`; a `Wire.Refused` |
| R | `to_wire(x)`, `from_wire(bytes)` | `wire_announce`, `wire_send`, `wire_close` | `wire_receiver()$accept(frame)`; conditions `guatiao_wire_*` |
| MATLAB, Octave | `v.to_wire()`, `guatiao.Value.from_wire(b)` | `guatiao.wire.announce`, `.send`, `.close` | `guatiao.wire.Receiver().accept(frame)`; `guatiao:<Status>:<refusal>` errors |
| Fortran | `guatiao_to_wire`, `guatiao_from_wire` | `guatiao_wire_announce`, `_send`, `_close` | `guatiao_open_receiver`, `r%accept`; a `guatiao_wire_refusal` |
| C++ | `guatiao::to_wire(v)`, `guatiao::from_wire(bytes)` | `guatiao::wire_announce`, `wire_send`, `wire_close` | `guatiao::receiver().accept(frame)`; an `error` whose detail names the refusal |

In every binding the refusal is also the status's own error, so code
that catches a bad value catches a malformed frame and an invalid value
alike, and code that wants the reason reads the refusal.

## A kind, described

A kind's C header says how to fill its table. Its **description** says
the same as data, so a binding, a code generator or a peer can load the
kind rather than declare it again: the kind's name, its floor hash, and
each slot in table order, with its arguments, its answer, and whether it
is required and whether it can refuse. It is a value, written as JSON in
a file beside the header (`include/<crate>.kinds.json`):

```json
{
  "kinds": [
    {
      "name": "greeter",
      "floor_hash": 2695856491,
      "object": false,
      "doc": "Greets by name.",
      "slots": [
        {
          "name": "greet",
          "required": true,
          "fallible": true,
          "args": [{ "name": "name", "type": "str" }],
          "answer": { "type": "map" }
        },
        {
          "name": "greet_into",
          "required": false,
          "fallible": true,
          "args": [
            { "name": "name", "type": "str" },
            { "name": "listener", "type": "object", "kind": "listener", "pass": "lent" }
          ]
        }
      ]
    }
  ]
}
```

- **The document** is a map whose `kinds` is the list of kinds; a reader
  tells it from one kind by that key, and skips any other key.
- **A kind**: `name` (what a descriptor and a registry call it),
  `floor_hash` (the `<table>_FLOOR_HASH` its header defines), `object`
  (an object kind: its table has `destroy` right after the header), an
  optional `doc`, and `slots`.
- **A slot**: `name` (its field in the table), `required` (a slot every
  provider fills, not null: the required slots come first, and the floor
  hash covers them; any other is optional, which a provider may leave
  null), `fallible` (it ends in `guatiao_provider_error
  *err`), `args` (after `ctx`, in order), `answer` (written through a
  pointer before `err`; absent when there is none), and an optional `doc`.
- **An argument** is `name`, `type`, and for a tree or an object
  `pass`. An **answer** is `type`, and `kind` for an object.

| `type` | as an argument (C) | as an answer (C) |
| --- | --- | --- |
| `bool`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `isize`, `usize`, `f32`, `f64` | the scalar (`isize` is `intptr_t`, `usize` `size_t`) | a pointer to it |
| `str` | `guatiao_str`, UTF-8 text the caller keeps | -- |
| `bytes` | `guatiao_bytes`, the caller's | -- |
| `bytes_mut` | `guatiao_bytes_mut`, the caller's memory, written in place (object kinds only) | -- |
| `text` | -- | `guatiao_string *`: owned text |
| `value`, `map`, `list` | `pass` says how: `borrowed` (`const guatiao_value *` / `const guatiao_map *`; a borrowed `value` may be null when `nullable` is true), `taken` (a non-`const` pointer to a tree the callee then owns) or `lent` (a non-`const` pointer to the caller's tree, for the call) | `guatiao_value *` / `guatiao_map *` / `guatiao_list *`, holding a carrier: the answer is built in the allocator it names |
| `object` | `kind` names an object kind; `pass` is `taken` (`guatiao_object` by value: the callee owns it) or `lent` (`const guatiao_object *`, for the call) | `guatiao_object *`: the caller owns it |

A `kind` an argument or answer names is found in the same document, or
among kinds the reader already holds. A description is written by
`#[guatiao::kind]` (`<Trait>Vtable::description()`, one kind as compact
JSON); a kind crate writes its document with the same switch that writes
its header, and a test compares the committed copy with a fresh one. A
kind that has no trait is written by the language that declares it: Node
reads TypeScript interfaces (`guatiao-kind`, `describeInterfaces`) and
JavaScript classes (`describeKinds`), Python `KindProtocol` classes
(`kinds.describe_kinds`), Dart abstract interface classes (`dart run
guatiao:kind describe`) and Go interfaces (`guatiao-kind -document`),
into the same document, and a floor
hash it is not given is the FNV-1a of the shape and the required slots
spelled as Rust spells them (`provider;greet(&str)->Result<Map,ProviderError>`).
`crates/guatiao-derive/tests/vectors/floor_hash.json` holds kinds as
their description says them, beside the Rust trait declaring the same
slots, that input and the hash: the derive and every language computing
the hash check themselves against it.

A provider carries the same document in its descriptor
(`guatiao_provider_info.kind_descriptions`, JSON text): the kinds it
serves and every object kind their slots take or answer, each once, so a
host binds them with no file. `#[derive(Provider)]` writes it, as do
guatiao-bridge for the providers a language bridge declares and
guatiao-source-remote for a peer's providers (from the descriptions its
catalog lists); a provider written by hand may leave it empty. A host reads it parsed with
`guatiao_registry_kind_descriptions` (ABSENT when the provider carries
none), `Provider::kind_descriptions` in Rust, `kindDescriptions` in Node
and Dart and `kind_descriptions` in Python.

## A library manifest

A library one of guatiao's sources serves (a Python module, a JavaScript
package, a Lua module, a Rhai script, a WebAssembly module) is described
by a manifest the source reads without running anything: a TOML file
named `<name>.guatiao.toml` beside or above the library's code.

```toml
type = "python"              # the source that serves it
module = "greeters"          # the library's id
version = "1.0.0"            # optional: else read from the code
kinds = ["greeter"]          # every kind its providers serve
paths = ["."]                # where its code is, relative to this file
```

| key | holds |
| --- | --- |
| `type` | required text: the source that serves it, as the source declares itself (`source=<type>`): `python`, `js`, `lua`, `wasm` or `rhai`; `remote` describes a peer (below) |
| `module`, or `name` for `js` | required text, not empty: the library's id |
| `version` | optional text: without it the source reads the version from the code, as its page says |
| `kinds` | a list of text: every kind its providers serve |
| `paths` | a list of text, relative to the manifest's directory, `["."]` when absent: where the code is; editing a code file under them is what a refresh notices |
| anything else | the source's own (`main` for `js`) |

A source finds manifests in each directory its configuration's `dirs`
names and in their immediate subdirectories, and in the files its
`manifests` names. It reads those of its own `type` and passes over those
of another type above without a word, so one directory may hold
libraries for several sources. A manifest with no `type`, or a type none
of them serves, is listed as a library that does not open, in words
naming the file: a host that adds several sources over its directory
hears of it from each.

### A peer

A manifest of `type = "remote"` describes a peer that guatiao-source-remote
reaches, rather than a library: a device or a process serving providers
over [guatiao's remote protocol](remote.md).

```toml
type = "remote"
name = "lab-printer"             # the peer's name
connect = "tcp://192.0.2.5:7000" # where it listens, and by which transport
security = "none"                # no TLS: the key proved both ways
key_file = "lab-printer.key"     # the pre-shared key, relative to this file
```

| key | holds |
| --- | --- |
| `name` | required text, not empty and holding no `/`: the peer's name. Its libraries and providers are served as `<name>/<id>` |
| `connect` | required text: `tcp://host:port`, `ws://host:port/path`, `tls://host:port`, `local://name`, `shm://name` or `rdma://host:port` |
| `security` | text: `none`, `server-tls` or `mutual-tls` ([which a scheme runs](remote.md#security)); when absent, the source's configuration's, else `none` for `local://` and `shm://` and `mutual-tls` for a network transport; `trust`, its former name, is refused with what to write now |
| `key_file` | text, relative to the manifest's directory: a file holding the pre-shared key (`none`, and server TLS when the server asks for it) |
| `token_file` | text, relative to the manifest's directory: a file whose text is the token the client presents when its server requires one (server TLS and mutual TLS). A byte order mark and one last line end (`\n` or `\r\n`) are not the token's; a token empty or only white space is refused |
| `pins` | list of text: the fingerprints (`sha256:<hex>`) of the peer's certificate TLS trusts |
| `roots` | text: `platform` (the operating system's certificate authorities), or a CA bundle relative to the manifest's directory: also trust the peer's certificate when one of them issued it for the host in `connect` |
| `policy` | table: the TLS policy, coqui-peer-tls's text form version 1 ([the keys](remote.md#mutual-tls)) |
| `identity_file`, `state_dir` | text, relative to the manifest's directory: the identity this host presents to the peer over TLS, a PEM file or a directory holding `identity.pem` |
| `timeout_ms`, `max_frame`, `max_calls` | whole numbers: a call's wait (`0` for as long as the link lives) and the link's bounds |
| `rdma_device` | text: the RDMA device `rdma://` goes through |

Nothing else: a manifest naming `key` or `token` is refused, since a
secret in a search path is a credential anyone reading the directory
holds, as is one naming `child` (a manifest describes a peer that
listens; a child the host starts is the host's own configuration) and
one naming a key not listed here. Keys the manifest leaves out are taken
from the source's configuration; a manifest's `key_file` or `token_file`
replaces the source's `key` or `token`. Its stamp is its content and the
path and content of the key file, the token file, the identity file and
the CA bundle the peer uses: a change to any is a new peer.

## In C, C++, Python, Dart, Node, Go, Julia, R, MATLAB, Octave and Fortran

Each text form above has C functions that read it into a plain struct,
write it back from one, and rewrite text in its canonical spelling, all
declared in `guatiao.h`; the C++, Python, Dart, Node, Go, Julia, R,
MATLAB/Octave and Fortran bindings call the same functions, so every grammar has one implementation. A refusal is
`GUATIAO_ERR_BAD_VALUE` with a `guatiao_text_error` that names the byte
offset and what was expected there. `guatiao_date_add_span` and
`guatiao_datetime_add_span` apply a span, and `guatiao_duration_read`
reads an exact length in any of its three spellings.

| form | C | C++ | Python | Dart | Node | Go | Julia | R | MATLAB, Octave | Fortran |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| a date | `guatiao_date` | `guatiao::date` | `datetime.date` | `PlainDate` | `PlainDate` | `civil.Date` | `Dates.Date` | `Date` | `guatiao.Date` | `guatiao_date` |
| a time of day | `guatiao_time` | `guatiao::time_of_day` | `datetime.time`, no `tzinfo` | `PlainTime` | `PlainTime` | `civil.Time` | `Dates.Time` | `civil_time` | `guatiao.TimeOfDay` | `guatiao_time` |
| a civil date and time | `guatiao_datetime` | `guatiao::date_time` | `datetime.datetime`, no `tzinfo` | `PlainDateTime` | `PlainDateTime` | `civil.DateTime` | `Dates.DateTime` | `civil_datetime` | `guatiao.DateTime`; MATLAB's `datetime` with no zone, written | `guatiao_datetime` |
| an instant | `guatiao_timestamp` | `guatiao::timestamp`; a `std::chrono` time point | `datetime.datetime` in UTC | `DateTime`, `isUtc` | `Date` | `time.Time` in UTC | `Guatiao.Timestamp` | `instant`, `POSIXct` | `guatiao.Timestamp`; MATLAB's `datetime` with a zone, written | `guatiao_timestamp` |
| an exact length | a list of two numbers | `guatiao::length`; a `std::chrono` duration | `datetime.timedelta` | `Duration` | `{ seconds: bigint, nanoseconds }` | `civil.Length`, `time.Duration` | `Guatiao.Length`, a `Dates.FixedPeriod` | `exact_length`, `difftime` | `guatiao.Duration`; MATLAB's `duration`, written | `guatiao_length` |
| a span | `guatiao_span` | `guatiao::span` | `guatiao.time.Span` | `Span` | `Span` | `civil.Span` | `Guatiao.Span`, `Dates.CompoundPeriod` | `span` | `guatiao.Span` | `guatiao_span` |

What a mapping loses, and what happens then:

- **Below a microsecond.** Python's `datetime`, `time` and `timedelta`
  and Dart's `DateTime` and `Duration` count microseconds. Reading a form
  into one of them drops the nanoseconds below a microsecond, toward the
  past; writing one back is exact. JavaScript's `Date` counts
  milliseconds, so an instant read into one drops what is below a
  millisecond the same way, as does a civil date and time read into
  Julia's `Dates.DateTime`; R's `POSIXct` is a double, written to the
  microsecond. `PlainTime`, `PlainDateTime`, every `Span`
  class, Node's exact length, every Go type, Julia's `Dates.Time`,
  `Timestamp`, `Span` and `Length`, R's `civil_time`, `civil_datetime`,
  `instant` and `span`, every `guatiao` time class in MATLAB and Octave,
  every C++ and Fortran type and every C struct keep the nanoseconds; MATLAB's own `datetime` and
  `duration` are written to the nanosecond they hold.
- **The year 0000.** Python's `datetime.date` and `datetime.datetime`
  start at year 1, so reading a date, a civil date and time or an
  instant in year 0000 into one raises `ValueError`. Dart, Node, Go,
  Julia, R, MATLAB, Octave, Fortran, C++ and C hold it.
- **Long exact lengths.** `timedelta` stops at 999999999 days and Dart's
  `Duration` at 2^63 - 1 microseconds (about 292471 years), and Go's
  `time.Duration` and Julia's `Dates.Nanosecond` at 2^63 - 1 nanoseconds
  (about 292 years); a longer length raises, or answers an error, rather
  than wraps, and Go's `civil.Length` and Julia's `Guatiao.Length` hold
  every one. An R number holds whole numbers exactly to 2^53, so R's
  `exact_length` and `span` keep a length or a unit past that as its exact
  text; `guatiao.Duration` and `guatiao.Span` in MATLAB and Octave, whose
  numbers are doubles, refuse one. Fortran's `guatiao_length` holds every one, its
  seconds an unsigned count in an int64's bits, as C++'s `guatiao::length` holds them; a
  `std::chrono` time point or duration answers BAD_VALUE for one its clock cannot hold. An exact
  length is never negative, so a negative
  `timedelta`, `Duration` or Julia period is refused when written.
- **An offset.** An instant read into any type is in UTC: the offset it
  was written with is not kept, as the canonical text does not keep it.

Dart has no type for a date or a time of day alone, and its `DateTime` is
always either UTC or the machine's own zone, which would move a civil
time that a daylight-saving change skips; hence the three `Plain`
classes, named after JavaScript's Temporal. Node's own `Date` has the same
two limits, and Temporal is not in Node 22 or 24, so the Node binding has
the same three classes. A span's units are `bigint` there, as a signed
64-bit unit can be larger than a `number` holds exactly. Go's `time`
package has no date or time of day alone either, so the Go binding's
`civil` package holds them, beside `time.Time` for an instant; each is its
text in a value. Julia's `Dates` has a date, a time of day and a civil date
and time but no instant apart from them, so the Julia binding writes a
`Dates.DateTime` as a civil date and time and has `Timestamp` for an
instant. Base R has a date and an instant (`POSIXct`, a double of
seconds) and neither a time of day nor a civil date and time, so the R
binding has `civil_time` and `civil_datetime`, and `instant` for an
instant to the nanosecond. Octave has no date or time type at all and
MATLAB's `datetime` is a double of milliseconds, so the MATLAB and Octave
binding has a class per form, each checked by the C functions; a MATLAB
`datetime` or `duration` is written in the form it holds. Fortran has no
date or time type, so the Fortran binding's `guatiao_civil` module has a
derived type per form, read and written by the C functions.

A secret is the value it holds in every language; what each offers is
redaction, `guatiao_schema_redact`, `guatiao.redact` in Python, MATLAB
and Octave, `redact` in Dart, Node, Julia and R, `guatiao.Redact` in
Go, `guatiao::redact` in C++ and `guatiao_redact` in Fortran, with a
placeholder of the caller's choosing.

## In Rust

The `guatiao` crate implements every form above for these types, under
`ToValue`, `FromValue` and `Schema` and the derives. A type that speaks
serde (`toml::Table`, `serde_json::Value`, a type of your own) crosses
through `guatiao-serde`'s `Serde<T>` wrapper or its `adapt` modules, in
serde's data model.

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
| a date, a time of day, a civil date and time | `guatiao::Date`, `guatiao::Time`, `guatiao::DateTime` |
| an instant | `guatiao::Timestamp`, `std::time::SystemTime` |
| a secret | `guatiao::Secret<T>`, holding any `T` above |
| an exact length | `std::time::Duration` |
| a span | `guatiao::Span` |
| the same as `T` | `Box<T>`, `Arc<T>`, `Rc<T>`, `Cow<'_, T>`, `&T` |
| a variant told apart by kind | an enum deriving the three with `#[map(untagged)]` |
