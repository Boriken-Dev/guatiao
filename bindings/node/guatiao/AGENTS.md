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
| `guatiao` | `Value`, `Ref`, `MapRef`, `ListRef`, `Registry`, `Instance`, `Tag`, `Status`, `redact`, `useLibrary`, the errors |
| `guatiao/kinds` | `kindsOf`, `kindOf`, `Kind`, `slot`, `kind`, `t`, `Implementation`, `Remote`, `ObjectHandle`, `DescribedKind`, `liveObjects` |
| `guatiao-kind` (bin) | writes `<base>.js` and `<base>.d.ts` from a kinds description |
| `guatiao/provide` | `Provider`, `library`, `libraryOf`, `Library`, `s` (configuration schemas) |
| `guatiao/time` | `PlainDate`, `PlainTime`, `PlainDateTime`, `Span`, instants, exact lengths |
| `guatiao/wire` | `announce`, `send`, `close`, `Receiver`, `Frame`, `Event`, the refusals |
| `guatiao/ffi` | for a package over another guatiao library: `openSurface`, `Surface`, memory, `adopt`, `borrow`, `nodeOf`, `withNodes` |

JSON, TOML and YAML are the package `guatiao-serde`, forms
`guatiao-intake`: each over its own library, built on `guatiao/ffi`.

## Finding the native library

Importing the package touches nothing native: koffi loads on the first
call that needs it (or when a kind is described), and the library
on the first call that needs it, found in this order:

1. `useLibrary(pathOrUrl)`, when called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded throws; naming the same one does
   nothing.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file, taken as it is.
3. The platform's own library search, by file name.

Not found throws `LibraryNotFound` naming each route. A symbol the loaded
build does not export throws `MissingSymbol` (`.symbol`, `.feature`) when
called.

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

## Registry

```ts
import { Registry } from 'guatiao';

using reg = new Registry('my-host', '1.0');
reg.scanDir('/path/to/plugins', { rules: 'kind=greeter' });   // rules: newline-separated
reg.providers('greeter');        // plain objects, best first
reg.best('greeter');             // { key, id, version, library, display_name, from, kinds, has_config, vtable_size }
reg.whyNot('codec');             // { available: false, why: 'nothing-claims-it', ... }
const greeter = reg.bind('acme_hello', Greeter);                // a Remote (see Kinds)
using instance = reg.create('acme_shouter', { prefix: 'hey' }); // an Instance
instance.bind(Greeter).greet('ana');
```

Also `loadFile(path)`, `scanPath(spec, { descending, rules })`,
`registerEntry(name, entry)` (the address of a `guatiao_library_entry`),
`registerLibrary(library)` (see Providers), `libraries()`,
`provider(key)`, `available(kind)`, `providerAvailable(key)` (`{
available, reason? }`), `keyedBy(template)` / `librariesKeyedBy`,
`setPriority` / `priority`, `providerTable(key, kind?)` (`{ table, size,
ctx }`, `table` `0n` for none), `providerConfig(key)` (a borrowed,
read-only `Ref` onto the schema, never closed), `providerMeta(key)` (a
copy of the provider's `meta` as a `Value` you close, `undefined` when it
declares none, `NotFound` for an unknown key; its form is under `'form'`,
`providerFormKey` of `guatiao-intake`), `host()` (the
`guatiao_host_info *`), `close()` / `using`. Every answer that is data is
a plain object or array, already freed.

