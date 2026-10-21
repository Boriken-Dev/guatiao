# guatiao (Node.js) — API header

Node.js and TypeScript bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the npm package, so it names no
repository path.

ESM only, typed (`.d.ts` beside every module), Node 22 or later, 64-bit
platforms. The one runtime dependency is `koffi`, a foreign-function
interface whose native part comes prebuilt per platform; this package
ships no native code of its own.

## Entry points

| import | what |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `MapRef`, `ListRef`, `Tag`, `Status`, `redact`, `useLibrary`, the errors |
| `guatiao/serde` | `loads`, `dumps`: JSON, TOML, YAML |
| `guatiao/intake` | forms, paths, the flat projection |
| `guatiao/time` | `PlainDate`, `PlainTime`, `PlainDateTime`, `Span`, instants, exact lengths |

## Finding the native library

Importing touches nothing native: koffi and the library load on the first
call that needs them, in this order:

1. `useLibrary(pathOrUrl)`, when called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded throws; naming the same one does
   nothing.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   `guatiao`; `guatiao_serde` and `guatiao_intake` are looked for beside
   it under their own names).
3. The platform's own library search, by file name.

Not found throws `LibraryNotFound` naming each route. `guatiao_serde` and
`guatiao_intake` resolve the same way, independently, when `guatiao/serde`
or `guatiao/intake` is first used. A symbol the loaded build does not
export throws `MissingSymbol` (`.symbol`, `.feature`) when called.

## Values

`Value` owns a root node; `close()` frees it (`using v = new Value(...)`
does it at the end of the scope; a value dropped unclosed is freed when it
is collected). `Ref` addresses a node inside a tree by a path of keys and
indices, walked fresh on every access, never a cached address: a write
next to a node can move the array it sits in. `MapRef` and `ListRef` are
`Ref`s over a map and a list. `Value` is itself a `Ref` onto its root.

```ts
import { Value } from 'guatiao';

using v = new Value({ host: '10.0.0.1', port: 5900, tags: ['a', 'b'] });
const m = v.asMap();
m.get('port')?.toJS();          // 5900
m.set('port', 5901);
m.keys();                       // ['host', 'port', 'tags']
m.get('tags')!.asList().push('c');
v.toJS();                       // back to a plain object
```

**Construction**: `new Value(x)` converts plain data: `undefined` ->
absent (and `new Value()` is absent), `null` -> null, `boolean`, `number`
and `bigint` -> number (its exact text: `String(n)`, `-0` kept; a
non-finite number is a `RangeError`), `string` -> string (a lone
surrogate is a `TypeError`), `Uint8Array` / `ArrayBuffer` / any typed
array -> bytes, an array -> list, a plain object or a `Map` with string
keys -> map, a `Value` or `Ref` -> a deep copy. Anything else (a `Date`, a
class instance, a cycle) is a `TypeError`. Also `Value.from(x)`,
`Value.absent()`, `.null()`, `.bool(b)`, `.string(s)`, `.bytes(data)`,
`.number(text)` (the exact text, no `number` on the way), `.map()`,
`.list()`. Every builder takes `{ alloc }`, the address (`bigint`) of a
`guatiao_alloc` the caller keeps alive while the tree lives.

**Reading**: `ref.toJS({ numbers, maps })` walks the whole node.
`numbers: 'auto'` (default) gives a `number` for a fraction or an
exponent and for a safe integer, a `bigint` for a larger integer;
`'bigint'` a `bigint` for every integer; `'text'` the exact text. `maps:
'object'` (default) gives plain objects (a key named `__proto__` is data)
and `'map'` gives `Map`s. Absent reads as `undefined`, bytes as a
`Uint8Array` copy. `toJS<T>()` is an unchecked cast. `ref.boolOr`,
`intOr(fallback: bigint)` (a signed 64-bit integer; a fraction, an
exponent, another kind or out of range falls back, never truncates),
`numberOr`, `stringOr` read one field. Also `ref.tag` (a `Tag`),
`isAbsent()`, `isNull()`, `asMap()` / `asList()` (`TypeError` for another
kind), `clone()` (a deep, independent `Value`).

**`MapRef`**: `size`, `has(k)`, `get(k)` (a `Ref`, or `undefined`),
`set(k, v)` (returns the map), `delete(k)` (whether there was one),
`keys()`, `values()`, `entries()` (arrays, as the map is now), iteration
over `[key, Ref]`, `clear()`, `copyFrom(other)`. Keys compare as UTF-8
bytes and keep insertion order.

**`ListRef`**: `length`, `at(i)` (negative from the end; `undefined`
outside), `push(...values)` (returns the length), `removeAt(i)`
(`RangeError` outside), `clear()`, iteration over `Ref`s, and `set(i, v)`
and `insert(i, v)`, which are O(n): the C surface only appends and
removes, so they clone every element, clear and push back.

A closed value, or a borrowed view whose source is closed or whose call
has returned, throws `TypeError` on any use.

## serde: JSON, TOML, YAML

```ts
import * as serde from 'guatiao/serde';

const text = serde.dumps(value, { format: 'json', pretty: true });  // or 'toml', 'yaml'
using parsed = serde.loads(text, { format: 'json' });
```

