# `guatiao-js` — API header

Header-file-style reference for the `guatiao-js` crate: a guatiao source
serving libraries written in JavaScript, run by QuickJS built into the
file. It ships inside the package and is self-contained. Development
documentation lives with the source at
<https://github.com/Boriken-Dev/guatiao>.

No features. MSRV 1.89. Depends on `guatiao` (`load`), `guatiao-bridge`,
`guatiao-libffi`, `guatiao-serde` (`json`) and `rquickjs` 0.14 (QuickJS-ng
0.16, built from its C sources; `loader`, `array-buffer`, `properties`).
Not published; a workspace of its own. `crate-type = ["cdylib", "rlib"]`.

The root exports `JsEngine`, `JsValues`, `Conversion` and `rquickjs`. The
root is `deny(unsafe_code)`; three small modules allow it, each with a
`SAFETY:` comment: `exports.rs` (`guatiao::source!`), `bytes.rs` (reading
the memory of a typed array QuickJS lends) and `frame.rs` (entering a
context a call is already inside).

## A library (`*.guatiao-js.toml` and its module)

```toml
name = "quickjs_greeter"          # required: the library's id (a package's name)
kinds = ["greeter", "counter"]    # required: every kind its providers serve
version = "1.0.0"                 # optional: else the package.json beside this file
main = "greeters.js"              # optional: else that package.json's entry
paths = ["."]                     # where its files are, relative to this file
```

- **The package** is `package.json` in the manifest's directory, read as
  JSON and never run. `version` is its `version` when the manifest states
  none (neither is a refusal asking for one). The entry is `main`, else
  the package's `exports` (a string, the `.` of a map of subpaths, or the
  first of its conditions, in the order it lists them, that an import
  meets: `import`, `node`, `module-sync`, `default`; never `require`) or
  `main`, else `index.js`, beside the manifest. The entry must lie under `paths`.
- **A stamp** follows the manifest's bytes and the newest modification of
  a `.js`, `.mjs` or `.json` file under `paths` (not under
  `node_modules`), so an edit to any module the library imports is served
  at the next refresh.
- **Opening** gives the library a context of its own (its globals are its
  own; the runtime, its memory limit and its limits are shared), imports
  the entry as an ES module (top-level `await` works for promises that
  settle without timers), and reads the exports: every class extending
  `Provider` is a provider, once, in the order a module namespace lists
  them. Each is built with `new Class()` once, with the library; its
  instance answers every call.
- **Refused** (`GUATIAO_ERR_BAD_VALUE`, in words naming the library): an
  entry that is not a file under `paths`, a module that throws or never
  finishes loading, no exported `Provider`, a class with no `static kinds`
  or naming a kind the manifest does not list, a kind with no description
  from the host (`GUATIAO_ERR_NOT_FOUND`) or described with another floor
  hash, an object kind, a required slot with no method (`Hello does not
  implement greet, which the greeter kind requires`), `static config` or
  `static configSchema` (a provider built from a configuration is not
  served), two providers with one id, an import outside `paths`.

## A provider

```js
import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';
import { Counter, Greeter } from './greeter_kind.js';   // written by guatiao-kind

export class Hello extends Provider {
  static kinds = [Greeter, Counter];
  static displayName = 'Hello, in JavaScript';
  #asked = 0n;
  greet(name) {                                          // a slot: this is the instance
    if (!name) throw new BadValue('nobody to greet');    // a refusal, with its status
    return { greeting: `hello, ${name}` };
  }
  start(listener) { return new Chat(listener); }         // answers an object kind
  count() { return ++this.#asked; }                      // i64: a bigint
}
```

The same file runs under the Node binding (`libraryOf`); the differences
are listed below. Statics, all optional but `kinds`:

| static | is |
| --- | --- |
| `kinds` | the kinds served: values `kindsOf(document)` makes, or the module `guatiao-kind` writes from the document |
| `named` | the name in the provider's id (default: the class name in snake case, `HelloWorld` is `hello_world`) |
| `id` | the whole provider id (default `<library id>_<named>`) |
| `displayName` | shown to a person |
| `meta` | plain data, a map, the provider's `meta` |
| `available()` | text when the provider cannot run here: it is then left out; asked when the library opens |

- **A slot** is the method named for it in camel case (`greet_into` is
  `greetInto`), found on the class's prototype. One it leaves out is null
  in the table: a Rust caller runs the kind's default.
- **Arguments** are the Node binding's shapes except trees: text as
  `string`; `bool` as `boolean`; `i8`..`i32`, `u8`..`u32`, `f32`, `f64` as
  `number`; `i64`, `u64`, `isize`, `usize` (and 128-bit) as `bigint`;
  `bytes` as a `Uint8Array` copy; `bytes_mut` as a `Uint8Array` read back
  after the call; a tree as plain data (an object, an array), a lent one
  changed in place and read back; an object as a handle (below).