`bind(key, kind)` answers the provider's table for that kind as a
`Remote` over its own instance: `NotFound` when it serves none,
`FloorMismatch` when the table is not the one the kind describes. An
`Instance` (`create(key, config)`, the configuration checked against the
provider's schema first) binds the same way; its `close()` reaches the
provider that built it, even after a retire or a refresh that loaded a
new build under the key.

`retire(libraryKey)` takes a library out and leaves it mapped until
nothing uses it; `collect()` closes every retired library whose own
`unload` agrees and answers their keys, as does the next load, retire or
unload. `unload(libraryKey, { unchecked })` unmaps when the library
agrees: `Busy` while something it handed out lives, `Unsupported` with no
`unload` function, `WrongKind` for a library linked rather than mapped,
`NotFound` for none. **Both take the LIBRARY key** (`%id` by default). No
table, `ctx` or borrowed schema taken from it is used again.

**Sources**: `addSourceFile(path, config)` maps a file exporting
`guatiao_source` (a bridge to another language) and adds the source it
answers, refusing with its words; `watchDir(dir, { rules })` watches a
directory; `refresh()` makes the registry agree with every source,
answering `{ loaded, retired, skipped, refused }`. Nothing refreshes on
its own.

## Kinds: calling a table, serving one

**A kind that exists is loaded, never declared again.** A kind crate
commits its kinds' description beside their C header
(`<crate>.kinds.json`: `{ format: 1, kinds: [{ name, floor_hash, object,
slots: [{ name, required, fallible, args, answer }] }] }`, the format on
the docs site's encodings page). Two ways to load it:

- **`guatiao-kind DOCUMENT [--out BASE] [--from MODULE] [--check]`**
  writes `BASE.js` (it calls `kindsOf` on the document, embedded) and
  `BASE.d.ts`, both committed. For each kind the declarations hold an
  interface of the kind's name in PascalCase (`greeter` is `Greeter`; its
  required slots as methods, the appended ones optional, each slot's name
  in camelCase), a `<Kind>Remote` interface (what a caller holds), and a
  value of the kind's name (`DescribedKind<Greeter, GreeterRemote>`):
  TypeScript merges the two, so `implements Greeter` and `static kinds =
  [Greeter]` name one import. `BASE` defaults to the document's path
  without `.kinds.json`; `--from` is the guatiao package's specifier
  (default `guatiao`); `--check` writes nothing and exits 1 when the files
  are not what it would write. A slot naming an object kind outside the
  document is refused.
- **`kindsOf(document, { known? })`** (the object, or its JSON text)
  answers each kind by its name; **`kindOf(description, { known? })`** one
  kind. An object kind a slot names is found in the document, else among
  `known`. `TypeError` naming the kind, the slot and the argument for a
  document that is not one, a `type` it does not know, or kinds that refer
  to each other in a cycle.

```ts
import { Greeter } from './greeter_kind.js';   // written by guatiao-kind
export class Hello extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  greet(name: string) { return { greeting: `hello, ${name}` }; }
}
```

**A kind defined here**, with no header behind it, is a class extending
`Kind` whose static fields describe it: `kind` (its name; default the
class name in snake_case), `floorHash`, `object` (an object kind), and
`slots`, a `slot(...args)` per slot in table order, refined by
`.returns(answer)`, `.required()` and `.fallible()`. The class itself is
the kind wherever one is asked for; it is described on first use, which
must be at module level like `kind()`. `Implementation<typeof Echo>` and
`Remote<typeof Echo>` read its statics.

```js
import { Kind, slot, t } from 'guatiao/kinds';

export class Echo extends Kind {
  static kind = 'echo';
  static floorHash = 0x6563686f;
  static slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
  };
}
```

**Underneath both, `kind()`**: a method per slot in the header's order
(the `methods` object's order), the shapes from `t`, and the header's
`<table>_FLOOR_HASH`. TypeScript infers what a provider implements and
what a caller holds.

```ts
import { type Implementation, kind, t } from 'guatiao/kinds';

export const Listener = kind('listener', {
  floorHash: 1867921573,
  object: true,
  methods: { heard: { args: [t.str], required: true } },
});
export const Greeter = kind('greeter', {
  floorHash: 2695856491,
  methods: {
    greet: { args: [t.str], answer: t.map, required: true, fallible: true },
    shout: { args: [t.str], answer: t.text },
    greetInto: { args: [t.str, t.lent(Listener)], fallible: true },
    check: { args: [t.lent(t.map)], answer: t.bool, fallible: true },
  },
});
export type Greeter = Implementation<typeof Greeter>;
```

`kind(name, { floorHash, object?, methods })`: `required` marks a slot
every table fills (the kind's original ones), `fallible` one ending in
`guatiao_provider_error *err`, `object: true` an object kind (`destroy`
first). A slot is found by position, so a method's name is free; it may
not be `has`, `close`, `closed`, `kind`, `ctx`, `size`, `table`, `then`
or a name `Object.prototype` has. A description that is not one throws
`TypeError` naming the method. `Implementation<typeof K>` is what a
provider or an object implements (required methods mandatory, the rest
optional); `Remote<typeof K>` what a caller holds; `ObjectHandle<typeof
K>` an object of an object kind the caller owns.

| shape | C | a caller passes / gets | a method receives / returns |
| --- | --- | --- | --- |
| `t.bool`, `t.i8`..`t.i32`, `t.u8`..`t.u32`, `t.f32`, `t.f64` | the scalar | `boolean` / `number`, range-checked (`RangeError`) | the same |
| `t.i64`, `t.u64`, `t.usize`, `t.isize` | the scalar | `number` or `bigint` / `bigint` | `bigint` / `number` or `bigint` |
| `t.str` | `guatiao_str` | `string` | `string` (`BadValue` if not UTF-8) |
| `t.bytes` | `guatiao_bytes` | a `Uint8Array`, `ArrayBuffer` or typed array | a `Uint8Array` copy |
| `t.bytesMut` | `guatiao_bytes_mut` | a `Uint8Array`, written in place | a `Uint8Array`, copied back when it returns |
| `t.text` (answer) | `guatiao_string *out` | `string` | `string` |
| `t.value` | `const guatiao_value *` (null allowed) | a `Ref`, plain data or `null` | a read-only `Ref` or `null`, valid for the call |
| `t.map` | `const guatiao_map *` | a `Ref` or plain data | a read-only `MapRef`, valid for the call |
| `t.taken(t.map)`, `t.taken(t.list)`, `t.taken(t.value)` | a tree handed over | a `Value` (moved: left absent), a `Ref` (copied) or plain data | a `Value` it owns |
| `t.lent(t.map)`, `t.lent(t.list)`, `t.lent(t.value)` | a tree lent (`&mut`) | a `Value` or `Ref`, changed where it stands | a writable `MapRef` / `ListRef` / `Ref`, valid for the call; what it adds is built in the allocator the tree records |
| `t.map`, `t.list`, `t.value` (answer) | `guatiao_map/list/value *out`, a carrier | a `Value` to close | a `Value`, a `Ref` or plain data, built in the allocator the carrier names |
| `t.taken(K)`, `K` an object kind | `guatiao_object` | an object implementing `K`, or an `ObjectHandle` | an `ObjectHandle` it owns |
| `t.lent(K)` | `const guatiao_object *` | an object implementing `K`, or an `ObjectHandle` | a `Remote`, valid for the call |
| `K` (answer) | `guatiao_object *out` | an `ObjectHandle`; `close()` destroys it once | an object implementing `K`, or an `ObjectHandle` |
| no `answer` | -- | `undefined` | anything, ignored |

A description names the same shapes: its scalars as `t` does (`i64`,
`usize`, ...), `str`, `bytes`, `bytes_mut` (`t.bytesMut`), `text`;
`value`/`map`/`list` with `pass` `borrowed` (`t.value`, `t.map`),
`taken` (`t.taken(...)`) or `lent` (`t.lent(...)`); `object` with its
`kind`, `pass` `taken` or `lent`, and as an answer the object kind
itself.

**A `Remote`**: a method per slot, plus `has(name)` (whether the table
carries it), `kind`, `table`, `size`, `ctx`. A slot past the table's size
or null throws `Unsupported` (a Rust caller would run the kind's default
body, which JavaScript cannot); a refusal throws the status's
`GuatiaoError` with the provider's words; a slot that cannot fail and
fails throws its status. Valid while the library is. `kind.bind(table,
size, ctx)` makes one over any table. **An `ObjectHandle`** is a `Remote`
whose `close()` (or `using`) calls `destroy` once; one dropped unclosed is
destroyed when collected; handed to a slot that takes it, it is the
callee's and `close()` does nothing.

**A JavaScript object as an object of a kind**: any object with the
required methods. Its table is built once per kind and set of methods it
has (a missing optional method's slot is null) and kept for the process.
It is held until its holder destroys it (handed over), when its
`[Symbol.dispose]()` runs if it has one, or until the call returns
(lent). `liveObjects()` counts what is held.

**A refusal**: a method throws a `GuatiaoError` (`new
BadValue('nobody to greet')`) and the caller reads that status and
message; any other error crosses as `GUATIAO_ERR_INTERNAL` with `"Name:
message"`. A slot that cannot fail has no `err`: the error is reported as
a `GuatiaoWarning` (`process.emitWarning`) and the status crosses alone.
A method answers before it returns: a promise is refused as `Internal`.

**Describe kinds and build libraries at module level.** koffi cannot
register a callback from inside one, so `kind()` and `kindsOf()` register
every slot's callback when they run, a kind class when it is first used,
and `library()` its descriptor's; each throws when called inside a call a
library is making into this binding.

## Providers written in TypeScript or JavaScript

```ts
// greeters.ts, in a package named node_greeter
import { BadValue } from 'guatiao';
import { Provider, s } from 'guatiao/provide';
import { Counter, Greeter } from './greeter_kind.js';

export class Hello extends Provider implements Greeter, Counter {
  static override readonly kinds = [Greeter, Counter];
  static override readonly displayName = 'Hello, in Node';
  #n = 0n;
  greet(name: string) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello, ${name}` };
  }
  count() {
    return ++this.#n;
  }
}

export const ShouterConfig = s.object({ prefix: s.string() });
export class Shouter extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  static override readonly config = ShouterConfig;
  readonly #prefix: string;
  constructor(config: s.Infer<typeof ShouterConfig>) {
    super();
    this.#prefix = config.prefix;
  }
  greet(name: string) {
    return { greeting: `${this.#prefix} ${name.toUpperCase()}` };
  }
}
```

**A provider** extends `Provider`, `implements` its kinds' types (which is
what makes `tsc --strict` check its methods) and lists them in `static
kinds`. A method a kind requires and the class lacks is a `TypeError` when
a library is built from it, as is an object kind among its kinds. Static
fields: `named` (default: the class name in snake_case), `id` (default
`{library id}_{named}`), `displayName`, `version` (empty: the library's),
`meta` (plain data), `config`, `configSchema`, `default`, and `static
available()` (`undefined` or `null` when it can run here, else why not;
asked every time a host asks). Methods must be on the prototype: an arrow
function in a field is not seen.

**In plain JavaScript** the same class drops the types and `override`
(`static kinds = [Greeter, Counter];`) and runs as it is, with no build
step; a missing required method is the same `TypeError` (`Hello does not
implement greet, which the greeter kind requires`). With `// @ts-check`,
`/** @implements {Greeter} */` above the class has an editor check its
methods against the generated `.d.ts`.