`dumps` takes a `Ref` or plain data. A document that does not parse
throws `ParseError` (a `BadValue`) with `.format`, `.line` and `.column`
(from 1, the column in characters; 0 when the parser did not say),
`.reason`, which never quotes the document. A TOML document must be a map
(`WrongKind`). `toml`/`yaml` throw `MissingSymbol` naming the feature when
the loaded `guatiao_serde` lacks it. `how` takes `BYTES_DATA_URI`,
`BYTES_BASE64`, `BYTES_ARRAY`, `BYTES_REFUSE`, `READ_DATA_URIS`,
`PRETTY`. A number keeps its exact text, except that an exponent is read
back with its sign (`1e400` is `1e+400`).

## intake

```ts
import * as intake from 'guatiao/intake';

intake.check(schema, form);              // null, or the error map
intake.layout(schema, form);             // [{ section, fields: [...] }, ...]
intake.isVisible(schema, form, 'key', values);
intake.at(value, 'agent[1].name');       // a borrowed Ref, or undefined
intake.forSchema(schema);                // the form a schema implies (a Value to close)
intake.formFor(form, schema, 'path');    // a Value to close, or undefined
intake.resolve(schema, 'auth.password'); // borrowed Ref onto the field, or undefined
intake.flatKeys(schema, 'auth');         // string[]; WrongKind for an unknown key
intake.flatten(schema, 'auth', value);   // Record<string, string>
intake.unflatten(schema, 'auth', store); // a Value to close, or undefined
```

Every `schema`, `form`, `value` and `values` is a `Ref` or plain data.
What `at` and `resolve` answer borrows from what it came from: read-only
(every write is a `TypeError`), found again on every access, and a
`TypeError` once the source is closed; `.clone()` copies it out.
`unflatten` answers `undefined` only for "nothing stored"; a store that is
not strings to strings (a plain object, a `Map` or a `Ref` onto a map) is a
`TypeError`.

## time

The forms are text in a value (an exact length is `[seconds,
nanoseconds]`), fixed at <https://boriken-dev.github.io/guatiao/encodings/>,
and every text is read and written by the library's own functions.

```ts
import { PlainDate, PlainDateTime, PlainTime, Span, normalize, parseTimestamp } from 'guatiao/time';

new PlainDate(2026, 10, 21);             // checked; toString() '2026-10-21'
PlainDate.parse('2026-10-21').add(Span.parse('P1M'));
new PlainTime(13, 45, 0, 500_000_000);   // hour, minute, second, nanosecond
PlainDateTime.parse('2026-10-21T13:45:00.5');
parseTimestamp('2026-10-21T19:15:00+05:30');   // a Date
formatTimestamp(date);                   // '...Z'
normalize('span', '1h 30m');             // 'PT1H30M', every nanosecond kept
toDuration(v);                           // { seconds: bigint, nanoseconds }
fromDuration({ seconds: 90n, nanoseconds: 0 });  // [90n, 0], what new Value() takes
new Span({ days: 1, hours: 2 });         // 'P1DT2H'; fields are bigint, nanoseconds a number
```

- **`PlainDate`, `PlainTime`, `PlainDateTime`** (named after Temporal's)
  hold a civil date and time to the nanosecond with no zone. A
  constructor is checked by the library: `BadValue` for a day the month
  lacks, a time off the clock, or a field past its C width. `equals`,
  `static compare`, `toString()` and `toJSON()` give the canonical text.
- **An instant is a `Date`**, which counts milliseconds: reading drops
  what is below one, toward the past; the offset it was written with is
  not kept. An invalid `Date` is a `RangeError`.
- **An exact length** is `{ seconds: bigint, nanoseconds: number }`;
  `toDuration` reads `[s, ns]`, a number of seconds or `{secs, nanos}` (a
  `Ref` or plain data) and throws `BadValue` for anything else.
- **`Span`** is immutable; `Span.parse` reads ISO 8601 and the friendly
  form (`1h 30m`); `negated()`, `isZero`, `isNegative`, `equals` (unit by
  unit: a day is not 24 hours). Fields of both signs, nanoseconds of a
  second or more, or a unit past a signed 64-bit integer are `BadValue`.
- **Text the grammar refuses throws `Unparsed`** (a `BadValue`) with
  `.form`, `.at` (the byte offset in the UTF-8 text) and `.expected`; it
  never quotes the text.

## Secrets

```ts
using shown = redact(schema, value, '[redacted]');
```

Every field a schema marks (`x-sensitive` or `writeOnly`) is replaced at
any depth, and whatever the schema does not describe is hidden too.
Local `$ref`s are followed. `WrongKind` for a schema that is not a map.

## Errors

Every failing `guatiao_status` throws a `GuatiaoError` (`.status` the raw
code, `.message` the other side's own words, empty when it gave none), as
one of `BadValue`, `AllocFailed`, `WrongKind`, `NotFound`,
`NullArgument`, `Gone`, `Unsupported`, `Busy`, `Internal`.
`new BadValue('why')` carries its class's status. `ParseError` and
`Unparsed` extend `BadValue`. `LibraryNotFound`, `MissingSymbol` and
`FloorMismatch` are this binding's own.

## Threads

A `Value` is used from the thread that made it: Node's main thread, or one
worker, never shared.