- **Answers**: a `number` or `bigint` for an integer slot (the host narrows
  it exactly, refusing one that does not fit), a `string` for text, plain
  data for a tree, a `boolean`, or for an object kind an object with the
  kind's methods (held until the host destroys it; its `[Symbol.dispose]()`
  runs then) or a handle handed back. A slot answers before it returns: a
  promise is `GUATIAO_ERR_INTERNAL`.
- **A refusal** is a thrown `GuatiaoError` (`new BadValue('why')`,
  `new GuatiaoError(13, 'why')`): its status and words cross. Anything
  else thrown is `GUATIAO_ERR_INTERNAL`, `Name: message` or `thrown: text`.
- **A handle** is a host's object handed over or lent: a method per slot
  of its kind, taking the same shapes and answering as above, throwing a
  `GuatiaoError` of the status when the host refuses. `close()` (also
  `[Symbol.dispose]()`) destroys one that was handed over; one dropped is
  destroyed when collected; a lent one answers `Gone` after the call.
  A handle can be passed on to a slot of its kind, or handed back as an
  answer; a script's own object cannot be passed to a host as an object.
  A handle passed to a slot is out of the script's hands from the moment
  its argument is read: while the later arguments of the same call are
  read (a getter, a `toString`), its methods answer `Gone`; it is back
  once the call fails, or after it if it was only lent.

## What a library imports

| specifier | gives |
| --- | --- |
| `guatiao` | `Status`, `GuatiaoError`, `BadValue`, `AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`, `Unsupported`, `Busy`, `Internal`, `errorFor(status, message)`, `ExactNumber` |
| `guatiao/kinds` | `kindsOf(document)` (an object or its JSON text) → kinds by name; `kindOf(description)` |
| `guatiao/provide` | `Provider` |
| `./x.js`, `../y/z.js` | a file under the library's `paths`, by its exact name |

Nothing else resolves (no bare package, no `node:`, no JSON modules, no
URL). QuickJS is given no file, network or process access. Globals: ES2023
and what QuickJS-NG adds (`Array.fromAsync`, `Object.groupBy`, `WeakRef`),
`queueMicrotask`, `console` (`log`, `info`, `debug` to standard output;
`warn`, `error` to standard error), `TextEncoder` and `TextDecoder`
(UTF-8). Not there:
`process`, `Buffer`, `setTimeout`, `fetch`, `URL`, `structuredClone`,
`import.meta` contents.

## What differs from the Node binding

| | Node binding | guatiao-js |
| --- | --- | --- |
| a library's id, version | the package's `name`, `version` | the manifest's `name`; its `version`, else the package's |
| a library's meta | `description`, `license`, `author`, licence files | not carried (a provider's `static meta` is) |
| a provider built from a configuration (`static config`, `s`) | yes | refused |
| a kind defined in JavaScript (`Kind`, `slot`, `t`, `kind()`) | yes | no: kinds come from the host's descriptions |
| a tree argument | `Value`, `MapRef`, `ListRef` | plain data |
| a number in a tree | `number`, or `bigint` beyond 2^53; a decimal as a `number` | the same, and an `ExactNumber` where a double would change the text (`1.10`) |
| bytes in a tree | `Uint8Array` | `Uint8Array`; `ArrayBuffer` also accepted, other views refused |
| a JavaScript object served as an object of a kind | yes | only as an answer, never as an argument to a host |
| `TextEncoder`, `TextDecoder`, `console` | Node's | given (UTF-8 only); `Symbol.dispose` is QuickJS's own |
| the process's other libraries | one realm | a context per library |
| time and stack | Node's | the limits below |

## Values (`JsValues`)

```rust
impl Convert for JsValues<'js> { type Native<'a> = rquickjs::Value<'js>; type Error = Conversion; }
pub fn JsValues::new(ctx: &Ctx<'js>) -> rquickjs::Result<JsValues<'js>>   // gives ctx the guatiao script
```

| guatiao | JavaScript | back |
| --- | --- | --- |
| absent | `undefined` | `undefined` |
| null, bool, string | `null`, `boolean`, `string` | the same; a string that is not Unicode (a lone surrogate) is refused |
| number | an integer: a `number` up to 2^53, a `bigint` beyond; a decimal: a `number` when its shortest text is the text, else an `ExactNumber` (`toString()` is the text, `valueOf()` its float) | an integer `number` as its integer, any other as its shortest text; a `bigint` and an `ExactNumber` as their exact text; `NaN` and the infinities refused |
| bytes | `Uint8Array` | `Uint8Array`, `ArrayBuffer` |
| list | `Array` | an `Array` |
| map | a plain object (a key `__proto__` is data) | a plain object (prototype `Object.prototype` or none), its own enumerable text keys in the order JavaScript keeps them |

Refused: a function, a symbol, a class instance, a `Date`, a `Map`, an
object that contains itself, trees nested deeper than 128, a list longer
than a sixteenth of the memory limit (16 777 216 items without one: holes
cost QuickJS nothing, and the copy is made outside its limit), a buffer a
script asks a host to fill that is longer than the memory limit (64 MiB
without one), and a conversion that adds up to more than that many values
or that many bytes of text, keys and buffers in all (an object that
appears twice is made twice). The bounds are the engine's own, out of the
script's reach. An integer-like
key (`"1"`) comes first in an object whatever its place in the map.