**Configuration**: with neither `config` nor `configSchema` the provider is
its one instance, `new cls()`, built with the library. `static config =
s.object({...})` builds an instance per configuration the host checked
against the schema `s` wrote, the constructor receiving the built object
(defaults filled in); `configSchema` (a raw JSON Schema) does the same and
the constructor receives plain data. `default = true` also offers `new
cls()`. An instance its host destroys runs its `[Symbol.dispose]()` when
it has one.

**`s`**: `s.string(kw?)`, `s.integer(kw?)` (a `number`; an integer a
`number` cannot hold exactly is a `RangeError`), `s.number(kw?)`,
`s.boolean(kw?)`, `s.literal(...values)`, `s.any(kw?)`, `s.array(item,
kw?)`, `s.record(value, kw?)`, `s.nullable(inner)`, `s.optional(inner, {
default? })`, `s.object(fields, kw?)` (refuses keys it does not declare;
a member not `optional` is required); `kw` are JSON Schema keywords added
to what the builder writes. `s.Infer<typeof X>` is the built type, `.json`
the schema, `.build(data)` the configuration.

**A module is a library**: `await libraryOf(urlOrPath)` imports an ES
module and builds its library once: its id is the nearest `package.json`'s
`name` (with the module's path in the package appended, extension
dropped, when it is not the package's entry point), its version that
file's `version`, its providers the `Provider` subclasses it exports in
the order a module namespace lists them (export names sorted), its `meta`
that file's `description` (as `summary`), `license`, `author` and
`contributors` (as `authors`), `homepage`, `repository` and `bugs` (as
`urls`), and each licence file (`LICENSE*`, `LICENCE*`, `COPYING*`) as a
`notices` entry. `TypeError` for a module outside a package, a package
with no name or version, or a module exporting no provider.

`library(id, version, { providers, meta })` is the same without a module.
A `Library` has `id`, `version`, `providers`, `kinds`, `meta`, `entry` (a
`guatiao_library_entry`), `infoAddress`, `liveInstances()` and `close()`
(lets go of each provider's own instance; the entry answers null from
then on). Its descriptor and callbacks are kept for the process, and its
`unload` slot answers `GUATIAO_ERR_BUSY` while an instance lives.
`Registry.registerLibrary(library)` files it under its id and keeps it:
it is linked, not mapped, so `unload` answers `WrongKind` and `retire`
takes it out.

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

## wire: values as bytes, and channels

`ref.toWire(): Uint8Array` (any `Ref`) and `Value.fromWire(data,
options?)` are the wire encoding: a tag byte per node, LEB128 lengths, a
number as its own text, bytes as bytes, a map in its order, the same
bytes in every language. `fromWire` throws `BadValue` for bytes that are
not exactly one value.

A channel announces a schema before its values; a receiver checks every
value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```ts
import * as wire from 'guatiao/wire';

const frames = [wire.announce(1, schema), wire.send(1, { greeting: 'hi' }), wire.close(1)];
using receiver = new wire.Receiver();
for (const frame of frames) {
  const event = receiver.accept(frame); // { kind, channel, value }
  event.value?.close();                 // the schema's copy, or the checked value
}
```

- `announce(channel, schema)`, `send(channel, value)` (a `Value`, a `Ref`
  or plain data), `close(channel)`, each a `Uint8Array`. A channel is
  `number | bigint`, 0 to 2^64 - 1.
- `new Receiver()`; `accept(frame: Uint8Array): Event`; `close()` or
  `using` (a `FinalizationRegistry` is the backstop). `Event`: `kind` a
  `Frame` (`Frame.SCHEMA` 0, `VALUE` 1, `CLOSE` 2), `channel` a `bigint`,
  `value` a `Value` the caller closes, or `null`.
- A refusal throws one of `Refused` (`channel`, `kind`, `message`), each
  also the status's class: `Malformed` (a `BadValue`, `at` the byte of the
  frame), `NoSchema` (`NotFound`), `NotASchema` (`WrongKind`), `Invalid`
  (`BadValue`; `key` the field, `expected` or `known`). It changes nothing
  the receiver keeps and quotes nothing it was given.

