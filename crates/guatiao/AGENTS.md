# guatiao — API header

The public surface of the `guatiao` crate, so a consumer does not have to
read the source. Current with the crate it ships beside.

**What it is.** One value model for crossing a language boundary: plain
`repr(C)` structs and no opaque handles. Three layers, each usable
without the ones above it:

| layer | answers | module |
| --- | --- | --- |
| **Value** | what is being passed | `value` |
| **Schema** | what that value is, and what a valid one looks like | `schema` |
| **Library** | who is offering it, and how a host loads them | `library` |

A C, C++ or Dart consumer reads a whole tree through
`include/guatiao.h` with no call into any library.

## Features

| flag | adds | default |
| --- | --- | --- |
| `derive` | `#[derive(ToValue, FromValue, Schema)]` | off |
| `provider` | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!` (implies `derive`; turns on `syn/full` in the derive crate) | off |
| `load` | `Registry` and the scans (`load_file`, `scan_dir*`, `scan_path*`, `probe`): everything a host does | off |
| `regex` | enforcing a schema's `pattern` in `validate_*` (carried and read either way) | off |
| `c-header` | regenerating the committed `include/guatiao.h` | off |

MSRV 1.89. No required dependencies; `derive` pulls `guatiao-derive`,
`load` pulls `libloading` and `object`, and `regex` pulls `regex`.

---

# Building a value

**Building in Rust names no allocator.** The short forms (`Map::new`,
`Text::new`, `set`, `push`, `push_str`) use the allocator the container
recorded, or the crate's own, and **abort** on allocation failure through
`alloc::alloc::handle_alloc_error`, exactly as `String::from` and
`Vec::push` do: they return no `Result`. The `_in` forms name an allocator
and the `try_` forms use the recorded one; both stay fallible, because a
foreign allocator refusing (a full arena) is recoverable.

```rust
use guatiao::Map;

let mut fields = Map::new();
fields.set("compression", 6);

let mut map = Map::new();
map.set("host", "10.0.0.1");
map.set("port", 5900);
map.set("tls", true);
map.set("fields", fields);
```

`set` and `push` take `impl Into<Value>`, which is implemented for
`&str`, `String`, `&String`, `&[u8]`, `Vec<u8>`, `bool`, every integer
width (`i8`..`i128`, `u8`..`u128`, `isize`, `usize`), `Number`, `Map`,
`List`, `Text`, `Buffer`, and `Option<T: Into<Value>>` (`None` stores a
null). Floats convert through `TryFrom<f64>`/`TryFrom<f32>`, because
`NaN` and the infinities have no representation here.

## Containers

`Map::new()` hands back a **`Map`**, not a value; `map.into()` makes it a
`Value` at the one point something wants one. All four containers free
what they own on drop.

```rust
Map::new() -> Map                      List::new() -> List
Map::new_in(alloc: Alloc) -> Map       List::new_in(alloc: Alloc) -> List
map.alloc() -> Result<Alloc, ValueError>
map.len() / map.is_empty() / map.entries() -> &[Entry]
map.set(key: &str, value: impl Into<Value>)                 // aborts on OOM
map.try_set(key, impl Into<Value>) -> Result<(), AllocError>  // recorded allocator
map.set_in(key, impl Into<Value>, alloc) -> Result<(), ValueError>
map.get(key) -> Option<&Value>         map.get_mut(key) -> Option<&mut Value>
map.required(key) -> Result<&Value, MapError>   // names the key it lacks
map.contains_key(key) -> bool
map.push_into(key, impl Into<Value>)   // appends to the list under `key`
map.remove(key) -> Option<Value>       // handed back, frees on drop
map.discard(key) -> bool               map.clear()
map.copy_from(&Map) / copy_from_in(&Map, alloc) -> Result<usize, ValueError>
map.iter() / map.keys() / map.values()
map["host"] -> &Value                  // panics when absent, as HashMap does
map.clone_in(alloc) -> Result<Map, ValueError>
map.is_local() -> bool                 // every container and key records Alloc::rust()
map.into_local() -> Map                // unchanged when local, else copied, original freed
map.into_alloc(alloc) -> Result<Map, ValueError>   // the same, for a named allocator
map.make_local_in(alloc) -> Result<(), ValueError> // in place, through &mut: moves only what alloc did not make
for (key, value) in map { .. }         // owned: (Text, Value); &Map: &Entry
for (key, value) in &mut map { .. }    // (&Text, &mut Value), as map.iter_mut(): a key never changes in place

list.push(impl Into<Value>)            list.push_in(impl Into<Value>, alloc)
list.try_push(impl Into<Value>) -> Result<(), AllocError>
list.remove(i) / list.pop()            list.discard(i) -> bool     list.clear()
list.clone_in(alloc) -> Result<List, ValueError>
list.is_local() / list.into_local() / list.into_alloc(alloc) / list.make_local_in(alloc)   // as on Map; Text and Buffer too
for value in list { .. }               // owned: Value; &List: &Value; &mut List: &mut Value

Text::new(&str) -> Text                Buffer::new(&[u8]) -> Buffer
Text::new_in(alloc, &str) -> Result<Text, ValueError>
Buffer::new_in(alloc, &[u8]) -> Result<Buffer, ValueError>
&*text -> &str                         // Deref<Target = str>: every str method
text.push_str(&str) / try_push_str(&str) / push_str_in(&str, alloc)
buffer.push(&[u8]) / try_push(&[u8]) / push_in(&[u8], alloc)

Number::new(&str) -> Result<Number, ValueError>     // the JSON grammar
Number::new_in(alloc, &str)            Number::float_in(alloc, f64)
&*number -> &str                       // Deref<Target = str>: every str method

text == "host" / "host" == text       // and Number; Text is Ord by its str
Entry::key() -> &str                   Entry::value() -> &Value
Entry::value_mut() -> &mut Value       // the key is not offered this way
Text::default() / Buffer::default()    // empty, allocating nothing
```

Every name above is at the **crate root**: `guatiao::Text`,
`guatiao::Entry`, `guatiao::Tag`, `guatiao::Str`, `guatiao::Number`,
`guatiao::Payload` (what `from_raw_parts` takes), beside `Alloc`,
`AllocError` (what the `try_` forms answer), `Buffer`, `Kinds` (beside
`MapError`), `List`, `Map`, `MAX_DEPTH`, `Span` (under "Field types"),
`Status`, `TryAsMut`, `TryAsRef`, `Value` and `ValueError`.

**The short form uses the allocator the container recorded**; the `_in`
form adopts one and is what a literal with a null `alloc` field needs.
Both are safe: a `&mut Map` in safe code is a well-formed container, and
the value is consumed.

**`guatiao::Bytes` is the conversion marker**, not the borrowed view: it
is the field type that says "cross as the bytes kind". The borrowed view
of the same name stays at `guatiao::value::types::Bytes`, with
`Bytes::new(&[u8])` and `Bytes::empty()` mirroring `Str`.

**The views borrow**: `Str<'a>`, `Bytes<'a>`, `Values<'a>`,
`Entries<'a>` and `BytesMut<'a>` carry the lifetime of what they view, as
a zero-sized marker C never sees. Their fields are private; read them
with `ptr()`, `len()` and `is_empty()`.

```rust
Str::new(&'a str) -> Str<'a>            Bytes::new(&'a [u8]) -> Bytes<'a>
unsafe Str::from_raw_parts(ptr, len) -> Str<'a>          // trusts, as String::from_raw_parts
<&str>::from(view)                      <&[u8]>::from(bytes)   // never fail
<&[u8]>::from(view)                     // a view's bytes, claiming nothing
```

The types whose content is checked (`Str`, `Text`, `Number`, `Map`) are
made the way std makes `String`: a checked door, and `from_raw_parts`
that trusts.

```rust
Str::new(&str)  /  core::str::from_utf8(bytes)?.into()   // Str or Text
unsafe Text::from_raw_parts(..) / Map::from_raw_parts(..) // trusts
const unsafe Str::from_ptr(*const Str<'a>) -> Result<Str<'a>, ValueError>      // UTF-8
const unsafe Text::from_ptr(*const Text) -> Result<&'a Text, ValueError>      // UTF-8
const unsafe Number::from_ptr(*const Number) -> Result<&'a Number, ValueError> // grammar
const unsafe Map::from_ptr(*const Map) -> Result<&'a Map, ValueError>          // keys
```

`from_ptr` is what reads something C points at, as `CStr::from_ptr`
does: the caller promises the memory, and the content is checked. A
`Value` is read through its doors (`TryAsRef`, …), which check the same.

A `Str` is UTF-8 from the moment it is made: `new` takes a `str`,
`str::from_utf8` checks bytes on the way to one, and the caller of
`from_raw_parts` promises, as with `String`. A `Str` C passes into an export or a shim was made by C, so the
export makes it again from its bytes, `str::from_utf8(view.into())`; text
that is not UTF-8 is `GUATIAO_ERR_BAD_VALUE`. A `*const Str` is read with
`Str::from_ptr`, `unsafe` as `CStr::from_ptr` is, since only a raw read
can follow a pointer; the caller promises the memory and the text is
checked. Null is an empty view. A
null pointer views nothing. `BytesMut` is neither `Clone` nor `Copy`,
since it converts to a `&mut [u8]`. Descriptors carry the lifetime of
their views (`LibraryInfo<'a>`, `ProviderInfo<'a>`, …); one a library
exports is `'static`; one a host registers itself is borrowed for the
registration, which copies what it says.

**Collecting into a container** is the standard trait, as for `Vec` and
`HashMap`: `["a", "b"].into_iter().collect::<List>()`, and
`[("host", "h"), ("port", "p")].into_iter().collect::<Map>()`. `Extend`
appends to either. Both grow through the container's allocator and abort
if it refuses, as the short constructors do; a map keeps insertion order
and a repeated key replaces its value in place.

**Setting an existing key replaces it in place**, keeping its position.
Map order is insertion order and is part of the contract.

## Values

**A node is what a container becomes.** `Value` carries only what has no
container to live on:

```rust
Value::null()      Value::absent()     Value::default()   // = null
v.tag() -> Result<Tag, ValueError>     v.alloc() -> Result<Alloc, ValueError>
v.clone_in(alloc) -> Result<Value, ValueError>
v.is_local() -> bool                   v.into_local() -> Value
v.into_alloc(alloc) -> Result<Value, ValueError>
v.make_local_in(alloc) -> Result<(), ValueError>
unsafe Value::from_raw_parts(tag: u32, Payload)   v.into_raw_parts() -> (u32, Payload)
unsafe { v.free() }                    // the boundary's spelling of drop
unsafe { v.write_to(out: *mut Value, alloc: *const Allocator) } -> Status
```

`write_to` is how an export of your own hands an answer to a C caller:
`GUATIAO_ERR_NULL` for a null `out`; a null `alloc` writes the tree as
built (every container records its allocator, so `guatiao_value_free`
frees it right wherever it lands); a named `alloc` gets a deep copy
unless the root already records it, and a refused copy leaves `out`
ABSENT with the status. The value is consumed on every path. `out` is
overwritten, not freed.

`Tag` is `#[non_exhaustive]`: new kinds are appended, so a `match` on one
outside this crate keeps a `_` arm, which is also where `tag()`'s
`Err(UnknownTag)` belongs.

Everything else is a conversion:

```rust
Value::from(Map::new())     Value::from(List::new())
Value::from(Text::new("x")) Value::from(Buffer::new(b"x"))
Value::from("x")            Value::from(String)    Value::from(&String)
Value::from(&b"x"[..])      Value::from(Vec<u8>)   Value::from(true)
Value::from(5900i64)        // every integer width, through `Number`
Value::try_from(1.5f64)?    Value::try_from(1.5f32)?   // NaN/inf refused
```

`impl<T: Into<Number>> From<T> for Value` is the **one** blanket impl in
this crate, and it has to be the only one: a second blanket over another
`Into` overlaps with it, because a downstream type may implement both.
Every other kind gets a concrete impl.

`absent` is not `null`: absent is the answer to a lookup that found
nothing, null is a stored value.

**Nothing here stores absent in a list or a map, and nothing refuses one
that arrives.** It is a convention this crate keeps, not an invariant it
enforces: a foreign producer can put an absent node anywhere a value
goes, and every reader treats it as the ordinary kind it is.

## Ownership

**A `Value` owns its tree and frees it on drop.** Handing it to something
else is a move, which is when Rust stops dropping it.

```rust
let mut out = Value::absent();
let status = unsafe { guatiao_merge(..., &mut out, ...) };
// `out` now owns the result and frees when it goes out of scope.
```

**`value.alloc()` refuses two different things.** A scalar — null, bool,
absent — has no container to have recorded an allocator and answers
`WrongKind`. A container whose `alloc` field is null — a literal another
language wrote as a brace initialiser — answers `Alloc(AllocError::Null)`.
Growing either means naming one to adopt: the `_in` form of any
operation.

**`Map::copy_from` is not atomic.** A failure at entry *k* leaves entries
`0..k` applied. Nothing leaks, and the source is copied whole before the
target is touched.

**`Value`, `Map`, `List`, `Text`, `Buffer` and `Number` are `Clone`,
`PartialEq`, `Eq`, `Send` and `Sync`.** `clone()` (and so `to_owned()`)
is a deep copy into **this binary's allocator**, `Alloc::rust()`, whatever
the source recorded: a copy of a tree that lives in an arena, a caller's
allocator or a library's never needs that allocator. A copy beside its
source is `v.clone_in(v.alloc()?)`. `clone()` aborts on an allocation
failure, as the short forms do, and **panics on content this build
refuses** (an unknown tag, text that is not UTF-8, a number outside the
grammar, nesting past `MAX_DEPTH`). `clone_in(alloc)` is the fallible
form that names one.