## The engine (`JsEngine`)

`impl guatiao_bridge::Engine`: `ThreadModel::Pinned` (one QuickJS runtime,
not `Send`, on the bridge's thread), manifests `*.guatiao-js.toml` with
`id_key = "name"` and code extensions `js`, `mjs`, `json`,
`Opened::Described`. `JsEngine::memory_used()` is the bytes QuickJS holds
for every library open (0 when asked from inside a call).

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<JsEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "guatiao-js")?.with_mechanism(Arc::new(Libffi)))
}
guatiao::source!(serves = "js", make = start);
```

## Limits

From the configuration's `limits` (the bridge's `Limits`), per call,
opening a library included:

| limit | kept by |
| --- | --- |
| `operations` | QuickJS's interrupt handler, polled once per 10 000 calls and loop iterations; a poll spends 10 000, so the stop comes within one poll past the limit |
| `time_ms` | the same poll |
| `depth` | the native stack QuickJS may use below the call's entry: 740 bytes per level (measured on Linux x86-64; a level takes more on other targets and in debug builds), at least 16 KiB and at most 1 MiB, plus 64 KiB (256 KiB in a debug build) for opening a library; a stack overflow while it is set is the limit |
| `memory` | QuickJS's allocator, for the whole runtime, in bytes; 128 KiB of it (a quarter, if the limit is small) is held back, so that the error that stops a call can always be made |

A stop is `GUATIAO_ERR_INTERNAL` ("stopped at its operations limit
(200000)"), or `GUATIAO_ERR_ALLOC` for memory; the runtime answers the
next call. The interrupt cannot be caught by `try`/`catch`/`finally`.
Without a depth limit QuickJS stops a recursion at 1 MiB of stack
(about 1 400 levels of an empty function, release build, Linux x86-64) with a `RangeError`, `GUATIAO_ERR_INTERNAL`.
Promise reactions a call leaves queued run when the outermost call
returns, whatever it came to (never inside a call made from inside
another), under the same budget. If the budget runs out there, or before,
the call is the one stopped, whatever its method answered; the reactions
still queued are then ended without running: each is taken off the queue
on a runtime with no stack and no memory, so it fails at its first call
and can queue no other. However many a call left, none outlives it, and
ending them costs the same whatever they would have run.

## Errors

| JavaScript | status |
| --- | --- |
| `throw new BadValue('why')` and the other `GuatiaoError`s | their status |
| any other thrown value | `GUATIAO_ERR_INTERNAL`, its first line |
| a value that cannot cross as the slot's answer | `GUATIAO_ERR_BAD_VALUE`, or `GUATIAO_ERR_WRONG_KIND` for a wrong kind of answer |
| QuickJS's out of memory | `GUATIAO_ERR_ALLOC` |
| QuickJS's out of memory with no memory left even for its error | `GUATIAO_ERR_INTERNAL`, `thrown: null` |
| a limit | as above |

QuickJS throws `null` when the memory limit leaves no room to make its
`out of memory` error, which happens when the limit is reached by many
small allocations rather than one large one; that `null` cannot be told
from a script's own `throw null`.

## The C surface

```c
guatiao_status guatiao_source(const guatiao_host_info *host,
                              const guatiao_value *config,      /* a map, or null */
                              const guatiao_source_info **out,
                              guatiao_provider_error *err);
```

As `guatiao.h` declares any source's entry. The file declares
`source=js`, so a scan reports it as a source and never maps it as a
library. A configuration that is not a map is `GUATIAO_ERR_WRONG_KIND`; a
key that is not a list of text, or a misspelled limit,
`GUATIAO_ERR_BAD_VALUE`.

## Gotchas

- **A context per library, one runtime.** Libraries share nothing but the
  runtime's memory limit and its thread: a handle is known to the library
  it was made for alone, by a key only that library's context holds, so
  one cannot hand back another's object. An edited library is a new
  context; the old one is dropped (and the cycles it held collected) when
  its last instance goes.
- **No event loop.** There are no timers; promise reactions run to
  completion after each call and after the module loads.
- **A call from a host's callback runs inline.** A script calling a host
  that calls another provider of this source is served on the same thread
  without waiting. A host that calls this source from another thread while
  the QuickJS thread waits on that thread waits on itself.
- **Handles are destroyed after the call.** An object a script drops, or
  closes, is destroyed by the host once no script is running (when the
  outermost call returns), because QuickJS frees it in its collector,
  where nothing may run; a library's `Symbol.dispose` runs then too.
- **A link does not lead out of `paths`**: imports are resolved to the
  files they name, links followed, before they are checked.
- **A limit is checked at a poll**: one long native call (a `repeat`, a
  sort) runs to its end; the memory limit bounds what it can build.
- **Stack per level is approximate** and differs by target and build
  profile.
- **`cargo test` does not build the file**: `cargo build` first, or the
  tests adding it by file state a skip.