## Secrets

```ts
using shown = redact(schema, value, '[redacted]');
```

Every field a schema marks (`x-sensitive` or `writeOnly`) is replaced at
any depth, and whatever the schema does not describe is hidden too.
Local `$ref`s are followed. `WrongKind` for a schema that is not a map.

## ffi: for a package over another guatiao library

`guatiao/ffi` is what `guatiao-serde` and `guatiao-intake` build on, and
what any package binding a C library that takes and answers guatiao
values may use. Supported API, versioned with this package. Such a
package never loads koffi itself: this one's koffi declares every library
and the by-value structs (`guatiao_str`, `guatiao_bytes`,
`guatiao_object`).

```ts
import * as n from 'guatiao/ffi';

const lib = n.openSurface('mylib', [
  ['uint32_t guatiao_alloc_default(void *out)'],
  ['uint32_t mylib_parse(guatiao_str text, void *alloc, void *out)'],
], { variable: 'MYLIB_LIBRARY', probe: 'mylib_parse' });
const out = n.alloc(n.SIZE.value);
n.raiseForStatus(lib.fn.mylib_parse!(n.str('a=1'), lib.alloc, out));
using value = n.adopt(out, lib.alloc);       // a Value; it frees the tree and `out`
```

- `openSurface(basename, exports, { chosen?, variable?, probe? }) ->
  Surface`: `chosen` (a file, or a directory holding the platform's file
  name), then the variable `variable` (the same), then `GUATIAO_LIBRARY`
  (a directory; or a file, used when it exports `probe` or no `probe` is
  given, else the file beside it), then the platform's search.
  `LibraryNotFound` names every place tried. Not cached: keep the
  `Surface`. `exports` are `[prototype, feature?]` pairs in koffi's C
  syntax; one the library lacks becomes a function throwing
  `MissingSymbol` naming `feature`. `libraryFilename(basename)`.