**Owning a tree you already hold is `into_local()`, not `clone()`.**
`is_local()` answers whether every container in it, map keys included,
records `Alloc::rust()`; it allocates nothing for a tree up to 16
containers deep. `into_local()` hands such a tree back unchanged, the
same storage, and otherwise copies it into `Alloc::rust()` and frees the
original through the allocators it recorded (panicking where `clone`
would). `into_alloc(alloc)` is the same rule for a named allocator and
reports a refusal as `clone_in` does, the original freed either way.
`make_local_in(alloc)` is the in-place form, through `&mut`: every block
another allocator made (a container's array, a key's or a leaf's bytes)
is moved into one from `alloc` and the old one freed through its own;
elements move bitwise, content is not read (a foreign STRING that is not
UTF-8 moves as it is and its door still refuses it), and nothing is
allocated when the tree is already there. A refusal stops it with the tree
whole, every block moved or where it was. On `Value`, `Map`, `List`,
`Text` and `Buffer`. Both copying and equality are loops
over a heap stack, so they work at **any depth** and a value is always
equal to itself. `Send` and `Sync` rest on the allocator contract below:
an `Allocator` may be called from any thread.

**The standard traits, as the standard library's own types have them:**

| type | as | so |
| --- | --- | --- |
| `List` | `Vec<Value>` | `Deref`/`DerefMut<Target = [Value]>` (`len`, `get`, `iter`, `list[0]`, `sort_by`, `swap` are the slice's), `AsRef`/`AsMut<[Value]>`, owned and `&mut` `IntoIterator`, `FromIterator`, `Extend` |
| `Buffer` | `Vec<u8>` | `Deref`/`DerefMut<Target = [u8]>`, `AsRef`/`AsMut<[u8]>`, `Borrow<[u8]>`, `Hash`, `io::Write`, `From<&[u8]>`/`From<Vec<u8>>`, `Into<Vec<u8>>` |
| `Map` | `HashMap`, ordered | `Index<&str>`, owned and `&mut` `IntoIterator`, `FromIterator<(K, V)>`, `Extend` |
| `Text` | `String` | `Deref`/`DerefMut<Target = str>`, `AsRef`/`AsMut<str>`, `AsRef<[u8]>`, `Borrow<str>`, `Hash`, `Display`, `fmt::Write`, `From<&str>`/`From<String>`, `Into<String>` |
| `Str<'a>`, `Bytes<'a>` (views) | `&'a str`, `&'a [u8]` | `From<&'a str>`/`From<&'a [u8]>`, and back with `From` |
| `Number` | its text | `Deref<Target = str>`, `AsRef<str>`, `AsRef<[u8]>`, `Borrow<str>`, `Hash`, `Display`, `FromStr` |

A slice cannot change its length, so `DerefMut` leaves what a container
owns untouched. **`Map` has no `IndexMut` and no `Deref`**: `IndexMut`
could only hand back a slot that exists, so `map["new"] = v` would panic
rather than insert (`set` inserts), and a map is not a slice. `Text`'s
`DerefMut` is `String`'s: a `&mut str` stays UTF-8. `Hash` follows
equality, which is what `Borrow` needs: a `HashMap<Text, _>` is searched
with a `&str`, and `1.10` and `1.1` are two keys.

**A `Text` is always UTF-8 and a `Number` always a JSON number**, which
is why each is a `str`. Both are checked once, when they are made: a
constructor takes a `&str`, and every way out of a value (`TryAsRef`,
`TryAsMut`, `TryFrom<Value>`) checks what a foreign producer wrote. A
map's keys are `Text`, so a map is checked the same way at its own door.
A node that fails is **no `Text`, `Number` or `Map` at all**:
conversions refuse it, copying it is `ValueError::NotUtf8` or a refusal,
and it still frees on drop and equals itself by its bytes. The line this
follows: a producer's **memory shape** (pointers, lengths, capacity)
cannot be checked and is its contract to keep; its **content** (UTF-8, a
number's text, a known tag) can, and is checked rather than trusted.

**Crossing FFI**: a value handed to a foreign caller must be forgotten
(`core::mem::forget`, `ManuallyDrop`, or a move into `ptr::write`) or Drop
will free what the far side now owns. The far side frees with
`guatiao_value_free`, which runs the same walk.

The free walk is **iterative**, because a tree may have arrived from a
caller this crate cannot see and recursion on adversarial depth is an
uncatchable stack overflow on Windows. Copying and equality are
iterative for the same reason. The walks that remain recursive -- merge,
validation, the serde formats -- are bounded by `MAX_DEPTH` (128).

---

# Reading a value

Get the value, then convert it. There are no per-kind getters on a map
and no per-kind readers on a value.

```rust
let host: &str = map.required("host")?.try_into()?;
let port: u16  = map.required("port")?.try_into()?;
let fields: &Map = map.required("fields")?.try_into()?;
let n: i64 = fields.required("compression")?.try_into()?;
```

`Map::required` is the step from a lookup to a value, and it **names the
key**. `Map::get` stays `Option<&Value>` like std's.

`TryFrom<&Value>` exists for every scalar plus `&str`, `&[u8]`,
`&[Value]`, `&[Entry]`, `&Map` and `&List`. Integer reads **never
truncate**: a fractional or exponent spelling is refused rather than
rounded.

The kind a node holds comes out through two traits, `AsRef`/`AsMut` with
the refusal this model needs. The type parameter is on the **trait**, so
a binding carries it:

```rust
let m: &Map = v.try_as_ref()?;                    // or TryAsRef::<Map>::try_as_ref(&v)
let l: &mut List = v.try_as_mut()?;
// T in bool | Map | List | Text | Number | Buffer | str | [u8]
```

`bool` is one of them: `TryAsRef::<bool>` and `TryAsMut::<bool>` borrow
the arm, and `bool::try_from(&v)` copies it out with a `MapError`.

Taking the container out of the node consumes it, and the error is **the
value handed back untouched**:

```rust
let map = Map::try_from(v)?;    // Err(v) for any other kind
// also List, Text, Number, Buffer
```

A NUMBER and a STRING each have their own arm (`number`, `text`; the same
32 bytes) and do **not** share a type:
`TryAsRef::<Text>` answers `None` for a number, and `TryAsRef::<Number>`
answers `None` for a string.

Defaulting getters, mirroring the header's `static inline` helpers:
`bool_or`, `int_or`, `float_or`, `str_or`, `bytes_or`. Iteration is the
containers': `map.iter()`, `map.keys()`, `map.values()`, `list.iter()`.
Structural comparison is `PartialEq` on both the node and the container.
`Dump(&v)` is a bounded `Debug` tree for diagnostics.

**Numbers are the exact text that declared them.** `1.10` reads back as
`1.10`, a 200-digit integer survives, and `u64::MAX` crosses as itself.
Nothing coerces between kinds.

## Values as bytes: `value::wire`

An exact, self-describing binary encoding, for sending a value anywhere
bytes go. A tag byte per node, LEB128 lengths, a number as its own text,
bytes as bytes, a map in its order, the same bytes on every pointer width.

```rust
wire::encode(&Value, &mut Vec<u8>) -> Result<(), ValueError>   // a node its door refuses: refused
wire::to_bytes(&Value) -> Result<Vec<u8>, ValueError>
wire::decode(&[u8]) -> Result<Value, WireError>                // decode_in(Alloc, ..)
WireError::{Truncated, UnknownTag, Overlong, NotUtf8, NotANumber, NotABool,
            DuplicateKey, Trailing, Alloc, ..} { at: usize, .. }   // the byte it went wrong at

wire::channel::{announce(ch, &schema), send(ch, &value)} -> Result<Vec<u8>, ValueError>
wire::channel::close(ch) -> Vec<u8>
Receiver::new() / new_in(Alloc);  receiver.accept(&frame) -> Result<Received, WireError>
Received::{Schema(ch), Value { channel, value }, Closed(ch)}
// NoSchema { channel }, NotASchema { channel }, Invalid { channel, error: ValidationError }
```

Decoding is a loop, so depth costs heap, not stack; every malformation
is refused, never repaired: a length not in its shortest LEB128, a
repeated key, bytes after the value. A channel's schema arrives first and
every value on it is checked with `validate_map` before `accept` answers;
offsets in a channel's errors name bytes of the whole frame. No I/O: the
transport says where a frame ends. C: `guatiao_wire_encode(alloc, value,
out)` writes a BYTES value, `guatiao_wire_decode(alloc, bytes, out)`
answers `GUATIAO_ERR_BAD_VALUE` for bytes that are not a value.

Channels from C (`exports::channel`):

```c
guatiao_status guatiao_wire_announce(const guatiao_alloc *alloc, uint64_t channel,
                                     const guatiao_value *schema, guatiao_value *out);
guatiao_status guatiao_wire_send(const guatiao_alloc *alloc, uint64_t channel,
                                 const guatiao_value *value, guatiao_value *out);
guatiao_status guatiao_wire_close(const guatiao_alloc *alloc, uint64_t channel,
                                  guatiao_value *out);       // each: the frame, a BYTES value
guatiao_wire_receiver *guatiao_wire_receiver_new(const guatiao_alloc *alloc); // null: bad alloc
guatiao_status guatiao_wire_receiver_accept(guatiao_wire_receiver *recv, guatiao_bytes frame,
                                            guatiao_wire_event *out_event);
void guatiao_wire_receiver_free(guatiao_wire_receiver *recv);
typedef struct guatiao_wire_event {
  uint64_t channel; uint32_t kind;      /* GUATIAO_FRAME_SCHEMA 0, _VALUE 1, _CLOSE 2 */
  uint32_t refusal;                     /* GUATIAO_WIRE_ACCEPTED 0, _MALFORMED 1, _NO_SCHEMA 2,
                                           _NOT_A_SCHEMA 3, _INVALID 4 */
  size_t at;                            /* MALFORMED: the byte of the frame */
  guatiao_value value;                  /* the schema's copy or the checked value */
  guatiao_value message;                /* a refusal's words, a STRING */
  guatiao_value detail;                 /* INVALID: guatiao_schema_validate's map */
} guatiao_wire_event;                   /* 64-bit: offsets 0, 8, 12, 16, 24, 64, 104; size 144 */
```

A refusal's status: MALFORMED and INVALID `GUATIAO_ERR_BAD_VALUE`,
NO_SCHEMA `GUATIAO_ERR_NOT_FOUND`, NOT_A_SCHEMA `GUATIAO_ERR_WRONG_KIND`;
the allocator refusing is `GUATIAO_ERR_ALLOC` with `refusal` 0. A refusal
changes nothing the receiver keeps and quotes nothing it was given. The
event is written empty first and its three values are the caller's
(`guatiao_value_free` each, whatever the status); everything is built in
the receiver's allocator, which outlives it. One thread at a time per
receiver. `tests/vectors/wire.json` is the shared vector file (values,
malformed inputs, frames, and a conversation with every refusal).

**wasm32**: the crate builds for `wasm32-unknown-unknown` (the layout is
written in pointer widths); `load` there is a compile error. The module
exports the value, schema, merge and wire C surface, channels included; a
JavaScript caller builds with `guatiao_alloc_default()`.

## Streaming, with no tree: `value::visit`

A value as events: absent, null, bool, a number as its text, a string,
bytes, a list, a map of keyed values. A type that **walks** hands them to
a visitor; a type **built from a walk** pulls them from a walker. The
wire codec, a tree, and a format bridge (`guatiao-serde`) are each one
implementation, so a struct goes to bytes or text and back with no
`Value` built. Shaped like serde's traits, so bridging serde needs no
buffer either way.

```rust
// writing
trait Walk { fn walk<V: Visitor>(&self, v: V) -> Result<V::Ok, V::Error>; }
trait Visitor: Sized {                       // one method per value, consuming
    type Ok; type Error: From<ValueError>;
    type List: ListVisitor<Ok = Self::Ok, Error = Self::Error>;
    type Map: MapVisitor<Ok = Self::Ok, Error = Self::Error>;
    fn absent(self); fn null(self); fn bool(self, bool); fn number(self, &str);
    fn i64(self, i64); fn u64(self, u64); fn f64(self, f64);   // default: Display text
    fn string(self, &str); fn bytes(self, &[u8]);
    fn list(self, len: Option<usize>) -> Result<Self::List, _>;  // Some(n): exactly n follow
    fn map(self, len: Option<usize>) -> Result<Self::Map, _>;
}
trait ListVisitor { fn item<W: Walk + ?Sized>(&mut self, &W); fn end(self) -> Result<Ok, _>; }
trait MapVisitor  { fn entry<W: Walk + ?Sized>(&mut self, key: &str, &W); fn end(self); }
trait Sink {                                 // the same events, &mut self, own stack: what C's table is
    type Error: From<ValueError>;
    fn absent/null/bool/number/string/bytes(&mut self, ..);
    fn list_begin(&mut self, Option<usize>); fn list_end(&mut self);
    fn map_begin(&mut self, Option<usize>); fn key(&mut self, &str); fn map_end(&mut self);
}
Flat(&mut sink)                              // a Sink as a Visitor: value.walk(Flat(&mut sink))
TreeSink::new() / new_in(Alloc); sink.into_value() -> Result<Value, ValueError>

// reading
trait FromWalk: Sized { fn from_walk<W: Walker>(w: W) -> Result<Self, MapError>; }
trait Walker: Sized { fn build<B: Build>(self, b: B) -> Result<B::Out, MapError>; }
trait Build: Sized {                         // every method defaults to refusing with kinds()
    type Out; fn kinds(&self) -> Kinds;
    fn absent/null/bool/number/i64/u64/f64/string/bytes(self, ..) -> Result<Out, MapError>;
    fn list<L: ListWalker>(self, L); fn map<M: MapWalker>(self, M);
}
trait ListWalker { fn len_hint(&self) -> Option<usize>;
                   fn next_seed<S: Seed>(&mut self, S) -> Result<Option<S::Out>, MapError>;
                   fn next<T: FromWalk>(&mut self) -> Result<Option<T>, MapError>; }
trait MapWalker  { fn len_hint; fn next_key<R>(&mut self, impl FnOnce(&str) -> R) -> Result<Option<R>, _>;
                   fn value_seed<S: Seed>(&mut self, S); fn value<T: FromWalk>(&mut self); fn skip_value(&mut self); }
trait Seed: Sized { type Out; fn read<W: Walker>(self, W) -> Result<Self::Out, MapError>; }  // PhantomData<T: FromWalk> is one
Replay::{Absent, Null, Bool, Number, I64, U64, F64, String, Bytes, List(L), Map(M)}  // one event handed on; NoWalker fills L/M
impl ListWalker for &mut L; impl MapWalker for &mut M      // hand one on, keep reading after
Number::is_number(&str) -> bool                            // the grammar, nothing allocated

// the wire codec on them (value::wire)
wire::encode_of(&impl Walk, &mut Vec<u8>) -> Result<(), ValueError>   // = encode(&to_value())'s bytes
wire::to_bytes_of(&impl Walk) -> Result<Vec<u8>, ValueError>
wire::decode_as::<T: FromWalk>(&[u8]) -> Result<T, WireError>         // Refused { error: MapError }
Encoder::new(&mut Vec<u8>): Sink;  Decoder::new(&[u8]): &mut Decoder is a Walker
Encoded(&[u8]): Walk                                                 // bytes walked as read, checked
```

`Walk` and `FromWalk` (re-exported at the root) are implemented for what
`ToValue`/`FromValue` cover, writing the same tree's events and refusing
with the same `MapError`; `Value`, `Map`, `List`, `Text`, `Number`,
`Buffer` walk themselves, `&Value` is a walker. A walk follows at most
`MAX_DEPTH` containers (`ValueError::TooDeep`; `WireError::TooDeep` from
bytes), where the tree codec has no limit. A walker whose input fails
keeps its error and answers `MapError::Stopped`; `decode_as` reports the
bytes' error. The encoder refuses a count that differs from the promised
one and events out of order (`WrongKind`), a repeated key
(`DuplicateKey`), a number outside the grammar; with no promised count it
reserves a byte and moves what follows when the count needs more. The
decoder passes over what a builder leaves unread. `Visitor::f64` means
the number's `Display` text (`2.0` writes `2`, as `ToValue` stores it);
`Build::f64` defaults to the shortest text with a fraction (`2.0`), as a
value read from a format holds it.

## Errors

```rust
ValueError = Alloc(AllocError) | NotANumber | NotUtf8 | WrongKind
           | UnknownTag(u32) | OutOfRange | TooDeep | DuplicateKey
           | Unrepresentable                 // no spelling in its text form
           | Unparsed { at: usize, expected: &'static str }  // a grammar refused text
MapError   = MissingKey | WrongType | BadValue     // carries the path
           | Stopped                               // a walker's own input failed; it keeps why
MapError::wrong_kind(expected: Kinds, found: Tag)  // WrongType for one kind, else BadValue
```

`MapError` builds its path on the way out: `.under(prefix)` and
`.at(index)`.

**`FromValue::KINDS: Kinds`** says which kinds `from_value` may accept;
every impl here and every derived one states it, and an enum told apart
by kind (`#[map(untagged)]`, below) checks its variants with it. The
default, `Kinds::ANY`, claims every kind, so a hand-written reader that
leaves it alone cannot share an untagged enum with another variant.

```rust
Kinds::{NONE, NULL, BOOL, NUMBER, STRING, BYTES, LIST, MAP, ANY}
Kinds::of(Tag) -> Kinds                 // every method is const
kinds.union(other) -> Kinds             kinds.contains(tag) -> bool
kinds.intersects(other) -> bool         kinds.is_empty() -> bool
// Display: "a string or a map"; Debug: Kinds(a string | a map)
```

---

# Derives (feature `derive`)

```rust
#[derive(ToValue, FromValue, Schema, PartialEq, Debug)]
struct Connection {
    /// Where to connect.          // doc comment becomes the description
    host: String,
    port: u16,
    #[map(rename = "view-only")]
    view_only: bool,
    #[schema(sensitive)]
    password: Option<String>,
}
```

Building names no allocator here either:

```rust
value.to_value() -> Result<Value, ValueError>          // the crate's allocator
value.to_value_in(alloc) -> Result<Value, ValueError>  // what an impl writes
Connection::schema() -> Result<Value, ValueError>
Connection::schema_in(alloc) -> Result<Value, ValueError>
```

A hand-written `ToValue` implements `to_value_in`; a hand-written
`Schema` implements `kind(alloc)`, and both short forms are provided.

`Value`, `Map`, `List`, `Text`, `Number` and `Buffer` implement all
three: written as a deep copy into the target allocator, read as a copy
in `Alloc::rust()` (as `clone` makes one; content this build refuses is
`MapError::BadValue`). Every field type and the value and schema it
crosses as is in the table under "Field types" below.

`#[map(rename = "...")]`, `#[map(skip)]`, `#[map(default)]` or
`#[map(default = <expr>)]` (an absent key reads as it, the field is not
required, and the schema carries it); `#[schema(title, description,
sensitive, default, read_only, deprecated, examples(..))]` on the field and
`#[schema(min_length, max_length, pattern, format, multiple_of, min_items,
max_items)]` on its kind -- named after the keys they write. The derive also
accepts attributes for presentation keys **it does not own**, which are
listed in `guatiao-derive`'s own `AGENTS.md` beside the crate that names
them; they reach the document through `Extras` like any annotation.
Every struct shape is accepted, as serde writes it: a newtype (or
`#[map(transparent)]`) is its field's value, a tuple struct a list by
position (`prefixItems`), a unit struct a null; a generic type's
parameters are each bound by the trait derived. `#[map(flatten)]`,
`#[map(with = path)]`, `#[map(skip_serializing_if = path)]` and
`#[map(deny_unknown_fields)]` mean what serde's do; `#[map(as_text)]`
writes a field with `Display`, reads it with `FromStr` and describes it
as a string, for a type with no value traits of its own. The full list,
and what is refused (`alias`, a union), is in `guatiao-derive`'s
`AGENTS.md`.

Enums, in three shapes:

```rust
#[derive(ToValue, FromValue, Schema)]
enum Level { Off, #[map(rename = "warn")] Warning, On }   // "Off" | "warn" | "On"

#[derive(ToValue, FromValue, Schema)]
#[map(tag = "auth")]
enum Auth { Ambient, UserPass { username: String } }       // {"auth": "UserPass", "username": ...}

#[derive(ToValue, FromValue, Schema)]
#[map(untagged, rename_all = "lowercase")]
enum Tls { Identity, Files { certificate: PathBuf, key: PathBuf } }  // "identity" | {"certificate": ..}
```

One variant of a unit enum may be the **catch-all**, `#[map(other)]
Other(String)`: a name no variant declares is kept there instead of
refused, and written back as that text, so a vocabulary can grow on one
side of a boundary without failing the other. Its schema is then
`KindBuilder::open_enumeration`: `type: "string"` with the known names
as `examples` and their labels in `x-enum-labels`, read back with
`FieldRef::suggestions()` (`Kind::Str`, since any text validates).

A unit enum is a string and describes itself as a choice; a tagged enum is
a map and describes itself as a variant. An enum whose variants carry
fields and names **no** tag is refused: the key that tells variants apart
is a wire-format decision, and it is yours. A doc comment on a choice is
its label; on an arm it is the description, as on a field.

**`#[map(untagged)]`** is the one way out of naming a tag, for variants
that each write a different **kind** of value: the unit variants are
strings of their names, named fields a map (at most one such variant),
`Name(T)` whatever `T` reads. No name is stored; a reader takes the
variant whose kind the value has, as any language can, and refuses a
kind none reads (`MapError::BadValue`, naming the kinds). The schema is a
union (`anyOf`) of the arms: the unit variants one string `enum`, the
record an object, `T`'s kind. Two variants reading one kind are a
compile error naming both, raised by the `FromValue` derive: from a
`const` that `cargo check` evaluates, and for a generic enum when an
instance whose arms overlap is built.

The attribute follows the key: an arm is a subschema, so it takes
`#[schema(title = "..", description = "..")]`; a choice is one entry in
`x-enum-labels`, so it takes `#[schema(label = "..")]` and nothing else.

Generated readers go through `Map::required(key)` and
`MapError::under(prefix)` / `at_key(key)`; they are public so a
hand-written impl reports a missing key, and where in the tree an error
happened, the same way.

An `Option<T>` field is omitted when `None` rather than written as null,
and both spellings read back as `None`. Its schema is `T`'s accepting
null as well (`type: [T, "null"]`, or `anyOf: [T, {"type": "null"}]` for
an enum or a variant), so validation accepts a null wherever `FromValue`
reads one: the field, a `Vec<Option<T>>` element, a map's value. An
integer width writes its bounds as exact digits (`u64::MAX`, `i128::MIN`)
and `f32` the exclusive bounds past which no finite `f32` exists, so the
schema refuses exactly what `FromValue` refuses at the edges.

Generated code names only `::guatiao::` paths, checked by a test.

**The streaming half.** `#[derive(ToValue)]` also implements `Walk`,
`#[derive(FromValue)]` also `FromWalk` (`value::visit`): the same value,
written and read with no tree, so `wire::to_bytes_of` / `decode_as` and
a format bridge take a derived type straight to bytes or text. A struct
with named fields, a tuple struct, a newtype, a transparent and a unit
struct stream field by field, with every `#[map]` attribute meaning what
it means in the tree impls; a field type with only `ToValue`/`FromValue`
goes through its own small tree (chosen at compile time); a `with`
field through the module's functions over a tree of that field; an enum
and a struct with a `#[map(flatten)]` member through the whole type's
tree. A read refuses with the tree reader's `MapError`, but reports the
first refusal in the order the source holds the keys, where `FromValue`
reads in declaration order. A type parameter of the streaming impls is
bounded by `Walk`/`FromWalk`. A hand-written `Walk` or `FromWalk` beside
the derive is a conflicting impl: write the tree impls by hand too.

## Field types

Every type below implements `ToValue`, `FromValue` and `Schema`, so a
derived struct may hold it; `T` means any field type. The forms are the
language-neutral contract at
<https://boriken-dev.github.io/guatiao/encodings/>, which states each
one's grammar exactly; these types are one implementation of them.

| Rust type | value | schema |
| --- | --- | --- |
| `bool`, integers, `f32`, `f64`, `String`, `Bytes` | boolean, number (exact digits), string, bytes | `boolean`, `integer` with the width's bounds, `number`, `string`, `bytes` |
| `Value` | itself, deep-copied | `{}` |
| `Map`, `List` | itself | `{type: object, additionalProperties: {}}`, `{type: array, items: {}}` |
| `Text`, `Number`, `Buffer` | itself | `string`, `number`, `bytes` |
| `Option<T>` | `T`, or omitted / null | `T` accepting null |
| `Box<T>`, `Arc<T>`, `Rc<T>`, `Cow<'_, T>`, `&T` (write only) | as `T` | as `T` |
| `Box<str>`, `Arc<str>`, `Rc<str>`, `Box<[T]>`, `Arc<[T]>`, `Rc<[T]>` | string, list | as `String`, `Vec<T>` |
| `char` | text of one code point | `string`, `minLength` = `maxLength` = 1 |
| `Vec<T>`, `VecDeque<T>`, `[T]` | list | `{type: array, items: T}` |
| `BTreeSet<T>`, `HashSet<T, S>` | list; a repeated element refused on read | + `uniqueItems: true` |
| `[T; N]` | list of exactly N | + `minItems` = `maxItems` = N |
| tuples of 1 to 12 | list by position | `prefixItems`, `items: false`, `minItems` = `maxItems` |
| `()` | null | `{type: null}` |
| `BTreeMap<K, V>`, `HashMap<K, V, S>`, `K: MapKey` | object keyed by `K`'s text | `{type: object, additionalProperties: V, propertyNames: K's}` |
| `PathBuf`, `Path`, `Box`/`Arc`/`Rc<Path>` | its text; not UTF-8 is `ValueError::NotUtf8` on write | `string` |
| `IpAddr`, `Ipv4Addr`, `Ipv6Addr` | `Display` text | `string` + `format: ipv4` / `ipv6` (`anyOf` both for `IpAddr`) |
| `SocketAddr`, `SocketAddrV4`, `SocketAddrV6` | `Display` text | `string` + `pattern` |
| `NonZeroU*` / `NonZeroI*` | integer, zero refused | `integer`, `minimum: 1` / the width's bounds + `not: {const: 0}` |
| `Date` | `2026-10-21` | `string` + `format: date` |
| `Time` | `13:45:00.5` | `string` + `pattern` (`format: time` needs an offset) |
| `DateTime` | `2026-10-21T13:45:00.5`, no offset | `string` + `pattern` |
| `Timestamp`, `SystemTime` | RFC 3339 UTC, `2026-10-21T13:45:00.5Z` | `string` + `format: date-time` |
| `Duration` | `[secs, nanos]` | the tuple `[u64, 0..=999999999]` |
| `Span` | ISO 8601 duration, `P1DT2H` | `string` + `format: duration` |
| `Secret<T>` | as `T` | `T`'s + `writeOnly: true`, `x-sensitive: true` |

`format` is never checked; `pattern` only with the `regex` feature.

**`HashMap` and `HashSet` write in their iteration order**, which is not
reproducible from one run to the next; `BTreeMap` and `BTreeSet` write
sorted.

**`SystemTime`** crosses as a `Timestamp` (below): outside the years
0000 to 9999 it is `ValueError::Unrepresentable`, and a read keeps the
fraction to the clock's resolution, 100 ns on Windows, whose
`SystemTime` starts in 1601.

**`Duration`** reads three spellings: `[secs, nanos]`, a number of
seconds read exactly from its text (`1.5`, `2e-3`; not negative, no
finer than a nanosecond), and the `{"secs", "nanos"}` map serde writes.
The schema declares only the first. There is no C struct for it: C reads
the list, or all three spellings with `guatiao_duration_read`.

```rust
pub trait MapKey: Sized {                    // guatiao::value::convert::MapKey
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError>;
    fn from_key(key: &str) -> Result<Self, MapError>;
    fn key_kind(alloc: Alloc) -> Option<KindBuilder> { None }  // propertyNames
}
// String, Box/Arc/Rc<str> (any text); integers (decimal, no leading zero,
// no sign when unsigned); char; bool ("true"/"false"); the six addresses;
// PathBuf; Date, Time, DateTime, Timestamp. Two keys reading as one key
// are refused.
```

### Dates and times (`guatiao::{Date, Time, DateTime, Timestamp}`)

No time zone anywhere. The proleptic Gregorian calendar, no leap
seconds, the years 0000 to 9999. Each is `Copy`, `Eq`, `Ord` (by time),
`Hash`, `Send`, `Sync`; `Display` writes the canonical text, `Debug`
shows it (`Date(2026-10-21)`), `FromStr`/`TryFrom<&str>` read it and
`String::from(x)` takes it. A `MapKey`.

```rust
Date::try_new(year: i32, month: u8, day: u8) -> Result<Date, ValueError>
date.year() -> i32; date.month() -> u8; date.day() -> u8
date.checked_add(span: Span) -> Option<Date>
Time::try_new(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Result<Time, ValueError>
Time::midnight() -> Time
time.hour() / minute() / second() -> u8; time.nanosecond() -> u32
DateTime::new(date: Date, time: Time) -> DateTime
at.date() -> Date; at.time() -> Time
at.checked_add(span: Span) -> Option<DateTime>
Timestamp::from_unix(seconds: i64, nanoseconds: u32) -> Result<Timestamp, ValueError>
t.seconds() -> i64; t.nanoseconds() -> u32      // nanoseconds counted forward
Timestamp::try_from(SystemTime) / SystemTime::try_from(Timestamp)
```

- **Text**: `Date` `YYYY-MM-DD`; `Time` `HH:MM:SS[.f]`; `DateTime` the
  date, `T`, the time; `Timestamp` its UTC date-time and `Z`. A fraction
  is written with trailing zeros removed and read as 1 to 9 digits. `T`
  or `t` is read; a space is not. `Time` and `DateTime` refuse an
  offset; `Timestamp` reads `Z`, `z`, `+HH:MM`, `-HH:MM` (`-00:00` is
  UTC) and does not keep it. Seconds run 00 to 59.
- **A refusal is `ValueError::Unparsed { at, expected }`**, never the
  text; an offset that moves an instant outside the years is refused at
  the offset. A constructor's refusal (`Date::try_new(2026, 2, 30)`) is
  `ValueError::Unrepresentable`.
- **`checked_add` applies a span**: years and months first as one count
  of months, a day past the month's end clamped to its last day
  (`2026-01-31` + `P1M` = `2026-02-28`), then weeks and days, then, on a
  `DateTime` only, hours to nanoseconds carried across midnight (a day is
  24 hours). `None` for a step outside the years, and on a `Date` for a
  span with any time unit. A negative span runs each step backwards.
- **`SystemTime::try_from(t)`** is `Unrepresentable` where the platform
  clock cannot hold `t` (before 1601 on Windows) and truncates to its
  resolution.

### `Span` (`guatiao::Span`)

A length of time in calendar units, equal unit by unit (`P1D` is not
`PT24H`), with one sign for the whole span. `Copy`, `Eq`, `Hash`,
`Default` (zero), `Send`, `Sync`; no ordering. `Date::checked_add` and
`DateTime::checked_add` apply one.

```rust
Span::new() -> Span                                   // PT0S
span.years(n: i64) .months(n) .weeks(n) .days(n) .hours(n) .minutes(n)
    .seconds(n) .nanoseconds(n) -> Span               // #[must_use] builders
span.get_years() .. .get_nanoseconds() -> i64         // carrying the sign
span.is_zero() / is_negative() -> bool
span.abs() / negate() -> Span;  -span                 // Neg
span.to_string()                                       // canonical ISO 8601
"P1Y2M3W4DT5H6M7.5S".parse::<Span>() -> Result<Span, ValueError>
"1y 2mo 3w 4d 5h 6m 7s 500ms".parse::<Span>()         // the friendly form
Span::try_from(&str), String::from(span)
Span::from(Duration)                                  // PT1H1M1.5S for 3661.5 s
Duration::try_from(span) -> Result<Duration, ValueError>
```

- **A non-zero builder argument sets the whole span's sign**, as jiff's
  do: `Span::new().days(2).hours(-3)` is `-P2DT3H`. `i64::MIN` is read
  as `-i64::MAX`. `nanoseconds(n)` carries whole seconds into the
  seconds, since the text has no unit below one.
- **ISO 8601 read**: optional `+`/`-`, `P`, then `nY nM nW nD` in that
  order, then `T` and `nH nM nS` in that order, at least one unit, only
  the seconds with a fraction (1 to 9 digits), designators in either
  case. **Friendly read**: optional `-`, then `n` and a unit (`y`, `mo`,
  `w`, `d`, `h`, `m` or `min`, `s`, `ms`, `us` or `µs`, `ns`), each unit
  once, any order, spaces optional, lowercase, a fraction on `s` only.
  Each number at most `i64::MAX`.
- **A refusal is `ValueError::Unparsed { at, expected }`**: the byte
  offset and what was expected there, never the text.
- **`Duration::try_from(span)`** counts a week as 7 days and a day as 24
  hours; years, months, a negative span or an overflow are
  `ValueError::Unrepresentable`.

---

# Schema

**A schema is an ordinary value, and that value IS a JSON Schema
(2020-12)**, so a consumer in any language reads one by walking a map --
and the keys it walks are ones its ecosystem probably already has a
library for. `schema::vocab` is the contract.

There is no serialisation here and no emitter: the value already IS the
document, so `guatiao-serde` writes it out in JSON, TOML or YAML knowing
nothing about schemas.

```text
{ "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": ..., "description": ...,
  "properties": { "<name>": <schema>, ... },
  "required": ["<name>", ...] }
```

Keys we added are `x-` prefixed, which is the space the specification
reserves for exactly that, and there are **three**: `x-sensitive`,
`x-enum-labels`, `x-variant-tag`. Everything else in a document is either
JSON Schema's or an annotation somebody else named — carried, never
interpreted here, and not listed here either. A crate that writes one
documents it in its own header.

**Two things are not JSON Schema's, on purpose.** `type: "bytes"` extends
the type set, so a document using it is readable by anything and fails a
strict meta-schema check -- the data is unaffected. And `x-variant-tag`
names a variant's discriminant, because JSON Schema has no discriminator
keyword and inferring one stops working the moment two properties are
`const`.

**An undeclared key is refused, and the document says so.** Every object
the builders seal — a struct's document and each arm's subschema — carries
`additionalProperties: false`, so a general JSON Schema validator refuses
exactly what `validate_map` refuses. Refusing is deliberate: silently
dropping a misspelled field is how somebody ends up convinced a setting
does nothing. A schema read from elsewhere that does not say is read as
JSON Schema reads it: an undeclared key is accepted, and one an
`additionalProperties` schema describes answers to that schema.

`Schema::schema()` carries **every key of the finished kind** onto the
root document, not only `properties`/`required`: a tagged enum's
`schema()` is its `x-variant-tag` and its `oneOf`. `SchemaBuilder::finish`
refuses a field key declared twice, or one that repeats a variant's
discriminant, and `KindBuilder::variant` two arms stored as one
discriminant (`ValueError::DuplicateKey`); the four builders are
`#[must_use]`.

Build:

```rust
use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};

SchemaBuilder::new()
    .title("Connection")                      // JSON Schema's own keyword
    .field(FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
        .title("Port").description("...")
        .required())
    .finish() -> Result<Value, ValueError>
```

**A name lives in one place, and so does requiredness.** A field's name
is its key in `properties` and appears nowhere inside the field; whether
it is required is a name in the owner's `required` list and appears
nowhere inside the field either. That is why a builder **collects** its
fields and writes both at `finish` rather than appending as it goes, and
why a `FieldRef` carries its name and its requiredness alongside the
schema it views -- a bare pointer to a field's subschema cannot say what
it is called.

The same reason `guatiao-intake`'s C flat exports take `(schema, key)`
rather than a field pointer.

**A schema does not know about forms**, and this header does not name
their vocabulary — not even to say where it lives. How to group, order
or disclose a control is an opinion held by whatever draws it; a crate
holding one writes its keys through `Extras` and documents them itself.
Naming them here would teach this crate a word it never acts on, and
would go stale the day that crate renames one.

What every builder writes is JSON Schema's own, named after the keyword:

| method | key | on |
| --- | --- | --- |
| `title` | `title` | `SchemaBuilder`, `FieldBuilder`, `ArmBuilder` |
| `description` | `description` | `SchemaBuilder`, `FieldBuilder`, `ArmBuilder` |
| `sensitive` | `x-sensitive` **and** `writeOnly` | `FieldBuilder` |
| `read_only` | `readOnly` | `FieldBuilder` |
| `deprecated` | `deprecated` | `FieldBuilder` |
| `examples(values)` | `examples` | `FieldBuilder` |
| `option(key, value)` | any | `SchemaBuilder`, `FieldBuilder` |
| `extra(key, Result<Value>)` | any | all three, via `Extras` |

`sensitive` stays here although it looks like presentation: "never print
this value" is obeyed by a log, a dump and a crash report, none of which
have a screen. It also writes JSON Schema's `writeOnly`, so a tool that
knows no `x-` key still treats the field as a secret; `x-sensitive` is the
one read back, because "never log" is the stronger promise.

`readOnly`, `deprecated` and `examples` are JSON Schema's meta-data
annotations: carried, read by name, and **never a reason validation
refuses a value** -- `readOnly` is about who writes, and an example is
offered, not required.

```rust
pub trait Extras: Sized {                     // the door another crate writes through
    fn extra(self, key: &str, value: Result<Value, ValueError>) -> Self;
    fn extra_alloc(&self) -> Alloc;           // so a hint lands in the schema's own arena
}
```

`option` takes a value already built and cannot fail; `extra` takes the
`Result` that building through a named allocator produces, and the
builder keeps the first error as it does for everything else.

**Reading such a key is that crate's job too**, and an extension trait
over `FieldRef` is how it does it (`extra` is the door underneath). The
split is by vocabulary, not by direction: a crate that does not name a
key has no business answering what it means. What stays here is every
key this crate DOES act on — `is_sensitive` among them, because "never
print this" is obeyed by a log with no screen in sight.

**The import is the tell.** A key this crate acts on is an *inherent*
method, so it needs no `use` — `title`, `description`, `sensitive`. A key
it merely stores for somebody else arrives as an *extension trait's*
method and does not exist until that trait is in scope — `section`,
`order`, `advanced`, all `guatiao-intake`'s. So "this method needs an
import" reads directly as "this crate has no opinion about that key",
which is the boundary made visible rather than an inconsistency in the
builder. A caller who wants both halves without thinking about it imports
`guatiao_intake::prelude::*`.

**`#[derive(Schema)]` needs no import**: it names `title` and
`description` by rooted path and writes any other key through `Extras`
with the key spelled out, so a crate deriving a schema depends on this
one and nothing else.

Presentation is optional and substance is not: every presentation key may
be missing and the schema is still correct and still usable. Never make a
validation or type decision depend on one.

`FieldBuilder` builds an entry in a schema's `properties` **and** in a map kind's.

**Building names no allocator**, the same rule the value API has. Every
constructor has an `_in` twin that takes one — `SchemaBuilder::new_in`,
`KindBuilder::int_range_in` — and that is what a schema built into a
host's arena uses. **Use them throughout when you use them at all**: a
sub-builder left on the plain form allocates through the crate's
allocator, and the tree then holds some of both. Sound, because every
container carries the allocator that made it, but not what somebody
building into an arena meant.

Kinds, with what each writes:

| builder | document |
| --- | --- |
| `bool` | `type: "boolean"` |
| `string` | `type: "string"` |
| `int`, `int_range`, `int_bounds` | `type: "integer"` + `minimum`/`maximum` |
| `float`, `float_bounds` | `type: "number"` + `minimum`/`maximum` |
| `bytes` | `type: "bytes"` (ours) |
| `list(items)` | `type: "array"` + `items` |
| `tuple(positions)` | `type: "array"` + `prefixItems` + `items: false` + `minItems` = `maxItems` = the count |
| `any` | `{}`: any value |
| `null` | `type: "null"` |

Constraints chain onto a kind, each JSON Schema's own keyword:

| method | key | on | checked |
| --- | --- | --- | --- |
| `.min_length(n)`, `.max_length(n)` | `minLength`, `maxLength` | string | yes, in **code points** |
| `.pattern(re)` | `pattern` | string | with the `regex` feature (`schema::PATTERNS_ENFORCED` says whether this build has it); **unanchored**; `\d \D \w \W` are ECMA-262's ASCII classes |
| `.format(name)` | `format` | string | **never** — an annotation, as 2020-12 defaults |
| `.multiple_of(step)` | `multipleOf` | integer, number | yes; exact for a whole step on an integer |
| `.min_items(n)`, `.max_items(n)` | `minItems`, `maxItems` | list | yes, on the value |
| `.unique_items()` | `uniqueItems: true` | list | yes: numbers by what they denote, the rest structurally |
| `.property_names(kind)` | `propertyNames` | object | yes, each key as text against `kind` |
| `.title(text)`, `.description(text)` | `title`, `description` | any | never: what a union's arm is called |

A keyword for another type is carried and ignored, as JSON Schema says.
A count that is negative or fractional, and a step of zero or less, is
read as absent rather than guessed at. A `pattern` the `regex` crate
cannot compile (ECMA-262 lookaround, backreferences) is carried and not
enforced -- refusing every value for a schema this build cannot evaluate
would make the field unusable. Without the `regex` feature **no pattern is
checked** and `validate_*` accepts what one would refuse; test
`schema::PATTERNS_ENFORCED` rather than finding out from a value that got
through. `\s`, `\b` and `\p{..}` keep the `regex` crate's Unicode meaning
(the list is on `vocab::PATTERN`).
| `map(fields)` | `type: "object"` + `properties`/`required` |
| `map_of(values)` | `type: "object"` + `additionalProperties`: the value schema |
| `enumeration(choices)` | `type: "string"` + `enum` + `x-enum-labels` (a map, keyed by value) |
| `union(arms)` | `anyOf` — **any** arm accepting is enough, and no `type` |
| `variant(tag, arms)` | `type: "object"` + `x-variant-tag` + `oneOf`, each arm pinning the tag with `const` and requiring it |

`anyOf` for a union and `oneOf` for a variant is not cosmetic: a union
asks only whether the value is acceptable, so `oneOf` would reject a value
two arms both accept.

Read (borrowed views over the value, no copying):

```rust
SchemaRef::new(&value) -> Option<SchemaRef>
  .dialect() .title() .description() .fields() .find(key) .extra(key) .extras()
  .as_value()
FieldRef::new(key, &schema) -> Option<FieldRef>   // answers is_required() false
FieldRef: .key() .kind() .title() .description() .default() .is_sensitive()
           .is_read_only() .is_deprecated() .examples() -> &[Value]
           .is_required() .is_nullable() .extra(key) .extras()
           // a hint this crate does not name is read with .extra(key),
           // or by an extension trait the crate that named it provides
Kind::Str { min_length, max_length, pattern, format }   // all Option
Kind::Int { min, max, multiple_of }  /  Kind::Float { min, max, multiple_of }
Kind::List { items, min, max }       // min/max: minItems/maxItems
Kind::Tuple(schema)                  // prefixItems; .positions() reads each position's kind
Kind::Enum(schema) / Kind::Null / Kind::Union(anyOf) / Kind::Variant { tag, arms }
Kind: .choices() .alternatives() .arms() .items() .positions() .values() .fields() .name()
      // Map vs MapOf: a declared object vs one whose KEYS ARE DATA.
      // Told apart by having `properties`; `.values()` answers MapOf's
      // one value kind, as `.items()` answers a list's.
```

Validation enforces, beside the kinds: `prefixItems` with `items: false`,
`uniqueItems`, `propertyNames` (each key, as text) and `not` (a value the
inner schema accepts is refused), so a schema the derive writes refuses
what `FromValue` refuses.

**The shapes a serde-based generator writes read as what they mean.**
`type: [T, "null"]` and `anyOf: [X, null]` are a `T` / `X` that is
`is_nullable()`; `enum` of any type, a lone `const`, and a `oneOf` whose
arms are all `const` (the arm's `title` is the choice's label) are
`Kind::Enum`, and `choices()` lists the members that are strings or
numbers. Beside an `enum` or a `const`, a `type` admitting null makes a
field nullable only when null is a member. A lone `allOf` member -- the
`{"description": .., "allOf": [{"$ref": ..}]}` a generator writes for a
documented field -- reads as that member. A shape this build does not
read (`$ref`, a `type` list of several types, a `oneOf` of non-constants,
an `allOf` of several) and a schema that constrains nothing (`{}`, prose
alone, `true`, which is a field too) are `Kind::Unknown`, which validation
accepts and a renderer skips. `Kind::Missing` is only for a value that is
not a schema at all (neither a map nor `true`). A local
`$ref: "#/$defs/Name"` is read after `schema::inline_refs(&schema)` has
copied the document with each reference replaced by its definition; a
cycle, a remote reference or a name that is not defined stays as written.
The copy is budgeted on what it copies (nodes plus bytes of text, 32
times the document plus 256 Ki), a reference naming a reference is
followed in place rather than as a level of nesting, and under `default`,
`const`, `examples` and `enum` a `{"$ref": ..}` is data. An integer
`minimum: 1.5` or `maximum: 1e1` reads as 2 and 10 in `Kind::Int`, and
`Kind::Int` reads `None` for a bound past `i64`.

Validate:

```rust
validate_map(schema, &values)   -> Result<(), ValidationError>
validate_value(field, &value)  -> Result<(), ValidationError>
validate_text(field, text)     // one text against one field's kind
schema::validate::{bool_is_true, text_of, BOOL_WORDS}   // the text grammar
schema::redact(schema, &value, "***") -> Result<Value, ValueError>
```

**A typed value is the type its kind names.** `validate_value` requires a
boolean, a number (`Number`), a string or bytes before it applies the
bounds, so `validate_map` refuses `5900` for a string field, `"5900"` for
an integer and `"yes"` for a boolean -- exactly what `FromValue` refuses.
`enum` is enforced by value equality on every type (`1` equals `1.0`),
and a `type` beside it still applies; a union accepts what any arm
accepts, a list for an array arm included; null is accepted only by a
nullable field or a `null` arm. **Every numeric bound is compared as the
exact text that declared it** -- `minimum`, `maximum`, `exclusiveMinimum`,
`exclusiveMaximum`, on values and on texts -- so a bound past `i64` or
`f64` still holds; below a minimum of zero an integer spelled with a minus
sign is refused, `-0` included, as an unsigned type refuses it. A key an
object does not declare answers to its `additionalProperties` (above).
`validate_map` over a root that is not a declared object -- a tagged
enum's document -- checks it as that value.

**`validate_text` is the check for a store of text** (a flat map, a query
string): a boolean is one of `BOOL_WORDS` (`1 0 true false yes no` and the
empty string, which is a flag written bare -- `bool_is_true` says whether
that reads as true), a number is a JSON number (no `inf`, `+1`, leading
zero or `1e999`), and a container has no text form, so it is accepted and
`validate_value` checks it. `text_of(&value)` is a scalar's one-line
spelling, `""` for a container.

**`redact` replaces every secret by the placeholder**: a field whose
schema sets `x-sensitive` or `writeOnly`, at any depth, through maps, lists
and the selected arm of a variant, and on the schema an optional wraps or
any `allOf` member. A value a union accepts is hidden when any accepting
arm is marked, and whole when the accepting arms would redact it
differently. What the schema does not describe is hidden too (an
undeclared key, a container where a scalar is named, an unknown arm or a
discriminant that is not text, a union no arm accepts), because a secret
may be there. **Run `inline_refs` first**: everything under a `$ref` is
hidden, a scalar included. `redact_in` names the allocator.

From C, `guatiao_schema_redact(schema, value, placeholder, alloc, out)`
does the same and follows local `$ref`s itself; `GUATIAO_ERR_WRONG_KIND`
for a schema that is not a map, `GUATIAO_ERR_BAD_VALUE` for a placeholder
that is not UTF-8.

**`Secret<T>`** (`guatiao::Secret`) holds a value nobody prints. It
crosses as `T` (same value, same reader); its schema is `T`'s plus
`writeOnly: true` and `x-sensitive: true`, so `redact`, `is_sensitive`
and the form a schema implies all treat it as a secret. `Debug` and
`Display` print `[redacted]` whatever it holds.

```rust
Secret::new(value: T) -> Secret<T>;  Secret::from(value)
secret.expose() -> &T;  secret.into_inner() -> T
// Clone, Copy, Default, PartialEq, Eq, Hash where T has them
```

`==` is `T`'s own, not constant-time, and nothing is wiped on drop: a
moved value, a grown `String` and a value tree each hold copies this type
cannot reach, so it does not pretend to.

`ValidationError` is `UnknownOption { .. }` or `BadValue { .. }`. **An
error never quotes the value it refused** — a field may be sensitive —
it says what would have been accepted.

**Validation is depth-bounded** by `MAX_DEPTH`, like every other walk
here: it runs over two trees a caller supplied, and either nested past
the bound is a `BadValue` naming the key rather than a stack overflow.

**The flat projection is not here.** Naming a place inside a value
(`agent[1].name`), projecting one onto `key -> text`, and checking a
store of text are `guatiao-intake`'s: `path`, `flat` and
`validate_texts`. They are decisions about how a consumer reads, enters
and stores a value, and a schema describes the struct exactly as it is.
`validate_text` stays, because "is this text acceptable for this field"
asks about the schema alone.

One consequence: **a field key may hold a `.`, a `[` or a `]`**, and
`finish` says nothing about it: `guatiao-intake`'s path grammar reaches
such a key by quoting it (`["a.b"]`). A consumer whose own keys are bare
text and cannot quote refuses one itself, asking
`guatiao_intake::path::is_bare_name`.

---

# Layering

```rust
MergeMode::{Simple, Deep, Substitute}
  .merge(earlier, later, alloc) -> Result<Value, MergeError>
  .merge_with(earlier, later, alloc, fields, overrides)
  .merge_layers([("system", &a), ("user", &b)], alloc)
      -> Result<(Value, Provenance), MergeError>
```

- **Simple** — scalars and maps replace, lists overwrite positionally
  (two lists nested past `MAX_DEPTH` are `MergeError::Build(TooDeep)`).
- **Substitute** — recursive maps, wholesale list replacement.
- **Deep** — recursive maps, lists union.

`MergeOptions::new().with_mergelists(true)` merges map elements of a list
by position; **off by default**. `MergeOverrides` is a `path -> mode`
lookup for per-key control, and `schema::merge` reads the same thing from
a schema's own `x-merge` annotation (`merge_overrides`, `merge_options`,
`merge_with_schema`, each a `Result`: a schema nested past `MAX_DEPTH` is
`MergeError::Build(ValueError::TooDeep)`). **Nested objects are walked**: a
declaration on a field of a nested object governs its path
(`["tls", "ciphers"]`). A variant's arms are not walked — two arms may
declare different modes for one path.

**A path is a list of segments**, never joined text, so the key `tls.ca`
and the key `ca` inside `tls` are different places:
`MergeOverrides::new().with(["tls", "ciphers"], MergeMode::Deep)`. A
`MergeError::Kind` path and the C override table's `path` are the text
spelling `guatiao-intake` reads (`tls.ca`, `peers[0].name`, `["a.b"]`).

**Provenance is keyed by leaf path**, not by top-level key: after a
recursive merge `tls.ca` and `tls.verify` may come from different layers,
so `source_of(["tls"])` answers `Source::Mixed` rather than a confident
lie. A stored absent value keeps the earlier one -- a map entry in every
mode, a list element under `Simple`'s positional overwrite -- and claims
nothing, and `Simple` mode
attributes a replaced key (with its whole old subtree) to the layer that
replaced it.

Merging **builds a new tree** and borrows its inputs. A value whose tag
this build does not know stops the merge rather than being copied, because
copying it would make two owners of whatever it points at.

---

# Library envelope

A library exports **exactly one symbol**. Everything else it offers is
reached through the descriptor it returns.

```rust
fn describe(host: Host) -> Option<&'static LibraryInfo<'static>> { ... }
guatiao::guatiao_library!(describe);   // emits `guatiao_library_entry`
```

`LibraryInfo { struct_size, abi_version, id, version, providers, meta }`;
`ProviderInfo { struct_size, vtable_size, kinds, id, display_name, config,
vtable, ctx, meta, version, available, tables, create, destroy }`;
`HostInfo { struct_size,
abi_version, host_id, host_version, alloc, meta, services }`;
`HostServices { struct_size, ctx, get, list, alloc, get_pinned, list_pinned,
pin, unpin }` (the last four appended together). `ABI_VERSION` is 1.

`Providers { ptr, len, stride }` and `Kinds { ptr, len }` — the provider
array states its stride and the kind array does not, because a `Str`
declares no `struct_size` and so has no way to grow. Build them with
`Providers::new(&SLICE)` / `Kinds::new(&SLICE)` rather than by hand.

**A provider written without the derive** is assembled with
`ProviderParts::builder`, and its library with `LibraryParts`, rather than
by filling the `repr(C)` structs: every slot defaults to absent, so a slot
the envelope appends later leaves it compiling and correct, and the
builder owns the strings and the schema the descriptor points into.

```rust
static TABLE: GreeterVtable = GreeterVtable::of::<Hello>();     // a kind's table, or a C one
let ctx: *mut c_void = Box::into_raw(Box::new(Hello)).cast();    // lives for the process
let provider = ProviderParts::builder("acme_hello")   // id; then any of:
    .name("Hello")                       // .version(..)  empty = the library's
    .kinds(&["greeter"])
    .table(KindTable::new("greeter", &TABLE as *const _ as *const c_void,
                          size_of::<GreeterVtable>()))
    .ctx(ctx)                            // .config(schema) .meta(map) .available(f) .create(f) .destroy(f)
    .build();                            // safe: what it was given is the builder's contract
let library = LibraryParts::new("acme", "1.0.0", vec![provider]);   // .unloading(slot)
// keep it for the process (a OnceLock of the library's own), then:
//   loaded: guatiao_library!(describe), describe answering Some(library.info())
//   linked: register_local(name, describe), describe answering Some(library) -- the library's
//           lease owns it until retired and unused -- or a borrow (Some(&library),
//           Some(library.info())), read and copied at the registration and not kept
```

`meta` is a `MaybeNull<Map>` on all three — an open-ended map for
whatever the envelope did not think of, and the escape hatch that keeps a
vendor's own key out of the fields everyone shares. A reader that does not
know a key skips it, the rule the value model already has for a tag it
does not know. Null and an empty map mean the same thing.

**Non-null means a well-formed map**, and nothing checks it: the same
class of promise as `vtable`, whose shape only the `kind` knows. It is a
pointer rather than an inline `Map` because these descriptors are `Copy`
and every reader projects their fields with a bitwise read — an inline
owned container would make a second owner of one buffer on every read.

`MaybeNull<T>` is `#[repr(transparent)]` over `*const T`, so C sees a
plain pointer (`guatiao_map_ptr` in the header); `unsafe fn get()` is the
read, and `is_null()` needs no contract.

Host side:

```rust
let mut reg = Registry::new("my-host", "1.0");
reg.load_file(&path)?;                     // Result<Loading, LoadError>
reg.register_local("engine", crate::library)?;   // a library the host LINKS: the same absorb, keys,
                                                 // dedup and `Loaded` record, from `<engine>` instead
                                                 // of a file; `library` is what `local_providers!`
                                                 // writes, answering owned LibraryParts; any
                                                 // FnOnce(Host) -> Option<L>, L: Into<LocalLibrary<'a>>
reg.register_entry("engine", entry)?;            // the same from a C entry point (`EntryFn`);
                                                 // C hosts: guatiao_registry_register_entry
reg.providers("greeter")                   // by kind, BEST FIRST
reg.available("greeter")                   // the same, that can run here
reg.best("greeter")                        // the head of that
reg.set_priority(id, 10) / reg.priority(id)
reg.provider("acme_net_pve")               // by key: Option<&Provider>
reg.providers_of("acme_net_pve")           // every loaded version of one id (>1 only under %id@%version)
reg.all() / reg.all_ranked()               // everything, load order / best first
reg.retire(library_key)                    // Result<Retired, UnloadError>: out of the registry,
                                           // still mapped
reg.unload(library_key)?                   // safe: Busy while a handle holds it, then asks the library,
                                           // then closes it
reg.collect()                              // Vec<String>: retired libraries closed once unused
provider.kinds() -> &[String] / provider.supports(kind)   // what it serves
provider.config_schema()                   // Option<&Value>, the registry's copy
provider.vtable() -> (*const c_void, usize)
provider.view().vtable_as::<T>()           // unsafe; checks vtable_size >= size_of::<T>()
provider.view()                            // &ProviderView: getters id(), kinds(), config(), ctx(),
                                           // tables(), create(), destroy(); no public fields
provider.meta()                            // Option<&Map>, the registry's copy
loaded.meta                                // Option<Map>, the registry's copy
```

**Loading or registering a library is the trust decision.** A
descriptor's fields are its producer's contract — a loaded library, the
derives and macros, a `ProviderParts`/`LibraryParts` builder — so every
door that reads one is safe: `load_file`, `register_local`,
`register_entry`, `ProviderPartsBuilder::build` and
`ProviderInfo::{view, as_kind, offer, instantiate}`. What can be checked
is checked where the descriptor enters (null, `ABI_VERSION`, the
`struct_size` floors, strides, lengths, UTF-8, a table's size, hash and
required slots); what cannot — that a table is one for the kind it names,
that a slot expects its `ctx` — is the contract. `unsafe` stays on the
doors that take a bare pointer from anywhere (`Remote::from_raw`,
`Object::from_raw`, `Borrowed*::from_raw`, `Host::from_raw`) and on
`unload_unchecked`; `unload` itself is safe. `ProviderView`'s fields are private, so its safe `available()`
can only call a slot with the `ctx` it was read with.

**What a reader checks** where a descriptor enters, for every origin
(a loaded file, `register_local`, `register_entry`, `ProviderInfo::view`,
a host's `get`/`list`): each `struct_size` floor, `abi_version`, element
size against stride; every non-null pointer aligned for its type, outside
the low 64 KiB and canonical (x86-64: 57-bit; AArch64: 52-bit, top-byte
tag ignored); every length × stride addressable and capped — 4096
providers, 256 kind names and 256 per-kind tables per provider, strides
≤ 4096 bytes, text ≤ 64 KiB, tables ≤ 64 KiB; text UTF-8; each per-kind
table's words after its 8-byte header null or a plausible function
pointer (a per-kind table is a header followed by function pointers).
**For a library mapped from a file** the loader also places its image —
from the platform loader's own record of the mapping, read without
opening the file: the PE headers at the module's base on Windows (its span
and its `IMAGE_SCN_MEM_EXECUTE` sections), the program headers
`dl_iterate_phdr` reports on Linux, Android and the BSDs (the loadable
segments, and those mapped executable), the Mach-O header `dladdr` finds
on Apple platforms (the segments, and those mapped executable); elsewhere,
or where that record does not hold the entry symbol, from the file's own
layout placed by where its entry symbol landed — and refuses a data
pointer that starts inside the image and runs past it, and any function
pointer — `available`, `create`, `destroy`, `unload`, every per-kind
table slot — outside the image's executable ranges; a pointer outside the
image (the library's heap) is accepted. On ELF and Mach-O the executable
ranges are whole segments, so read-only data a linker maps beside the code
counts as code. An image neither source places is read without the image
checks. None of this proves a table is the kind it names or that a slot
expects its `ctx`.

**Everything a registry reports is OWNED.** `ProviderView` and
`LibraryView` copy every string (`String`) and every value (`Value`,
`Map`) out of the library's descriptor at the read, and the library's
lease copies every table (`vtable` and each per-kind table), so nothing
a host holds points into the image but the code those tables and the
slots name (`available`, `create`, `destroy`) and `ctx`, which the
lease keeps mapped. No handle keeps the descriptor itself.

### A library is a collection; a provider is a thing in it

- **A library loads once.** By canonical path first (which costs no
  `dlopen`), then by its rendered library key. A repeat is
  `Loading::Skipped(Skipped::AlreadyLoaded { from })` **naming where it
  came from** — not an error. A host's search path and the directory
  beside its executable are routinely the same place, so this is the
  common path, and refusing it silently disables whatever the first load
  had not reached.
- **A provider id is globally meaningful**, conventionally
  `{library id}_{name}`. Two libraries may offer one provider — a
  re-export, a vendored copy, a second build — and they agree on its id,
  which is what makes it detectable. **The rendered key decides**: when
  the key a provider renders is already filed and the holder has the same
  id, the newcomer is `Skipped::ProviderAlreadyLoaded { id, from }` and
  **the rest of that library goes on loading**; so `%id` keeps one build
  of a provider and `%id@%version` keeps every build. Same key, different
  id is `LoadError::Duplicate`.
- **A file can come to nothing five ways, each reported as itself:**
  `Skipped::NoEntrySymbol` (not a library), `Skipped::Filtered { by }`
  (a scan rule kept it out by what it declares, before it was mapped),
  `Skipped::DeclinedThisHost` (its entry point answered null),
  `Skipped::UnsupportedAbi { declared }` (it speaks another envelope
  version; the host's `abi_version` is checked by the library, the
  library's by the loader), and `LoadError::Malformed { path, reason }`
  (a check where the descriptor entered refused it; `reason` names the
  part and the fault, e.g. "the provider array is not aligned for what it
  points at" — see "What a reader checks" below). `Skipped` implements
  `Display` as one clause to print after the file's name
  (`{path}: already loaded from ..`).
- **A library can be retired.** `Registry::retire(library_key)` takes its
  providers out of the registry and out of the snapshot other libraries
  read, drops its `Loaded` record, and frees the key — a retired library
  is not `AlreadyLoaded` and may be loaded again. **The mapping stays**,
  so every handle already taken keeps working. Safe, because it dangles
  nothing a handle reaches. **Every handle holds a lease on its library**
  (`Remote`, `Offer`, `Instance`, `Object`, `ProviderView`): the
  registry's copies of the library's tables, which the handle's table
  pointer addresses, and the `LibraryParts` of a library the host built,
  which are freed when the library is retired and the last handle is
  dropped. `Retired { key, id, version, providers }` says what left;
  `UnloadError::NotFound { key }` is the only refusal.
- **`LocalLibrary<'a>`** is `Owned(Box<LibraryParts>)` or
  `Borrowed(&'a LibraryInfo<'a>)`, from `LibraryParts`,
  `Box<LibraryParts>`, `&'a LibraryInfo` or `&'a LibraryParts`;
  `info()`. **A borrowed descriptor is read once**: the registry copies
  its text, values and tables at the registration and keeps only its
  providers' code and `ctx`, the producer's to keep while the library is
  registered or a handle into it lives. A loaded library's descriptor is
  still `'static`, because a derived library builds it once for its
  mapping. Owned parts belong to the library's lease;
  `local_providers!` writes `pub fn library(Host) ->
  Option<LibraryParts>`, built on each call with nothing kept but each
  provider's default instance and tables. The handles
  (`Remote`, `Offer`, `Object`, `Instance`) are `'static` and `Send`,
  because each holds an `Arc` lease rather than a borrow, so a host
  stores them and holds them across `.await`.
- **Dropping a registry leaves every library mapped**, as retiring does:
  a `Remote`, `Offer`, boxed trait or `Instance` taken from it holds its
  lease and keeps working (`guatiao_registry_free` the same). A file
  passed over by its library key, or refused as `Duplicate`, after its
  entry point ran keeps its mapping as well — unless the loader answered
  it with an image the registry already holds (a hard link to a loaded
  file, on Linux), whose extra handle is closed so `unload` still unmaps.
- **A registry does not isolate an image.** The loader maps a file once
  per process, so a second registry loading it shares its statics and the
  descriptor a derived library built on its first entry call, with the
  `Host` and allocator of the registry that loaded it first; that host's
  lookups answer `GUATIAO_ERR_GONE` once the first registry drops. A host
  rebuilding its registry on a reload carries the library's state over.
- **And unloaded, when nothing uses it and the library agrees.**
  `Registry::unload(key)` is **safe**. It first takes the library's
  providers out of what the host's services answer, so nothing can pin
  one meanwhile, then refuses with `Busy { key, leases }` while any
  handle holds the library's lease (a `Remote`, `Offer`, `Instance`,
  `Object` or `ProviderView` taken from it, or an answer another library
  pinned through the services), then asks the library's `unload` slot,
  its promise that what it can account for is released, then closes the
  mapping. `UnloadError`: `Busy`; `NotSupported { key }` when it has no
  slot and so promises nothing; `Refused { key, status }` when it says no
  (`GUATIAO_ERR_BUSY` from a derived library with an instance or object
  alive that no handle holds, one handed across with `into_raw` say);
  `Linked { key }` for a library the host LINKS; `NotFound`; and
  `Close { key, reason }` when the loader could not close, in which case
  it is retired. Every refusal leaves the library registered, answered by
  the services again, and mapped.
- **A retired library closes when nothing uses it.**
  `Registry::collect() -> Vec<String>` closes every retired library whose
  lease no handle holds and whose `unload` slot agrees, and answers their
  keys, oldest first; one that refuses is asked again next time, one with
  no slot stays mapped. The registry's next `load_file`, `register_local`,
  `register_entry`, `retire` or `unload` collects as well. Dropping a
  handle never closes anything: it may drop while the library's own code
  is on the stack. A library the host links has nothing to close and is
  let go at the retire. Dropping the registry still never unmaps.
- **Whether the image really leaves the address space is the loader's
  call.** The library is out of the registry on every platform. Measured:
  Windows and Linux unmap, macOS keeps the image mapped and its statics
  alive, so a host that unloads to reclaim memory, or to pick up a new
  build of the same file, gets neither there.
- **`unsafe Registry::unload_unchecked(key)` is the host insisting**: it
  skips the lease count and a missing slot. A library that HAS a slot is
  still asked, and its refusal still stands. Its contract is that nothing
  the library handed out is used again.
- **What stays the caller's word**, under the safe `unload` too: a raw
  pointer taken on purpose and kept past the handle it came from
  (`vtable()`, `table()`, `table_for`, `ctx()`, `ObjectRaw`), and
  anything a C caller of the registry holds (see the C surface below).
- **A derived library counts what it hands out.** `#[derive(Provider)]`
  counts instances built through `create`, `#[guatiao::kind(object)]`
  counts object cells, and `providers!` writes an `unload` slot that
  answers `GUATIAO_ERR_BUSY` while either count, for the kinds its
  providers serve and the object kinds those hand back, is above zero.
  The counters are statics the derives emit into the library; this crate
  holds none.
- **A derived library builds its descriptor in the host's arena** when
  the host offers one: `providers!`/`local_providers!` take
  `host.alloc()` and fall back to their own. So the configuration schema
  a library declares outlives that library's mapping wherever the host
  passed `guatiao_registry_new` an allocator.
- **What a call returns lives in the host's allocator, built there when
  the provider can.** Before a call answering a `Value`, `Map` or `List`
  (or a type crossing as a value through `ToValue`), the proxy writes a
  **carrier** into the out slot: an empty container recording the
  allocator the answer is wanted in, which allocates nothing (for a
  `Value`, ABSENT whose map arm records it). The shim reads it
  (`kind::return_alloc`, falling back to its own allocator for a null or
  zeroed slot, an older host) and calls `<method>_in(.., alloc)`, which
  `#[guatiao::kind]` adds to the trait beside every method answering a
  tree. Its default answers `<method>(..)`. An implementation that
  overrides `<method>_in` and builds with the `_in` forms hands its tree
  over as it is; anything else is copied by the proxy into the carried
  allocator, the original freed through the allocator that made it while
  the call's lease still holds the library (and a `ProviderError`'s
  message likewise). A `String` is a Rust copy already and scalars carry
  nothing. So nothing the host keeps from a call needs the library's
  allocator once the library is gone. A C caller calling a table itself
  gets what the library wrote, and a C provider builds into the carrier
  by reading its `alloc` (`guatiao_kind_header`'s comment in
  `guatiao.h`).

```rust
fn greet(&self, name: &str) -> Result<Map, ProviderError> { self.greet_in(name, Alloc::rust()) }
fn greet_in(&self, name: &str, alloc: Alloc) -> Result<Map, ProviderError> {   // what the shim calls
    let mut map = Map::new_in(alloc);
    map.set_in("greeting", Text::new_in(alloc, name)?, alloc)?;                // no copy on arrival
    Ok(map)
}
greeter.greet_in("ana", arena)                // a host asking for the answer in an arena of its own
```
- **`LibraryInfo::unload`** is the library's say, an appended slot:
  `Option<unsafe extern "C" fn() -> Status>`, `GUATIAO_OK` to agree.
  Null, or a descriptor from before the slot, means the library does not
  support being unloaded. `providers!` always fills it; its long form's
  `unload = <fn>` is asked after the counts, for what only the library
  knows: a thread of its own, a callback registered elsewhere. A
  hand-written descriptor sets the field.
- **A library's entry point must not call back into the registry loading
  it.** `load_file` holds the registry exclusively for the whole call; a
  provider that needs a peer looks it up later, from a vtable call.
- **A provider serves many kinds.** `kinds` is a list; `providers(kind)`
  filters on `supports(kind)`. One implementation that both discovers
  hosts and opens sessions to them is one provider answering to both, not
  two registrations a host has to know are the same. An empty list is a
  provider reached by name rather than by capability.
- **A provider carries its own version**, empty meaning its library's —
  which is the common case, because a provider shipped in its own library
  moves with it.
- `LoadError` is then only `Open`, `Malformed { path, reason }`, and `Duplicate` for two
  **different** providers landing on one key, which only a host's own
  template can produce.

### A library can reach its host

The entry point receives a `Host` (`Copy + Send + Sync + 'static`), a
pointer to a block the registry leaks on first use and never frees. Keep
it in a `OnceLock` of your own.

```rust
fn describe(host: Host) -> Option<&'static LibraryInfo<'static>> { .. }
host.id() / host.version() / host.abi_version()
host.alloc() -> Option<Alloc>                // the host's, or None
host.meta() -> Option<&'static Map>
host.get(key) -> Result<Option<ProviderView>, Status>   // read under the size it declares, pinned
host.list(kind) -> Result<Vec<ProviderView>, Status>    // "" lists all; each pinned
view.id() / .ctx() / .table_for(kind)
host.offers::<dyn K>() / host.offer::<dyn K>(key) / host.mismatches::<dyn K>()
host.snapshot() -> HostInfo                   // a copy, absent fields nulled
```

- **Answers are descriptors the host built, read** under the
  `struct_size` each declares, so an older host's shorter one reads its
  newer fields as absent and nothing forms a reference past it. Text and
  values are copied into the view; its tables are the host's copies.
- **A view holds a pin**: `get`/`list` go through the host's
  `get_pinned`/`list_pinned`, the view (and every `Offer` and `Remote`
  taken from it) keeps the provider's library loaded while it lives, a
  clone pins again (`pin`), dropping unpins (`unpin`), and the host's
  `unload` of that library answers busy until then. So a host retiring or
  unloading the provider's library on another thread cannot pull the
  descriptor or the code from under a read or a call. From a host that
  does not pin, a view copies the tables and holds nothing.
- `list` returns **every** provider claiming the kind, unavailable ones
  included, in the host's order. **There is no best-pick**: ask each
  (`view.available()`) and choose.
- `Err(GUATIAO_ERR_NULL)`: the host offers no services (older host, or a
  host that passed none). `Err(GUATIAO_ERR_GONE)`: the registry was
  dropped; the block is still readable, the lookups are not.
- A lookup from `describe` sees the libraries registered before this one.
  A provider that needs a peer looks it up from a vtable call, on every
  call, never at `describe`.
- The C side: `guatiao_registry_host(reg)` hands a host the same block,
  to pass to a `guatiao_library_entry` it drives itself; a library reads
  `host->services->list(ctx, kind, out, cap, &total)` (call with `cap = 0`
  to size), `->get(ctx, key, &out)` and `->alloc(ctx)`, guarded by
  `host->struct_size >= offsetof(services) + sizeof` and
  `services->struct_size`. An answer from `get`/`list` is valid until its
  library is retired; `get_pinned`/`list_pinned` answer one valid until
  the library passes it to `unpin` (`pin` holds it again), and the host
  does not unload its library meanwhile. A library that keeps an answer
  past a call, or may race a retire, pins it.

### A kind is a trait (feature `provider`)

Declare a kind once, as a trait; the host and every library compile
against it. The table, the shims and the proxy are generated, and every
`unsafe` step in them is a call into `guatiao::library::kind`.

```rust
#[guatiao::kind]                       // name defaults to "greeter"; #[kind(name = "..")]
pub trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<Map, ProviderError>;   // required
    fn shout(&self, name: &str) -> String { .. }                  // default body = appended slot
}

#[derive(Default, Provider)]
#[provider(Greeter, Counter)]          // long form below
struct Hello;
impl Greeter for Hello { .. }
guatiao::providers!(Hello);              // id and version from Cargo; the whole library
guatiao::local_providers!(Hello);        // the same, for a library the HOST LINKS: writes `library`
                                         // (Host -> Option<LibraryParts>, owned by the registry),
                                         // no entry symbol, no declaration -- see register_local

// Consuming, from a host or from a library through its Host:
for offer in registry.offers::<dyn Greeter>() {
    offer.id(); offer.available();
    if let Some(g) = offer.default_instance() { g.greet("x")?; }   // None: built only from a config
}
registry.offer::<dyn Greeter>("acme_hello")    // Option<Result<Offer, KindMismatch>>
registry.mismatches::<dyn Greeter>()           // (&Provider, KindMismatch): tables that failed
provider.as_kind::<dyn Greeter>()              // Result<Option<Remote<dyn Greeter>>, KindMismatch>; None: built only from a config
host.offers::<dyn Greeter>() / host.offer(key) / host.mismatches()
unsafe { Remote::<dyn Greeter>::from_raw(table, size, ctx) }   // a table from anywhere else

// Building a configured provider, from the host's own type:
#[derive(ToValue, Schema)] struct Settings { prefix: String }
let config = Settings { prefix: "hey".into() }.to_value()?;
let instance = offer.instantiate(&config)?;   // checked against config_schema() first;
                                              // derefs to dyn Greeter; drop runs `destroy`
```

The whole host side, as a program that scans a search path and does the
above against a library on disk, is `examples/greeter_host` in the
repository (`cargo run -p greeter_host` after a workspace build).

- **Object kinds: a handle one caller owns.** `#[guatiao::kind(object)]`
  on a trait naming `Send` declares what a provider hands BACK (a
  session, a scan, a stream) or a host hands IN (a callback): never
  offered by a registry, driven through `&mut self`, destroyed by
  whoever holds it last. The table carries `destroy` right after its
  header; `&mut [u8]` crosses as an out-buffer (`BytesMut`, object kinds
  only). A Rust implementation becomes one with `value.into_object()`
  (the attribute appends that method; it is the safe
  `Object::from_cell(Box::new(ObjectCell::new(value)))`, whose bound
  `K::Vtable: TableFor<T, Kind = K>` makes a table built for another type
  or kind a compile error); the receiver holds
  `Object<dyn K>` — `Deref`/`DerefMut` to the trait, `destroy` on drop,
  `into_raw()`/`unsafe from_raw(ObjectRaw)` for a C caller
  (`guatiao_object { table, size, ctx }`). Any kind's method may return
  `Object<dyn K>` or take one; both need `Result<_, ProviderError>`
  (validation can fail), and **ownership crosses with the call**: the
  callee destroys an argument it was handed, even when it refuses the
  call, and the caller owns a return. **Or lend one for the call**:
  an argument `&mut dyn K` (or `&dyn K`, for a `K` with a `&self`
  method) crosses as a `const guatiao_object *` on the caller's stack,
  with no allocation and nothing owed; the callee's shim hands the method
  a `BorrowedMut<'_, dyn K>`/`Borrowed` (no `Drop`), and a C callee calls
  the slots during the call, never keeps the pointer and never calls
  `destroy` (on a table Rust lends it does nothing; lent shared, a
  `&mut self` slot answers `GUATIAO_ERR_UNSUPPORTED`). Lending a provider
  kind, or `&dyn K` of a kind with no `&self` method, is a compile error.
  A `&self` method reaches the value through a shared reference and a
  `&mut self` one through an exclusive one, so `Object<dyn K>` is `Sync`
  exactly when `K`'s trait names `Sync`, and an object of such a kind,
  owned or lent, may have its `&self` slots called from two threads at
  once — a C implementation of one allows that. A C lender fills
  `destroy` (every required slot is checked non-null) with a function
  that frees nothing; lent as `&mut dyn K`, nothing but the callee
  reaches the object until the call returns; lent as `&dyn K` of a kind
  that does not name `Sync`, its slots are never called from two threads
  at once. `Kind::OBJECT` says which a kind
  is; an object table's hash covers `object;` so it never passes for a
  provider table. A registry or host never serves one
  (`KindMismatch::ObjectKind`), and `#[provider(..)]` naming one is a
  compile error on the kind's span.

- **A kind is a C ABI, and its table renders to C.** cbindgen cannot see
  a macro-generated struct unless it expands macros, so a kind crate
  renders its own header with three settings (the pattern is
  `examples/greeter_kind/{build.rs,cbindgen.toml}`, header
  `include/greeter_kind.h`): `[parse.expand] crates = ["<kind crate>"]`
  with `RUSTC_BOOTSTRAP=1` and `CARGO_EXPAND_TARGET_DIR` set by the
  build script around the render (the stable compiler refuses
  `-Zunpretty=expanded` otherwise; the expansion re-enters cargo and
  needs its own target directory); `[export] include` naming every
  table, since a kind crate exports no function that references one;
  `parse_deps = false` plus an `[export.rename]` table mapping this
  crate's type names onto `guatiao.h`'s (`KindHeader` →
  `guatiao_kind_header`, `Str` → `guatiao_str`, …), so the tables are
  declared against the header that already defines those types. **That
  table ships** as `include/kind_rename.toml`, itself a cbindgen
  configuration: the kind crate's `build.rs` reads it from
  `DEP_GUATIAO_INCLUDE` with `cbindgen::Config::from_file` and extends its
  own `config.export.rename`, so it covers every guatiao type a table can
  name (a test pins that) without a copy to keep in step. Each
  table carries `<Trait>Vtable::FLOOR_HASH` as a **literal** (computed
  by the attribute at expansion time), which renders as
  `#define <table>_FLOOR_HASH n` — the number a C implementation writes
  into its table's `header.floor_hash`, beside `sizeof` the table as
  `struct_size`. A C library then offers its table through a
  `guatiao_provider_info` from its own `guatiao_library_entry`, exports
  `guatiao_declares`, and links nothing; the registry validates it and a
  Rust host calls it through the same proxy a Rust table is called
  through. `tests/c_consumer/greeter_in_c.c` is one, compiled and
  loaded by `tests/c_library.rs`.

```rust
#[guatiao::kind(object)]
pub trait Session: Send { fn poll(&mut self) -> Result<(), ProviderError>; fn read(&mut self, dst: &mut [u8]) -> i64; }
fn open(&self, uri: &Value, observer: Object<dyn Observer>) -> Result<Object<dyn Session>, ProviderError>;  // on a provider kind
let mut s = backend.open(&uri, MyObserver { .. }.into_object())?;   // the host; the library now owns the observer
s.poll()?; let n = s.read(&mut buf);                                 // &mut through the handle
drop(s);                                                             // the library's destroy runs, and drops the observer -- the host's destroy
```

- **What may cross.** Receiver `&self`; the trait names `Send + Sync`.
  Arguments: integers, floats, `bool`, `&str`, `&[u8]`, `&Value`,
  `Option<&Value>`, `&Map`, `Value`, `Map` and `List` by value (handed
  over: a move, nothing copied), `&mut Value`, `&mut Map` and `&mut
  List` (lent: see below), `Object<dyn K>` (handed over), `&mut dyn K`
  and `&dyn K` (lent), any other type by value through `ToValue`.
  Returns: `()`, the scalars, `Value`, `Map`, `List`, `String`, any other
  type through `FromValue`; each optionally in `Result<_, ProviderError>`.
  A method that converts must return `Result`. Refused by name: generics,
  `async`, `&mut self`/`self`, borrowed returns, `impl Trait`, closures,
  associated items, a required method after a defaulted one, and a method
  named `<m>_in` beside a method `<m>` answering a `Value`, `Map` or
  `List` (the attribute adds that one; its allocator parameter is
  `alloc`, or `alloc_` when an argument already has the name).
- **A tree lent mutably** (`&mut Map`, `&mut List`, `&mut Value`)
  crosses as `*mut Map`/`*mut List`/`*mut Value` to the caller's own
  node, for the call only: the callee reads, changes, inserts, removes
  and grows it, and never frees it or keeps the pointer. The shim refuses
  a null tree, and a map whose keys are not UTF-8, before the method
  runs. Before the call the proxy reads the allocator the tree records
  (`Alloc::rust()` for one that records none); after it, answered or
  refused, it calls `make_local_in` with it, so nothing in the caller's
  tree needs the callee's allocator. A key a foreign slot wrote that is
  not UTF-8 is removed and freed, and the call answers
  `GUATIAO_ERR_BAD_VALUE` (a panic for an infallible method), its own
  error winning when it refused. **The fast path**: build what goes in
  with the tree's allocator (`let alloc = config.alloc()?;` then
  `set_in`, `Text::new_in`, `to_value_in(alloc)`), and the walk after the
  call finds nothing to move and allocates nothing. `Option<&mut T>` is
  refused (lend `&mut Value`, which can hold null). In C the slot's
  comment says "Lent for the call, not handed over", because a lent and
  a handed-over tree are the same pointer type. A C caller lending a map
  gets back whatever the callee built; a Rust callee that follows the
  fast path builds in the map's allocator.
- **Versioning.** Slots follow declaration order; a defaulted method is an
  appended slot an older table may lack (the proxy runs the default). The
  table header `KindHeader { struct_size, floor_hash }` carries FNV-1a
  over the required signatures; a mismatch is refused, never called. The
  signatures are hashed as spelled, with `std`/`core`/`alloc` paths of
  `Vec`, `String`, `Option`, `Result`, `Box` and `guatiao` paths of
  `Value`, `Map`, `List`, `Text`, `ProviderError`, `Object`, `Status`
  reduced to the bare name; any other type by the name it is written
  with, so renaming a model type in a kind crate breaks every built
  provider (`HashMismatch`). Pin each kind's `FLOOR_HASH` in a test.
- **Only a per-kind table** (`ProviderInfo::tables`) is validated as a
  kind. A hand-written `vtable` is neither an offer nor a mismatch; it
  keeps working through `provider.table_for(kind)` / `vtable_as`.
- **`Offer<K>`** carries `id`, `display_name`, `version`, `library`,
  `key`, `priority`, `meta`, `config_schema`, `available()` (asked live),
  `builds_instances()` and `instantiate(&config)`. **It does not
  deref**: a provider built only from a configuration has a null `ctx`
  and nothing to call, so the default instance is `default_instance() ->
  Option<&K>`, `boxed() -> Option<Box<K>>`, `shared() -> Option<Arc<K>>`
  and the proxy over its `ctx` `remote() -> Option<Remote<K>>`, each
  `None` for one with `create` and a null `ctx`; `Provider::as_kind` and
  `ProviderInfo::as_kind` answer `Ok(None)` for it by the same rule. Unavailable providers ARE offered;
  the consumer chooses. `Remote<K>` is `Clone + Send + Sync + 'static`,
  **not `Copy`**: each clone holds the lease on its library, and
  `table()` is the registry's copy of the library's table, valid while
  the `Remote` lives. `Box<dyn K>: From<Remote<dyn K>>`. An `Object` a
  method returns holds the lease of the `Remote` it was called through;
  `into_raw` lets it go, the library's own object count answering from
  then on.
- **Every handle may be held across an `.await`.** `Remote`, `Offer` and
  `Instance` are `Send + Sync` and `Object` is `Send`, with no bound on
  `K`, so a future holding one stays `Send` and can be spawned. (`Object`
  is `Sync` only for a kind whose trait names `Sync`.)
- **`KindMismatch`** (`#[non_exhaustive]`): `NoTable`, `BelowFloor {
  size, floor }`, `HashMismatch { expected, found }`,
  `NullRequiredSlot(name)`, `ObjectKind`, `Misaligned` (the table's
  address is not aligned for the kind's table type), `Unreadable(reason)`
  (`ProviderInfo::{as_kind, offer}` over a descriptor the checks refuse).
  A `floor_hash` of `0` passes only through `from_raw`.
- **`InstantiateError`** (`#[non_exhaustive]`): `Invalid(ValidationError)`,
  the configuration does not fit `config_schema()` and the provider was
  never asked (a schema this build cannot read is not checked);
  `Refused(ProviderError)`, the provider's own answer, a table that does
  not fit, or a provider that builds no instances. `status()` is what C
  would see.
- **`ProviderError { status, message: Text }`** is the one error type
  both sides share: `From<Status>`, `From<ValueError>`, `message()`.
- **An infallible method is a promise, and breaking it panics the host.**
  A method with no `Result` has nowhere to put a failure, so when its
  slot answers a status other than OK the generated proxy panics — in
  the **caller's** stack, not the provider's. A provider that cannot
  keep the promise must say so in the signature; one that answers non-OK
  anyway is a **bug in that provider**, and it is heard loudly rather
  than swallowed, the same treatment a null required slot gets at
  validation. Pinned by
  `tests/kind_glue_probes.rs::an_infallible_method_whose_provider_fails_panics_in_the_host`.

  Writing `Result<_, ProviderError>` is the fix, and it is free: a
  provider that never fails just returns `Ok`. **A Rust provider cannot
  trip this by accident** — the boundary checks its arguments before its
  body runs (`&str` UTF-8, a non-null `&Value`, a `&Map`'s keys), and
  everything the proxy sends has already passed those checks. What
  reaches the panic is a C provider, or a hand-written table, answering
  non-OK for its own reasons. Measured 2026-09-25, which is why the
  macro does **not** refuse an infallible method for taking one of those
  arguments: there is no reachable failure to prevent.
- **`#[provider(..)]` long form**: `kinds(A, B)`, `id = ".."` (default
  `{package}_{type}` snake case), `name = ".."` (default the type),
  `version = ".."` (default empty: the library's), `config = C` or bare
  `config` (below),
  `new = path` (`fn() -> Self`) or `new_with_host = path`
  (`fn(Host) -> Self`; default `Default`), `available = path` (`fn(&Self)
  -> Result<(), &'static str>`). `providers!(id = .., version = ..,
  providers = [A, B], declares = [".."])` is the long form of the library
  line; `providers!(A, B; declares = ["VIEWER=1"])` the short one
  with extra declarations. The library declares every kind its providers
  serve (`ProviderDecl::KINDS`) for a scanner to read before mapping it;
  `guatiao::declares!(..)` does the same for a hand-written library.
- **The crate that calls `providers!` is a `cdylib` and nothing else.**
  The entry symbol and the declaration are `no_mangle`, and a `cdylib`
  exports every `no_mangle` symbol of the Rust libraries linked into it.
  So a crate that calls `providers!` (or `guatiao_library!`,
  `declares!`) and is an `rlib` as well makes every `cdylib` depending on
  it scan and load as a library offering those providers. Two such
  dependencies are a link error (`LNK2005`/`LNK1169` on MSVC, a multiple
  definition elsewhere), or, where the linker never pulls one
  dependency's object in (as an optimised build may), the other's entry
  symbol wins and nothing reports it. A provider
  that is also a Rust library is **two crates**: the library derives
  `Provider` and exports the type, and a plugin crate holds one line,
  `guatiao::providers!(acme::Tuner, acme::Recorder);`. The library id
  and version are then the plugin crate's; a provider's default id is
  `{package}_{type}` of the crate that **derived** it (`acme_tuner`). A
  host that links the library instead calls `local_providers!`, which
  exports nothing.
- **A kind crate can be the one dependency.** `pub use guatiao;` in the
  crate that defines the kinds, and a provider names that path:
  `#[provider(Tuner, crate = ::acme_kinds::guatiao)]`,
  `#[map(crate = ::acme_kinds::guatiao)]` on a type deriving `ToValue`,
  `FromValue` or `Schema`, `#[kind(crate = ..)]` on a trait, and
  `acme_kinds::guatiao::providers!(..)`. Without it every expansion is
  rooted at `::guatiao`, so each provider names `guatiao` itself at the
  exact version the kind crate was built against -- two copies of this
  crate are two different `Kind` traits.
- **Instances from a configuration.** `config = C` means the provider is
  **built from `C`**: `C: Schema + FromValue`, `Self: TryFrom<C, Error:
  Into<ProviderError>>`. Bare `config`, or `config = Self`, makes the
  type **its own configuration**: derive `Schema` and `FromValue` on it
  and there is no second type and no `TryFrom`. The host reads the
  schema (`offer.config_schema()`), fills it in, and calls
  `offer.instantiate(&config) -> Result<Instance<dyn K>, InstantiateError>`,
  which checks it against that schema first;
  the `Instance` derefs to the trait, its address is the `ctx` every call
  on it takes (what `&self` is in the impl), and dropping it runs the
  provider's `destroy`. Many instances per provider, each its own
  configuration. **An `Instance<dyn K>` is also a `K`**: store one as
  `let shared: Arc<dyn K> = Arc::new(instance);` or `Box<dyn K>::from(instance)`,
  beside `offer.shared()` / `offer.boxed()` for a provider that is its
  one instance, and `destroy` runs when the last owner goes.
  `instance.remote()` is `&Remote<K>`, borrowed from the instance; never
  clone it into an `Arc` (`K::shared(instance.remote().clone())`), which
  outlives the instance. With `config` and no `new`/`new_with_host` there is no
  default instance (`ctx` null) and `available` is refused. A provider
  without `config` is its one instance and `instantiate` answers
  `GUATIAO_ERR_NULL`; `offer.builds_instances()` says which. A panic in an
  instance's `Drop` stops at the derived `destroy` slot. The envelope
  slots are `ProviderInfo::create` / `destroy`; from C,
  `guatiao_registry_provider_create(reg, key, config, &instance, &err)`
  and `_destroy(reg, key, instance)`, the instance being the `ctx` for
  that provider's tables; `_create` checks `config` against the schema
  first, as `instantiate` does (`GUATIAO_ERR_BAD_VALUE`, the refusal in
  `err`).
- A provider that cannot build its config schema makes the library
  decline the host.

### A kind known only by its description

A kind an engine learns while it runs (a Lua or JavaScript provider, a
remote peer) has no trait to compile against, only its description: the
JSON `#[guatiao::kind]` writes (`<Trait>Vtable::description()`, a
crate's `<crate>.kinds.json`). Tables of it are built and called from
that description. Always built, no feature, no dependency; the one thing
Rust cannot do here -- make a C function pointer for a signature it
learns at run time, and call one -- is a `Mechanism`'s, and
`guatiao-libffi` (`Libffi`) is one.

```rust
KindDescription::from_value(&v)? / ::document(&v)?   // one kind / {"format": 1, "kinds": [..]}
kind.name() .floor_hash() .object() .slots() .slot("greet") -> Option<(usize, &SlotDescription)>
     .size() .floor() .word_of(slot)
slot.name() .required() .fallible() .args() .answer() .doc()

// Serving: every slot of the table calls one handler.
pub trait Serve: Send + Sync + 'static {
    fn call(&self, ctx: usize, kind: &KindDescription, slot: usize, args: Vec<Arg<'_>>)
        -> Result<Answer, ProviderError>;
    fn destroy(&self, ctx: usize) {}                 // an object kind's destroy
}
let table = ServedTable::new(kind, |slot| implemented, serve, &Libffi)?;  // Result<_, TableError>
table.as_ptr() .size() .kind() .kind_table()     // kind_table() -> ProviderPartsBuilder::table
table.object(ctx) -> Option<RawObject>           // an object of this kind, for an answer
// Keep the table as long as anything may call it: ProviderPartsBuilder::keep(table).

// Calling: any table of the kind.
let caller = Caller::new(kind, &Libffi)?;
unsafe { caller.call_raw(table, size, ctx, slot, args) } -> Result<Answer, ProviderError>
DescribedRemote::new(provider.view(), Arc::new(caller))? .call(slot, args) .call_named("greet", args)
                                                 // holds the provider's library loaded
raw_object.call(&caller, slot, args)  lent.call(..)  handle.call(..)

enum Arg<'a> { Bool, Int(i64), Uint(u64), Float(f64), Str(&str), Bytes(&[u8]), Buffer(&mut [u8]),
               Value(Option<&Value>), Map(&Map), List(&List), TakenValue(Value), TakenMap(Map),
               TakenList(List), LentValue(&mut Value), LentMap(&mut Map), LentList(&mut List),
               Object(RawObject), LentObject(LentObject<'a>) }      // #[non_exhaustive]
enum Answer { Nothing, Bool, Int, Uint, Float, Text(String), Value, Map, List, Object(RawObject) }
enum TableError { Unsupported(String), MissingRequired(String), Mechanism(String) }
RawObject (owned; drop calls the kind's destroy) .lend() .into_raw() .as_raw(); unsafe from_raw
LentObject<'a> .share(|handle: LentHandle| ..)   // callable from another thread while lent;
                                                 // GUATIAO_ERR_GONE after
```

Scalars arrive widened (`Int`, `Uint`, `Float`) and a `Caller` narrows
them to the slot's width, refusing one that does not fit
(`GUATIAO_ERR_BAD_VALUE`); a scalar or text answer may be an
`Answer::Value`, converted exactly. A handler's panic stops at the slot
(`GUATIAO_ERR_INTERNAL`).

**Writing a mechanism.** `unsafe trait Mechanism { fn prepare(&self,
&CSignature) -> Result<Arc<dyn Prepared>, TableError> }`, and `unsafe
trait Prepared { fn closure(self: Arc<Self>, SlotHandler) ->
Result<Closure, TableError>; unsafe fn call(&self, f, args: &[*mut
c_void]) -> u32 }`. A `CSignature` (`CSignature::of_slot(kind, slot)`,
`::destroy()`) is `args() -> &[CType]` (`ctx` first, then the slot's
arguments, `out`, `err`) and `returns_status()` (a `u32`, or nothing for
`destroy`); `CType` is the fixed-width integers, `F32`, `F64`, `Pointer`
and `Struct(fields)` by value (a `Str` is `{Pointer, Usize}`). The
contract: a closure's code is callable with exactly the signature, from
any thread, until the `Closure` (`unsafe Closure::new(code, owner)`)
drops, and each call runs `SlotHandler::call(args)` once with one address
per argument, returning its status; `call` calls `f` with the signature
and returns its status.

### Ordering is `(priority DESC, key ASC)`

`providers`, `available` and `providers_of` all answer **best first**.

**Priority is the HOST's**, kept beside the registry rather than in a
descriptor: a library does not know how a person ranks it against the
others they installed, and two machines with the same libraries can rank
them differently. A frontend reads its own configuration and calls
`set_priority`; nothing in this crate reads a file.

Absent means 0, so an unranked provider sorts below any raised one and
alongside every other unranked one; negative sorts below them all. A rank
set **before** anything loads still applies, so the order a host ranks and
scans in does not change the answer.

The **key** tiebreak is deliberate. Load order follows directory
iteration, which no filesystem promises to keep stable across runs or
machines, so "whichever loaded first" is not a rule anyone can document or
reproduce. A key is arbitrary but deterministic and inspectable, and a
host that wants a different winner says so with a priority.

`best(kind)` is the head of what can actually **run**, which is a
different question from what ranks highest: a provider that ranks first
and refuses is passed over. Iterate `available` instead to show what was
passed over — "ssh → openssh (also: putty)".

In C: `guatiao_registry_set_priority`, `_priority`, `_best`; and every
provider map carries its `priority`.

### Can it actually run here?

`ProviderInfo::available` is an optional slot: `true` for yes, or `false`
with a borrowed reason written through an out-parameter. **Null means
yes** — a library that does not implement it is available, which is the
common case and the right default.

```rust
provider.available()            // Result<(), &str>, the reason borrowed while the provider lives
reg.providers("codec")          // who CLAIMS the kind
reg.available("codec")          // who can serve it now
reg.why_not("codec")            // Option<WhyNot>: NothingClaimsIt | NoneAvailable
```

**The envelope defines the slot, not the meaning.** What "available" means
and when to ask are between a host and a library, like everything a `kind`
agrees. The one promise this crate makes is that it **never caches the
answer**: a library may load an optional dependency, lose a device, or
fail its own integrity check while a process runs.

**The reason must outlive every reader** — a literal in the image, or
something leaked once. Never a buffer shared between callers: two threads
asking the same provider would each read what the other just wrote, and
what comes back is a fragment with nothing reporting it.

`why_not` keeps two refusals apart **because the remedies differ**:
nothing claims it (install something) versus everything claiming it is
unavailable here (fix what you have), the latter carrying each provider's
own words. One "unsupported" leaves a person with no idea which way to go.

In C: `guatiao_registry_available`, `guatiao_registry_why_not`, and
`guatiao_registry_provider_available(reg, key, &reason)`.

### What things are filed under is the host's choice

```rust
let reg = Registry::new("my-host", "1.0")
    .libraries_keyed_by("%id@%version")?    // hold two builds of a library
    .keyed_by("%id@%version")?;             // and tell their providers apart
reg.provider("acme_net_pve@1.2.0")
provider.key()  /  loaded.key               // what each answers to
```

`KeyTemplate` fields: a **library** has `%id` and `%version`; a
**provider** adds `%name` (display name) and `%library`. `%%` is a literal
`%`, everything else is text. Naming a field the subject does not have is
refused where it is written, as is a `%` beginning nothing and a template
naming **no** field (which would give everything one key). Both `keyed_by`
methods re-key what is already loaded and refuse a template that would
collide there (`KeyError::Collides`).

The library key is the "how many builds may I hold" knob. With `%id` and
`scan_dir`, **which** build survives is the scan's order —
`scan_dir_ordered(reg, dir, Order::Descending)` offers the
highest-sorting name first. That is byte order, not version order:
nothing here parses a version, because ordering one is a host's policy
with the semver library it already has.

**A library declares what it is, and a scan reads that before mapping
it.** The data symbol `guatiao_declares` (`DECLARES_SYMBOL`) holds
`key=value` strings, NUL-separated, ended by an empty string: `kind=<name>`
for every kind a provider serves, written by `providers!`, plus whatever
`guatiao::declares!("kind=greeter", "VIEWER=1")` or `providers!(A,
B; declares = ["VIEWER=1"])` adds. `probe(path) -> Probe { entry,
declared: Declared }` reads it as data (clamped to its section and 4 KiB;
malformed is `NotExaminable`); `declared.has(k, v)`, `values(k)`,
`kinds()`. **The probe reads the file on demand**, never whole: headers,
export and dynamic symbol tables, the declaration. Only a DEFINED symbol
counts (a library that imports the entry symbol is not one); an export
table that cannot be read is `Err`, so `NotExaminable`, never "no entry
symbol"; a universal (fat) Mach-O is probed through the slice for the
architecture the host runs on, and is `Err` without one. A host filters with
`scan_dir_rules(reg, dir, order, &ScanRules::parse(&["!VIEWER=1",
"kind=session-backend"])?)` — `!KEY=VALUE` skips a file declaring the
pair, `KEY=VALUE` skips one that does not, a file declaring nothing
passes every `!` rule and fails every positive one — or with
`scan_dir_with(reg, dir, order, |declared| Ok(()) | Err(why))`. Either
reports the file as `Skipped::Filtered { by }` and never maps it. That is
the replacement for a filename denylist: the library says
`VIEWER=1`, the host writes one rule.

### Where a host looks is a search path

```rust
let path = SearchPath::parse(&std::env::var("MY_HOST_PLUGIN_PATH").unwrap_or_default())
    .with(exe_dir.join("plugins"));            // adjacency accumulates, never replaced
let report = scan_path(&mut reg, &path, Order::Ascending, &rules);
report.loaded / report.skipped / report.failed / report.unreadable
// loaded: Vec<(PathBuf, String)>, each file with the library key it was filed under
```

- **A list of directories or files**, split on the platform's own
  separator (`SearchPath::SEPARATOR`: `;` on Windows, `:` elsewhere),
  empty parts dropped, **each place once** — by canonical path when it
  exists, as written when it does not — so an override that names the
  directory adjacency already found is one entry.
- A directory is scanned as `scan_dir_rules` scans it. A **file** is
  probed and loaded on its own, under the same rules: naming it is
  asking for it, so its extension is not checked, but what it declares
  still is.
- **`scan_path_with(reg, &path, order, |declared| Ok(()) | Err(why))`**
  is the same walk under a filter of the host's own, for what a rule
  cannot say: a rule set is a conjunction of exact pairs, so "any kind of
  mine" is `declared.kinds().any(|k| k.starts_with("acme_"))`.
- A **bundle** is its binary: `Name.framework` resolves to `Name` inside
  it, flat (iOS) or under `Versions/Current` (macOS). `bundle_binary` is
  pure path logic and answers on every platform; a scan of a directory
  offers a bundle as a candidate too.
- An entry that **cannot be read** — missing, or not listable — is
  `LoadReport::unreadable` and the rest of the path is still walked.
  Reported rather than dropped for the same reason a skip is: a person
  looking for a plugin that did not appear needs to see the place was
  considered. A single-directory scan still answers that with its `Err`.
- A library reachable twice loads once; the second is
  `Skipped::AlreadyLoaded { from }` naming the first.

In C: `guatiao_registry_scan_path(reg, spec, descending, rules, alloc,
&answer)`, whose report gains `"unreadable": [{"path", "error"}…]` only
when there is one.

### Sources: libraries that come and go while the host runs

```rust
use guatiao::library::{Candidate, Registry, Refreshed, ScanRules, Source};

reg.add_source("shelf", MySource)?;                          // a Source the host links
reg.add_source_table("shelf_c", &table)?;                    // a guatiao_source_info the host links
let name = reg.add_source_file(Path::new("guatiao_py.dll"), &config)?;   // a file exporting guatiao_source
reg.watch_dir(Path::new("plugins"), ScanRules::default())?; // a directory kept in step with disk
let changed: Refreshed = reg.refresh();                     // when the host chooses: a timer, its own watcher
changed.loaded / changed.retired                            // Vec<Served { source, id, stamp, key }>
changed.skipped                                             // Vec<(source, id, Skipped)>
changed.refused                                             // Vec<Refused { source, id: Option<String>, reason }>
```

A **source** serves libraries that are not files a scan can probe — an
interpreter's modules, a manifest's entries — or keeps a directory in
step with disk. The registry keeps it for its life; **what it serves are
ordinary libraries**, retired and closed like any other. Nothing runs on
its own: no thread, no watcher, no global state. The host calls
`refresh()`, which re-lists every source and diffs by `(source, id,
stamp)`:

- a new candidate is opened and loaded, filed under `<source:id>`;
- a changed stamp opens the new build **first** and only once it loads
  retires the old: instances of the old keep working on their lease, new
  lookups find the new; a refused open keeps the old one serving;
- a vanished candidate is retired;
- a library the host retired itself stays out until its stamp changes;
- an open is retried only when its stamp changes; the opened library's
  id must be the candidate's, or the open is refused (and closed).

Refusals are in `Refreshed::refused`, never a failed refresh. A served
library closes through `collect`/`unload` once nothing holds it, its own
`unload` slot asked when it has one (a missing one is no refusal: the
source's `close` is the promise), and the source is then told. Dropping
the registry closes nothing.

`trait Source: Send + Sync { fn list(&self, host: Host) ->
Result<Vec<Candidate>, ProviderError>; fn open(&self, host: Host, id: &str)
-> Result<LocalLibrary<'static>, ProviderError>; fn close(&self, id: &str,
stamp: &str) {} }`. `Candidate::new(id, version, kinds, stamp)`. Calls are
one at a time, from the thread calling the registry, panics caught.

**The C table**, `guatiao_source_info` (`SourceInfo`): `struct_size`,
`abi_version`, `ctx`, then

- `list(ctx, host, guatiao_list *out)`: one map per candidate, `{"id",
  "version", "kinds": [..], "stamp"}`, written over a carrier naming the
  allocator to build in; runs no provider code;
- `open(ctx, host, guatiao_str id, const guatiao_library_info **out,
  guatiao_provider_error *err)`: the descriptor, valid until `close`, or
  a refusal with words;
- `close(ctx, id, info)`: once per descriptor `open` answered, after the
  registry closed it (null when nothing needs letting go).

A source **file** exports `guatiao_status guatiao_source(const
guatiao_host_info *host, const guatiao_value *config, const
guatiao_source_info **out, guatiao_provider_error *err)` (`SOURCE_SYMBOL`,
`SourceFn`) and declares `source=<what it serves>` in `guatiao_declares`:
a scan reports it as `Skipped::Source { serves }` and never loads it.
`add_source_file` maps it, checks the table's slots lie in its code, and
**never unmaps it**. `SourceError`: `Open`, `NotASource`, `Refused {
status, message }`, `Malformed`, `Duplicate`; `.status()` is what it
crosses C as.

A **watched directory**: each refresh loads a library file that appeared
(by `rules`, as a scan), retires one that vanished, and retires and loads
again one whose length or modification time changed. A file replaced
under the same path may load the image the platform loader still holds;
replace under a new name to be sure of the new code. Windows refuses to
delete a mapped image but moves one.

In C: `guatiao_registry_add_source(reg, name, &table, &err)`,
`guatiao_registry_add_source_file(reg, path, &config, &err)` (`err`
written empty first; `NOT_FOUND` unmappable, `WRONG_KIND` no
`guatiao_source` or a config that is not a map, `BAD_VALUE` refused table
or a name already added, else the source's own status),
`guatiao_registry_watch_dir(reg, dir, rules)`, and
`guatiao_registry_refresh(reg, &alloc, &answer)`, answering `{"loaded":
[{"source", "id", "stamp", "key"}…], "retired": […], "skipped": [{"source",
"id", "skipped", …}…], "refused": [{"source", "id"?, "why"}…]}`. A scan
report's `skipped` gains `"source"` with `"serves"`.

### Notices ride on `meta`

```rust
use guatiao::library::{Notice, declare_notice, notices};
declare_notice(&mut meta, Notice::text("openh264", "OpenH264 Video Codec provided by ..."))?;
declare_notice(&mut meta, Notice::markdown("", "# Licence\n..."))?;
for n in notices(provider.meta()) { n.component; n.text; n.is_markdown(); }
```

`meta["notices"]` is a list of `{component, text, format}` maps, one per
licence. **A convention, not a field**: a notice is text a host must be
able to show — some licences require the attribution to be displayed —
and only the library knows what it links, so it belongs on the escape
hatch every descriptor already has, under a key named once here rather
than invented per host.

- `component` says what the licence covers; empty is the whole library
  or provider, and a host shows the display name.
- `text` blank contributes nothing: refused on write, dropped on read.
  Never trimmed — leading indentation is part of a licence.
- `format` is **declared, never sniffed**: `text` (the default) or
  `markdown`. Anything else reads as text, the safe direction; a prose
  licence rendered as Markdown is a licence that was not reproduced.
- **Read on display, never cached.** A library that loads an optional
  component on demand owes its attribution only while it is in use.

**A loaded library stays mapped until the registry closes it**, through
`unload` or `collect`, and a Rust handle into it holds a lease that stops
both. A C caller holds raw pointers instead, which no lease counts: a
vtable, a `ctx` or a descriptor taken from a library is the caller's to
have let go before it asks `guatiao_registry_unload`, as the library's own
slot cannot see them; one taken from a retired library is valid until
`guatiao_registry_collect`, or the registry's next load, registration,
retire or unload, closes it. `guatiao_registry_unload` answers
`GUATIAO_ERR_BUSY` while another library holds a pinned answer from it.

### A host in C, or Python, or anything with an FFI

The whole registry is `extern "C"`, so Rust is one user and not the
audience:

```c
guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
guatiao_registry *reg = guatiao_registry_new(guatiao_cstr("my-host"),
                                             guatiao_cstr("1.0"), &alloc);
guatiao_value answer;
guatiao_registry_load_file(reg, guatiao_cstr(path), &alloc, &answer);
guatiao_registry_scan_dir(reg, dir, /*descending=*/false, &alloc, &answer);
guatiao_registry_scan_dir_rules(reg, dir, false,
                                guatiao_cstr("!VIEWER=1\nkind=session-backend"),
                                &alloc, &answer);       // rules, one per line
guatiao_registry_providers(reg, guatiao_cstr("greeter"), &alloc, &answer);
guatiao_registry_provider(reg, key, &alloc, &answer);
guatiao_registry_retire(reg, guatiao_cstr("hello_library"));   // out of the registry, still mapped
guatiao_registry_unload(reg, guatiao_cstr("hello_library"));   // BUSY while held, asks the library, then closes it
guatiao_registry_unload_unchecked(reg, key);                   // past a hold or a missing slot: the host insists
guatiao_registry_collect(reg, &alloc, &answer);                // closes retired libraries nothing uses: a list of keys
guatiao_registry_keyed_by(reg, guatiao_cstr("%id@%version"));
guatiao_registry_libraries_keyed_by(reg, tmpl);
guatiao_registry_libraries(reg, &alloc, &answer);
const void *vt = guatiao_registry_provider_table(reg, key, guatiao_cstr("greeter"), &size);  // by kind
const void *one = guatiao_registry_provider_vtable(reg, key, &size);                       // its single table
void *ctx = guatiao_registry_provider_ctx(reg, key);
const guatiao_value *schema = guatiao_registry_provider_config(reg, key);   // borrowed
guatiao_registry_provider_meta(reg, key, &alloc, &answer);      // a copy; ABSENT when it declares none
guatiao_registry_free(reg);
```

`bindings/python/guatiao/` wraps the same calls over ctypes (its own `AGENTS.md`
is the API reference):

```python
from guatiao.registry import Registry

with Registry("my-host", "1.0") as reg:
    reg.scan_dir("/path/to/plugins", rules="kind=greeter")
    reg.best("greeter")   # a plain dict, already freed
```

`guatiao_registry` is the **one opaque handle** in this crate: it owns
growable collections and changes over time, which is what a handle is for
and a `repr(C)` struct is not.

**Answers come back as values**, so there is no accessor per field and a
language that can already read a value can already read the answer. A
provider map is `{key, id, version, library, display_name, from, kinds,
has_config, vtable_size}`; a library map is `{key, id, version, path,
providers, skipped}`. Free each answer with `guatiao_value_free`.

**The exceptions are what is not data**: the vtable, its size, the `ctx`,
and the borrowed config schema have their own accessors. A provider's
`meta` has one too, so a listing does not copy it per provider:
`guatiao_registry_provider_meta` writes a copy in `alloc` (ABSENT for a
provider that declares none, `GUATIAO_ERR_NOT_FOUND` for an unknown key),
which outlives the library. A pointer inside
a map would be a number a caller has to cast back, and reading one is the
moment a caller takes on the kind's contract.

**A skip is an answer, not a failure.** `load_file` writes `{"loaded":…}`,
`{"skipped":"<why>","from":…,"abi":…}` or `{"failed":"<the loader's own
message>"}` and returns `GUATIAO_OK` for all three; a non-OK status means
it could not answer at all. `why` is one of `no-entry-symbol`,
`declined-this-host`, `unsupported-abi` (with `abi`), `already-loaded`
(with `from`), `provider-already-loaded` (with `from` and `id`), or in a
scan report `not-examinable`, `filtered` (with `by`, the rule) and
`source` (with `serves`: a source file, which a host adds rather than
loads). A rule
that is not `[!]KEY=VALUE` is `GUATIAO_ERR_BAD_VALUE`. `guatiao_registry_providers` with an empty
kind lists every provider; both listings are best first.

**These need the `load` feature** — the only part of the C surface that
does, because loading needs `libloading` and a library author takes no
dependency. Build the shipped artifact with `--features load` (or
`--all-features`), or its export table will not have them.

## Appending to a descriptor

Every descriptor leads with `struct_size`, and a reader decides field by
field what the writer covered. **A slot is appended, never changed**: a
changed meaning at an existing offset is silent memory corruption, not a
version error. `floor()` on each type is the frozen size of its v1
prefix, and each appended field has its own guard — `HostInfo::alloc_end()`,
`HostInfo::meta_end()`, and so on.

The guard is `>=` against offset-plus-size, not `>` against the offset, so
a descriptor declaring only PART of a slot reads it as absent rather than
splicing half a pointer with whatever followed.

**`struct_size` cannot version an ARRAY's element size.** It places the
fields within one element; finding element `i` needs the size the library
laid the array out at. So `Providers` carries `{ptr, len, stride}`, a
reader walks by bytes, and it refuses a stride below `ProviderInfo::floor()`
or an element whose own `struct_size` exceeds the stride (which would
overlap its neighbour). `Providers::new(&SLICE)` sets the stride for you —
a hand-set one is a number to get wrong exactly once.

Appending `meta` is what this machinery is for, and it did not move
`ABI_VERSION` or any `floor()`: a library built before it declares a
shorter `struct_size`, and the host reads `meta` as absent.

Reading a caller's struct must not start by making a reference to it — a
reference asserts the whole pointee is dereferenceable, which a caller
compiled before the last field was appended does not have.

---

# Passing data: ownership, allocators and copies

Every owned container records the allocator that made it and frees
through it wherever it ends up. So each crossing has three answers: who
owns the tree now, which allocator it is in, and whether anything was
copied. The rules that keep the copies to the necessary ones:

**1. Return trees by value; take them by value when you keep them.** A
kind method's types say who owns what:

```rust
#[guatiao::kind]
pub trait Store: Send + Sync {
    fn check(&self, config: &Map) -> Result<bool, ProviderError>;  // read: borrowed, nothing copied
    fn save(&self, config: Map) -> Result<(), ProviderError>;      // kept: handed over, a move
    fn load(&self) -> Result<Map, ProviderError>;                  // answered: owned by the caller
    fn complete(&self, config: &mut Map) -> Result<bool, ProviderError>;  // changed in place: lent
}
```

- Borrowed (`&Map`, `&Value`, `Option<&Value>`, `&str`, `&[u8]`): read in
  place for the call.
- `Map`, `Value`, `List` by value: a move. The slot gets the caller's
  node, the caller forgets it, the callee owns it on every path. The tree
  stays in the caller's allocators; a callee keeping it past them calls
  `into_local()`.
- `&mut Map`, `&mut List`, `&mut Value`: the caller's tree, lent to be
  changed in place. Build what goes in with `config.alloc()?` and the
  `_in` forms; the proxy moves anything built elsewhere into the tree's
  allocator once the call returns (`make_local_in`).
- A returned `Map`/`Value`/`List` is the caller's. A borrowed return is
  refused: nothing on the caller's side owns what it would borrow.
- A type of yours by value crosses through `ToValue`/`FromValue`: built
  once, read once. Take the tree itself when the callee would only turn
  it back into one.

**2. Build a tree where it will live.** The engine's allocator reaches a
library three ways:

- **Per call**: the proxy writes a carrier (an empty map or list, or
  ABSENT for a value, recording the allocator it wants) into the out
  slot, and the shim calls `<method>_in(.., alloc)` with it. Override it
  with the `_in` forms and the answer crosses with no copy; its default
  calls `<method>`, whose answer the proxy copies once. Implement
  `<method>` as `self.<method>_in(.., Alloc::rust())`. A host wanting an
  answer in its own arena calls `<method>_in(.., arena)` on the proxy.

```rust
fn load(&self) -> Result<Map, ProviderError> { self.load_in(Alloc::rust()) }
fn load_in(&self, alloc: Alloc) -> Result<Map, ProviderError> {
    let mut map = Map::new_in(alloc);
    map.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;  // the leaf too
    Ok(map)
}
```

  `set_in` puts the entry and key in `alloc`; a value converted from
  `&str` by `Into<Value>` is built on Rust's heap, so build leaves with
  `Text::new_in`/`Buffer::new_in`/`Number::new_in` or `to_value_in(alloc)`.
- **Outside a call**: `host.alloc() -> Option<Alloc>` in a library's
  entry point; what a library builds once and the host keeps goes there
  (`providers!` does this for the descriptor).
- **For a tree in hand**: `into_alloc(alloc)` (a move, copying only what
  is elsewhere) or `clone_in(alloc)` (a copy of a borrow).

**3. Move, do not copy.** `Value::from(container)` and
`Map::try_from(value)` (and `List`, `Text`, `Buffer`) are moves;
`remove`, `pop`, `core::mem::take` and owned `IntoIterator` take values
out; `map.extend(other)` moves `other`'s values in; `set`/`push` move a
container in. Read with `TryAsRef`, `<&Map>::try_from` and
`required(..)?.try_into()?` instead of converting to an owned copy.
`value.to_value()` on a `Value` is a deep copy, not a move.

| you have | you want | write | copies |
| --- | --- | --- | --- |
| a `Value` you own | it, local | `v.into_local()` | only containers not in `Alloc::rust()` force one |
| a `Value` you own | it, in `alloc` | `v.into_alloc(alloc)?` | only when something is not in `alloc` |
| a `&Value` | your own | `v.clone()`, `Value::from_value(v)` | always, into `Alloc::rust()` |
| a `&Value` | a copy in `alloc` | `v.clone_in(alloc)?` | always |
| a `&Value` | a copy beside it | `v.clone_in(v.alloc()?)?` | always |
| a container | a `Value` | `Value::from(c)` | never |
| a `Value` | a container | `Map::try_from(v)` | never |
| a value in a container | it, owned | `remove`, `pop`, `mem::take` | never |
| a type of yours | a tree | `t.to_value()`, `t.to_value_in(alloc)?` | builds one |
| a kind's answer | it, owned | the returned tree | never through `<method>_in`, else once at the proxy |
| a tree for a kind call | kept by the callee | pass it by value | never |
| a tree for a kind call | read by the callee | pass `&Map`/`&Value` | never |
| a tree for a kind call | changed in place by the callee | pass `&mut Map`/`&mut List`/`&mut Value` | never; parts built in another allocator moved once |
| a tree reached through `&mut` | it, in `alloc`, where it stands | `t.make_local_in(alloc)?` | only the parts not in `alloc` |

**From C**: initialise a tree out slot as a carrier (`len` and `cap` 0,
`alloc` yours; or ABSENT with `payload.map.alloc`), or zeroes; free the
answer with `guatiao_value_free` (a map or list inside a `guatiao_value`
with its tag). A non-`const` `guatiao_value *`/`guatiao_map *`/
`guatiao_list *` argument is handed over: the slot owns it and the caller
never frees it, unless the slot's comment says "Lent for the call, not
handed over" (a Rust `&mut`): then it is the caller's, to change and grow
through the allocator it records, never freed or kept. A `const` one is
lent for reading. A provider error's message is the
caller's to free through its `alloc` when its `cap` is not 0.

---

# The C surface

**The `extern "C"` surface is always compiled.** It is not behind a
feature: this is an FFI library, and Rust is one of its consumers rather
than the privileged one — a surface that appears only when somebody
remembers a flag is one a C caller cannot rely on.

The crate is `crate-type = ["rlib", "cdylib"]`, so one package is both the
Rust library and the artifact a C consumer links: `cargo build` produces
`guatiao.dll` / `libguatiao.so`. An rlib exports nothing, which is why the
cdylib exists at all.

A consequence worth knowing: **a plugin cdylib exports these too**,
alongside its own entry symbol, because a cdylib that references this
crate takes its object code. Two copies in one process are harmless, and
that is a property rather than luck — every one of these functions is
pure over the structs plus the allocator pointer the tree carries, so a
tree allocated through one copy frees correctly through another.

**Linking this rlib beside the import library of a DLL that carries it
is `LNK2005` on MSVC.** A Rust crate that depends on `guatiao` for the
Rust API and also links, through its `.lib`, a DLL built from a crate
that depends on `guatiao` (an engine, a plugin host) gets every
`guatiao_*` symbol twice and the link fails with `LNK2005 ... already
defined` and `LNK1169`. Link that DLL by name instead:
`#[link(name = "engine", kind = "raw-dylib")]` on every `extern` block
that imports from it, and no import library on the link line. The
crate then holds this crate's copy and calls the DLL's own functions,
which is the harmless two-copy case above. The exports stay
unconditional: a feature turning them off would be unified across a
whole workspace build, so one crate could not opt out alone.

**`cargo test` does not build a cdylib for the package under test**, only
for a dev-dependency, so `tests/c_exports.rs` needs a `cargo build` first
and states a skip otherwise. CI builds before testing.

The header at
`include/guatiao.h` is rendered by `build.rs` on every build with this
feature on, committed so a C consumer needs no Rust toolchain, and
compared byte for byte by a test. Refresh the committed copy with
`GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header`.

Values: `guatiao_value_{free,clone,null,absent,bool,map,list,string,
number,bytes}`, `guatiao_map_{set,discard,clear,copy_from}`,
`guatiao_list_{push,discard,clear}`, `guatiao_string_push`,
`guatiao_buffer_push`, `guatiao_alloc_default`.
Also `guatiao_merge`, `guatiao_schema_validate`, and the wire encoding
and channels (`guatiao_wire_*`, above). Dates, times and
spans: see "Dates, times and spans from C" below. The path and flat-key
functions (`guatiao_intake_{resolve,flat_keys,flatten,unflatten}`) are
`guatiao-intake`'s, declared in its own `guatiao_intake.h`.

Every function in `guatiao.h` returns `Status`. The two in
`guatiao_intake.h` that answer a borrowed pointer (`guatiao_intake_resolve`
and `guatiao_intake_path_get`) answer a `const guatiao_value *` borrowed from
the schema they were given, or null: a function returning a pointer has no
way to report a status, so null is the only failure it can express.

The statuses (`guatiao_status` is a `uint32_t`; in Rust, `Status` is a
`repr(transparent)` `u32` with these as associated constants):

```
GUATIAO_OK 0   BAD_VALUE 7   ALLOC 8   WRONG_KIND 9   NOT_FOUND 10
NULL 11        GONE 12       UNSUPPORTED 13  BUSY 14  INTERNAL 100
```

**Any `u32` is a `Status`.** A library or host may return a code of its
own; it compares unequal to every constant, `is_ok()` is false, and it
displays as `status <n>`. `From<u32>` and `From<Status> for u32` convert;
there is no `TryFrom`, because there is nothing to refuse.

Contracts that are not in the signatures:

- **Null is refused, not dereferenced.** Every exported function checks.
- **Every boundary catches unwinds**, so a panic never crosses.
- **A tag is a `uint32_t`, not an enum**, because a newer producer may
  write one this build does not know: a reader **skips** it and never
  stops. **A boolean is a `bool`**: 0 or 1 is the contract, and a producer
  that writes another byte has broken it.
- **`guatiao_map_clear` is the map one** and refuses a list, unlike the
  Rust `Value::clear`, which dispatches on the tag.
- **Every out-parameter is written `absent` on entry**, before anything
  can fail, so a caller reading one back after a failure reads what the
  call produced rather than its own uninitialised local. That includes
  the constructors and the registry's answers;
  `guatiao_registry_provider_create` writes `out` null and `err` an empty
  error (`GUATIAO_OK`, empty message). `tests/exports_boundary.rs` holds a
  case per export with an out-parameter and checks the list against the
  header.
- **A null or unusable allocator is `GUATIAO_ERR_ALLOC`**, in every
  module, however it is wrong. A `Str` argument that is not UTF-8 is
  `GUATIAO_ERR_BAD_VALUE`, the registry's included; `GUATIAO_ERR_NULL` is
  only ever a null pointer.
- **The kind is checked before the content.** A node of the wrong kind is
  `GUATIAO_ERR_WRONG_KIND` (`guatiao_map_discard` and
  `guatiao_list_discard` included, which answer `NOT_FOUND` only for a
  missing key or index); the right kind with content a reader refuses (a
  string or a map key that is not UTF-8) is `GUATIAO_ERR_BAD_VALUE`. In
  Rust, `<&str>::try_from`, `<&Map>::try_from` and the number readers
  answer `MapError::BadValue` for that case and `WrongType` for the
  other.
- **A refused `guatiao_map_set` or `guatiao_list_push` takes nothing**:
  the key is copied and the room reserved before `value` is moved, so on
  `GUATIAO_ERR_ALLOC` the caller still owns it.
- **The two pointers of `guatiao_map_set`, `guatiao_list_push` and
  `guatiao_map_copy_from` must not overlap**, and the same pointer for
  both is `GUATIAO_ERR_BAD_VALUE`. `guatiao_map_copy_from` does accept a
  `src` stored inside `dst`: it copies the source whole first.
- **`guatiao_string_push` and `guatiao_buffer_push` accept a view of the
  node's own buffer**; an overlapping source is copied out first.
- **The schema vocabulary is in the header** as `GUATIAO_KEY_*` macros,
  so a C consumer compares keys without spelling them.

## Values as events from C (`exports::visit`)

```c
typedef struct guatiao_visitor {        /* what receives a walk's events */
  uint32_t struct_size; void *ctx;      /* sizeof as compiled; ctx handed back */
  guatiao_status (*absent)(void *ctx);  guatiao_status (*null)(void *ctx);
  guatiao_status (*boolean)(void *ctx, bool value);
  guatiao_status (*number)(void *ctx, guatiao_str text);   /* JSON number grammar */
  guatiao_status (*string)(void *ctx, guatiao_str text);
  guatiao_status (*bytes)(void *ctx, guatiao_bytes bytes);
  guatiao_status (*list_begin)(void *ctx, uint64_t len);   /* or GUATIAO_LEN_UNKNOWN */
  guatiao_status (*list_end)(void *ctx);
  guatiao_status (*map_begin)(void *ctx, uint64_t len);
  guatiao_status (*key)(void *ctx, guatiao_str key);
  guatiao_status (*map_end)(void *ctx);
} guatiao_visitor;                      /* 64-bit: 104 bytes, 13 words */
typedef struct guatiao_walker {         /* what hands its own data to a visitor */
  uint32_t struct_size; void *ctx;
  guatiao_status (*walk)(void *ctx, const guatiao_visitor *into);
} guatiao_walker;

guatiao_status guatiao_value_visit(const guatiao_value *value, const guatiao_visitor *visitor);
guatiao_status guatiao_wire_visit(guatiao_bytes bytes, const guatiao_visitor *visitor);
guatiao_status guatiao_value_from_walker(const guatiao_alloc *alloc, const guatiao_walker *walker,
                                         guatiao_value *out);          /* a value */
guatiao_status guatiao_wire_encode_walker(const guatiao_alloc *alloc, const guatiao_walker *walker,
                                          guatiao_value *out);         /* a BYTES value */
```

A callback's status other than `GUATIAO_OK` stops the walk and is what
the call returns; a null slot refuses its event with
`GUATIAO_ERR_WRONG_KIND`; a table smaller than this build's is
`GUATIAO_ERR_BAD_VALUE`. The visitor a walker is handed is valid for the
`walk` call only, on the calling thread; after its first refusal every
event answers the same status, and the call returns it whatever the
walker returns. A walker's events out of order, a length given at a
begin that the events do not keep (the encoder; a tree takes it as room
only), and a walk that gave no one complete value are
`GUATIAO_ERR_WRONG_KIND`; text not UTF-8, a number outside the grammar,
a key repeated (the encoder) `GUATIAO_ERR_BAD_VALUE`. A callback must
return: never unwind or `longjmp` out (the C ABI). Every export runs
under `catch_unwind` and writes `out` ABSENT first; the walker's
callbacks this library provides catch panics too.
`TableSink` (a C visitor as a `Sink`, `unsafe fn from_ptr`), `Stopped`
and `unsafe fn run_walker(walker, &mut impl Sink) -> Status` are public
for a crate building its own C surface on these (`guatiao-serde` does).

## Dates, times and spans from C

Five forms, each with three functions, `guatiao_<form>_parse` (text into
a struct), `_format` (a struct as canonical text, a string value built
through `alloc`) and `_normalize` (text rewritten canonically), for
`<form>` = `date`, `time`, `datetime`, `timestamp`, `span`; plus
`guatiao_date_add_span`, `guatiao_datetime_add_span` and
`guatiao_duration_read`. In Rust they are `exports::time` and
`exports::span`, the structs `DateFields`, `TimeFields`,
`DateTimeFields`, `TimestampFields`, `SpanFields` and `TextError`, each
`From` the Rust type and `TryFrom` back.

```c
typedef struct guatiao_date { int32_t year; uint8_t month, day; } guatiao_date;
typedef struct guatiao_time { uint8_t hour, minute, second; uint32_t nanosecond; } guatiao_time;
typedef struct guatiao_datetime { guatiao_date date; guatiao_time time; } guatiao_datetime;
typedef struct guatiao_timestamp { int64_t seconds; uint32_t nanoseconds; } guatiao_timestamp;
typedef struct guatiao_span { int64_t years, months, weeks, days, hours, minutes, seconds;
                              int32_t nanoseconds; } guatiao_span;   /* one sign for all */
typedef struct guatiao_text_error { size_t at; guatiao_value expected; } guatiao_text_error;

guatiao_status guatiao_date_parse(const guatiao_alloc *alloc, guatiao_str text,
                                  guatiao_date *out, guatiao_text_error *out_error);
guatiao_status guatiao_date_format(const guatiao_alloc *alloc, const guatiao_date *date,
                                   guatiao_value *out);
guatiao_status guatiao_date_normalize(const guatiao_alloc *alloc, guatiao_str text,
                                      guatiao_value *out, guatiao_text_error *out_error);
guatiao_status guatiao_date_add_span(const guatiao_date *date, const guatiao_span *span,
                                     guatiao_date *out);
guatiao_status guatiao_duration_read(const guatiao_value *value, uint64_t *out_seconds,
                                     uint32_t *out_nanoseconds);
```

- **A refusal is `GUATIAO_ERR_BAD_VALUE`** and, through a non-null
  `out_error`, the byte offset and a string saying what was expected
  there, built through `alloc` (free it with `guatiao_value_free`); with
  a null `alloc` the offset is still written and `expected` stays ABSENT.
  Text that is not UTF-8 is refused at its first bad byte. Nothing quotes
  the text.
- **Out-parameters are written first**: a struct zeroed, a value ABSENT,
  `out_error` `{0, ABSENT}`. A `_parse` needs no allocator when
  `out_error` is null.
- **A struct that is not a form is refused when given**: a date off the
  calendar, a time off the clock, nanoseconds of a second or more, an
  instant outside the years, a span with fields of both signs or
  `INT64_MIN`. A span's every non-zero field carries its one sign
  (`-P1DT2H` is `{.days = -1, .hours = -2}`).
- **`_add_span` may write over its input** (`out` == `date`), and refuses
  a span with time units on a date and any step outside the years.
- **`guatiao_duration_read`** reads `[secs, nanos]`, a number of seconds
  and `{"secs", "nanos"}`, as `Duration`'s `FromValue` does; writing one
  is a list of two numbers.

## The allocator

**An `Allocator` may be called from any thread**, the way Rust's global
allocator can, because a value built through it is `Send` and `Sync` and
is freed wherever it ends up. A host handing out an arena synchronises
it; the crate does not lock on the host's behalf.

An owned container carries the allocator that made it, so growth and free
never take one. `cap == 0` means the buffer is **not owned** — a literal
or a borrow, never freed, copied out of on first growth. So `cap >= len`
does **not** hold on input.

**Only growth copies out.** Removing, clearing and replacing write
through the buffer the container already has, whatever its capacity says,
so a literal that will be mutated must live in **writable storage** — a
C `static` without `const`, or a local. One in read-only memory may be
read, cloned, merged, grown and freed, but not emptied.

```c
typedef struct guatiao_alloc {
  uint32_t struct_size;  void *ctx;
  void *(*alloc)(void *ctx, size_t size, size_t align);
  void  (*free)(void *ctx, void *p, size_t size, size_t align);
  void  (*release)(void *ctx);   /* optional */
} guatiao_alloc;
```

An implementer must guarantee, and `struct_size` cannot express:

- `alloc` returns memory aligned to at least `align`, or null. Returning
  a misaligned pointer is treated as null.
- `free` receives the same `(size, align)` the block was allocated with.
  Pairing `_aligned_malloc` with `free` corrupts the heap.
- Neither may unwind, throw, or `longjmp`. A foreign unwind entering Rust
  across a `"C"` boundary cannot be defended against on this side.
- **The allocator outlives every tree allocated through it.** This is a
  contract, not a borrow: `Alloc` carries no lifetime, because
  `Alloc::from_raw` is `unsafe` and hands back whatever lifetime is asked
  for, so the check would be real only for a stack-allocated allocator —
  the case nobody has.

`Alloc::rust()` is the crate's own, borrowed from a constant.
`rust_alloc()` is a `const fn` returning the vtable, for a `static`.

---

# Gotchas

- **A `Value` is not `Copy`**, and neither are the four containers. They
  have `Drop`.
- **A tree a library handed over by value is in the library's
  allocators** (its `Alloc::rust()` is a vtable in its own image). A host
  keeping one past that library's `unload` calls `into_local()` on it
  first; a returned tree needs nothing, the proxy re-homes it.
- **`clone()` panics on a tree a foreign producer wrote wrong**: an
  unknown tag, text that is not UTF-8, a number outside the grammar. A
  tree that crossed from another language is copied with
  `clone_in(alloc)`, which reports those; `v == v` still holds for it.
- **The fields of `Value`, `Payload`, `Entry` and the four containers are
  private.** Safe code cannot forge a node, corrupt a length, or copy a
  container's fields into a second owner; the compiler refuses all three
  (pinned as `compile_fail` doctests). The one door for a literal or a
  buffer another language owns is `unsafe fn from_raw_parts` on each
  container and on `Value` (with `Payload::text/number/bytes/list/map/bool`),
  and `into_raw_parts` is its safe inverse. The borrowed views (`Str`,
  `Bytes`, `Values`, `Entries`, `BytesMut`) are private too, and carry a
  lifetime, so safe code cannot point one at freed memory. The C header
  is unchanged — cbindgen renders private fields and drops the marker.
- **`try_into` on a value needs `TryFrom`, not a bespoke trait.** A trait
  method named `try_into` is ambiguous against std's blanket impl at
  every call site: it compiles inside the crate and fails in a doctest.
- **Interior NUL is legal everywhere.** Keys and string values are
  pointer and length; comparing only to the first NUL makes two different
  keys look identical.
- **Key comparison is exact.** No case folding, no normalisation, no
  trimming: `"Host"` and `"host"` are two keys.
- **`#![forbid(unsafe_code)]` is per module**, and a test names the paths
  allowed to contain `unsafe`: the value model, `library/raw/`, the
  kind runtime's files under `library/kind/`, and `exports`. Every other module must carry the attribute.