- `Surface`: `.path`, `.fn[name]`, `.missing` (a set of names), `.alloc`
  (the address of its `guatiao_alloc_default`, filled once).
- Memory: `alloc(len)` / `release(ptr)` (zeroed memory koffi keeps),
  `str(text)` (a `guatiao_str` argument), readers `u8`, `u32`, `i32`,
  `u64`, `i64`, `ptrAt`, `size`, `bytesAt`, `textAt`, `stringText` (an
  owned `guatiao_string`), writers `setU8`, `setU32`, `setU64`, `setPtr`;
  `freeNode(node)` frees what a node owns through the core library. The
  layout: `SIZE`, `VALUE`, `ENTRY`, `OWNED`, `ALLOC`; `Status`, `Tag`.
  `koffi()` is the one koffi, for a package's own callbacks.
- Trees: `adopt(root, alloc?) -> Value` takes over a root a call wrote
  into memory from `alloc()`; `withNodes(items, body)` lends `Ref`s as
  they are and plain data built for the call; `nodeOf(ref)` is a `Ref`'s
  node now; `borrow(source, find) -> Ref` views the node `find()` answers,
  read-only, refused once `source` closes. `raiseForStatus(status,
  message?)` throws the status's `GuatiaoError`.

**A tree crosses libraries as its root node**: built by one library, it
is read and freed by any other, since a value records its own allocator.
The library that built it stays loaded while the tree lives.

## Errors

Every failing `guatiao_status` throws a `GuatiaoError` (`.status` the raw
code, `.message` the other side's own words, empty when it gave none), as
one of `BadValue`, `AllocFailed`, `WrongKind`, `NotFound`,
`NullArgument`, `Gone`, `Unsupported`, `Busy`, `Internal`.
`new BadValue('why')` carries its class's status. `Unparsed` extends
`BadValue`. `LibraryNotFound`, `MissingSymbol` and
`FloorMismatch` are this binding's own.

## Threads

Everything here runs on the thread that loaded it, Node's main thread: a
`Value`, a `Registry` and a `Remote` are used there, never shared.

A library may call a table this binding serves (a provider, a lent or
handed-over object) from any thread. A call from a thread Node did not
start is queued and runs on Node's main thread when its event loop next
turns, the calling thread waiting for the answer, so calls into
JavaScript are serialised. **A native call made synchronously from Node's
thread that waits for such a thread never returns**: the queued call
cannot run until it does.

koffi runs a synchronous native call on a stack of its own. Another
runtime embedded in the process that judges its stack by address can
misjudge it: on macOS, CPython 3.14 served through guatiao-py in a Node
process does not free an object dropped during such a call. On Linux the
bridge tells CPython which stack the call runs on, and it frees, as it
does on Windows and FreeBSD.
