# Passing data: ownership, allocators and copies

Every owned container in a tree records the allocator that made it, and
frees itself through that allocator wherever it ends up. That makes three
questions worth asking every time a tree crosses a function or a library
boundary: **who owns it now, which allocator is it in, and was anything
copied to get it there?** The answers below keep the copies to the ones a
program actually needs.

The short version:

- **Return trees by value, and take them by value when you keep them.**
  Borrow (`&Map`, `&Value`) what you only read, and lend `&mut Map` to a
  callee that changes the tree you keep.
- **Build a tree where it will live.** A kind call carries the caller's
  allocator to the provider; a library outside a call builds in the one
  its host offered.
- **Move, do not copy.** `Value::from(map)`, `Map::try_from(value)`,
  `remove`, `mem::take` and `into_local()` hand over what you already
  hold; `clone()` is a copy, always into this binary's allocator.

## Return by value, take ownership

A kind is a trait, and its methods say who owns what by their types:

```rust
use guatiao::library::ProviderError;
use guatiao::{Alloc, Map, Text};

#[guatiao::kind]
pub trait Store: Send + Sync {
    /// Reads the configuration and keeps none of it: borrowed.
    fn check(&self, config: &Map) -> Result<bool, ProviderError>;
    /// Keeps the configuration: handed over, a move.
    fn save(&self, config: Map) -> Result<(), ProviderError>;
    /// An answer the caller owns, returned by value.
    fn load(&self) -> Result<Map, ProviderError>;
}
```

- **An argument the callee only reads is borrowed**: `&Map`, `&Value`,
  `Option<&Value>`, `&str`, `&[u8]`. Nothing is copied and nothing is
  owed back.
- **An argument the callee keeps is taken by value**: `Map`, `Value` or
  `List`. It crosses as a move: the slot receives the caller's own node,
  the caller forgets it, and the callee owns it whatever it answers. The
  tree is still in the caller's allocators, which outlive the call; a
  callee that keeps it longer than those calls `into_local()` on it. A
  provider served by one of the language bindings (Python, Dart, Node,
  Go, Julia, R, MATLAB and Octave, Fortran, C++) receives it already
  moved into the binding's allocator, and owns every tree and object
  handed over in a call the binding refuses while receiving it; one
  written in C does what a Rust callee does.
- **An answer is returned by value**: `Map`, `Value`, `List`, `String`, or
  a type of yours. Never a reference into the provider: nothing on the
  caller's side owns what it would borrow, and the attribute refuses one.
- A type of yours taken by value crosses through `ToValue` and
  `FromValue`: built once on the way in and read once on the other side.
  Take the tree itself when the callee would only turn it back into one.

The provider builds what it returns in the allocator the call carried in.
`#[guatiao::kind]` adds `<method>_in(.., alloc)` beside every method that
answers a tree, and that is what a call goes through:

```rust
use std::sync::Mutex;

struct Files {
    saved: Mutex<Option<Map>>,
}

impl Store for Files {
    fn check(&self, config: &Map) -> Result<bool, ProviderError> {
        Ok(config.contains_key("path"))
    }

    fn save(&self, config: Map) -> Result<(), ProviderError> {
        // The caller's tree, now this provider's: kept as it arrived.
        *self.saved.lock().expect("not poisoned") = Some(config);
        Ok(())
    }

    fn load(&self) -> Result<Map, ProviderError> {
        self.load_in(Alloc::rust())
    }

    // What a call goes through: the answer is built in the allocator
    // the caller carried in, so it crosses back without a copy.
    fn load_in(&self, alloc: Alloc) -> Result<Map, ProviderError> {
        let mut map = Map::new_in(alloc);
        map.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;
        Ok(map)
    }
}
```

`load_in`'s default calls `load`, whose answer the proxy then copies into
the caller's allocator: correct, one copy. Overriding `load_in` with the
`_in` forms makes that copy unnecessary, and the proxy hands the tree over
as it is. `set_in` places the entry and its key in `alloc`, but a value
converted from `&str` is built by `Into<Value>` on Rust's heap, so the
leaf is built with `Text::new_in` (or `to_value_in(alloc)` for a type of
yours).

The caller does not change at all:

```rust
fn configure(store: &dyn Store) -> Result<Map, ProviderError> {
    let mut config = Map::new();
    config.set("path", "/etc/app.toml");
    // Lent to `check` for the call, then handed to `save`: nothing is
    // copied either way.
    if store.check(&config)? {
        store.save(config)?;
    }
    // Built in this side's allocator, by the provider.
    store.load()
}
```

## How an engine passes its allocator

Three ways, from the most common:

- **Per call, in the out slot.** Before a call that answers a tree, the
  proxy writes a *carrier* into the out slot: an empty map or list, or an
  ABSENT value, recording the allocator the answer is wanted in. The shim
  reads it and calls `<method>_in` with it. A host that wants an answer in
  an arena of its own calls `<method>_in(.., arena)` on the proxy itself.
- **Outside a call, through the host.** A library's entry point receives a
  `Host`; `host.alloc()` is the allocator the host offered, or `None`.
  What a library builds once and the host keeps (a descriptor, a schema,
  metadata) goes there, which is what `providers!` already does for the
  descriptor.
- **By value, for whatever you hold.** `into_alloc(alloc)` moves a tree
  into an allocator, copying only when some container in it is elsewhere;
  `clone_in(alloc)` copies a borrowed one.

```rust
use guatiao::{Alloc, Map, Text, ValueError};

/// What a library builds outside any call, in the allocator its host
/// offered: in a library, `host.alloc().unwrap_or_else(Alloc::rust)`.
fn describe(alloc: Alloc) -> Result<Map, ValueError> {
    let mut meta = Map::new_in(alloc);
    meta.set_in("vendor", Text::new_in(alloc, "acme")?, alloc)?;
    Ok(meta)
}

let meta = describe(Alloc::rust())?;
// A tree in hand goes where it is wanted, copied only if it is not
// already there.
let kept = meta.into_alloc(Alloc::rust())?;
```

An allocator outlives every tree built through it. That is a contract,
not a borrow: a host that unloads a library first takes back anything
the library's allocator still owns (`into_local()`), and a library never
keeps a tree in its own allocator past its `unload`.

## Avoiding needless copies and moves

```rust
use guatiao::{List, Map, Value};

// Build in place: a child goes into its parent by moving.
let mut child = Map::new();
child.set("depth", 2);
let mut parent = Map::new();
parent.set("child", child);

// A container becomes a node by moving, and comes back out the same way.
let node = Value::from(parent);
let mut parent = Map::try_from(node).expect("a map");

// Read by borrowing, not by converting into an owned copy.
let child: &Map = parent.required("child")?.try_into()?;
let depth: i64 = child.required("depth")?.try_into()?;

// Take a value out instead of cloning it: `remove` hands it back owned,
// and `mem::take` leaves a null behind.
let taken: Value = parent.remove("child").expect("present");
let mut list: List = ["a", "b"].into_iter().collect();
let first = core::mem::take(&mut list[0]);

// Own a tree you were handed: `into_local` copies only what is elsewhere.
let owned = taken.into_local();
```

- **Build in place.** `Map::new()` then `set`, `List::new()` then
  `push`: the leaves are made where they are stored, and a container
  handed to `set` is moved in. Building a tree and then copying it into a
  container (`set(k, tree.clone())`) pays for the tree twice.
- **`into_local()` instead of `clone()` when you own the tree.** It hands
  back a tree that is already this binary's without allocating, and
  copies one that is not, freeing the original through its own
  allocators.
- **`Value::from(container)` and `Map::try_from(value)` are moves.** The
  failed conversion hands the value back untouched.
- **Take, do not copy, out of a container**: `map.remove(key)`,
  `list.remove(i)`, `list.pop()`, `mem::take(map.get_mut(key)?)`, or
  `for (key, value) in map` over an owned map. `map.extend(other)` moves
  every value of `other` in.
- **Borrow to read**: `TryAsRef`, `<&Map>::try_from(value)`,
  `map.required(key)?.try_into()?` read in place. A conversion to an
  owned type (`String`, `Vec<T>`, a derived struct) copies by design.
- **`to_value()` builds a tree from a type; it does not move one you
  already have.** `value.to_value()` on a `Value` is a deep copy.
- **A field of a derived type may be a `Value`** (or `Option<Value>`,
  `Vec<Value>`, `BTreeMap<String, Value>`), for what a record carries
  without modelling it. Reading one copies it into this binary's
  allocator, as `clone()` does.

## Letting a library edit your tree

A kind method may take `&mut Map`, `&mut List` or `&mut Value`: the
caller's own tree, lent for the call, which the callee reads, changes and
grows in place. Choose it when the callee completes or edits a tree the
caller keeps using — fills in defaults, adds what follows from the rest,
removes what it rejects:

- **`&Map`** when the callee only reads it.
- **`&mut Map`** when it changes the caller's tree and the caller keeps
  it: only what changes is built, and nothing is handed back.
- **`Map` by value** when the callee keeps it.
- **A returned `Map`** when the callee builds an answer of its own.

```rust
use guatiao::library::ProviderError;
use guatiao::{Map, Text, TryAsRef};

#[guatiao::kind]
pub trait Settings: Send + Sync {
    /// Completes the configuration where it stands: lent, and changed.
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError>;
}

struct Files;

impl Settings for Files {
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        // Built in the allocator the caller's map records, so nothing has
        // to be moved into it once the call returns.
        let alloc = config.alloc()?;
        if !config.contains_key("path") {
            config.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;
        }
        let Some(path) = config.get("path").and_then(TryAsRef::<str>::try_as_ref) else {
            return Ok(false);
        };
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let dir = Text::new_in(alloc, dir)?;
        config.set_in("dir", dir, alloc)?;
        Ok(true)
    }
}
```

The caller lends it and goes on with its own map:

```rust
fn prepare(settings: &dyn Settings) -> Result<Map, ProviderError> {
    let mut config = Map::new();
    config.set("mode", "strict");
    // Lent for the call: filled in where it stands, and still this
    // side's map afterwards.
    if !settings.check(&mut config)? {
        return Err(ProviderError::new(
            guatiao::Status::GUATIAO_ERR_BAD_VALUE,
            "no path",
        ));
    }
    Ok(config)
}
```

**What goes into the tree is built with the tree's allocator.** A map
grows through the allocator it records, but a value the callee builds
with the short forms (`Map::new()`, `Value::from("..")`, `to_value()`)
lives in the callee's own, and a library's allocator is gone once the
library is unloaded. So once the call returns, answered or refused, the
proxy moves every part of the tree another allocator made into the one
the tree recorded before the call, and frees the original through its
own: the same walk as `make_local_in(alloc)`, which you can call on any
tree you hold. A callee that builds with `config.alloc()?` and the `_in`
forms, as above, leaves that walk nothing to move, and it then allocates
nothing. An allocator that refuses makes the walk fail, the tree left
whole where it stands, and that refusal is the call's error when the
callee itself answered (its own refusal comes first), what it answered
dropped unread; every binding's caller reports it the same way. The
proxy also removes, and reports, a key the callee wrote into a map that
is not UTF-8.

A tree lent this way is never freed or kept by the callee, so a library
may hold nothing that points into it once it returns. In C it crosses as
`guatiao_map *` (or `guatiao_list *`, `guatiao_value *`), the same type
as a tree handed over, and the slot's comment in the header says "Lent
for the call, not handed over".

## Every way to a tree you own, and when it copies

| you have | you want | write | copies |
| --- | --- | --- | --- |
| a `Value` you own | it, local | `v.into_local()` | only when some container is not in `Alloc::rust()` |
| a `Value` you own | it, in `alloc` | `v.into_alloc(alloc)?` | only when some container is not in `alloc` |
| a `&Value` | your own copy | `v.clone()`, `Value::from_value(v)` | always, into `Alloc::rust()` |
| a `&Value` | a copy in `alloc` | `v.clone_in(alloc)?` | always |
| a `&Value` | a copy beside it | `v.clone_in(v.alloc()?)?` | always |
| a `Map`, `List`, `Text` or `Buffer` | a `Value` | `Value::from(c)` | never |
| a `Value` | a `Map`, `List`, ... | `Map::try_from(v)` | never |
| a value inside a container | it, owned | `remove`, `pop`, `mem::take` | never |
| a type of yours | a tree | `t.to_value()`, `t.to_value_in(alloc)?` | builds one |
| a kind's answer | it, owned | the returned `Map`/`Value`/`List` | never when the provider builds through `<method>_in`, else once at the proxy |
| a tree for a kind call | to keep it on the other side | pass `Map`/`Value`/`List` by value | never |
| a tree for a kind call | to have it read | pass `&Map`/`&Value` | never |
| a tree for a kind call | to have it changed in place | pass `&mut Map`/`&mut List`/`&mut Value` | never; what the callee built in another allocator is moved into the tree's once |
| a tree you were lent or that was changed | it, in `alloc`, where it stands | `t.make_local_in(alloc)?` | only the parts not in `alloc` |

`Map`, `List`, `Text` and `Buffer` answer `is_local()`, `into_local()`,
`into_alloc(alloc)` and `make_local_in(alloc)` the same way.

## From C

- **An answer is built in the allocator you carry in.** Before calling a
  slot whose out parameter is a `guatiao_value *`, `guatiao_map *` or
  `guatiao_list *`, initialise it: an empty map or list (`len` and `cap`
  0) whose `alloc` is yours, or an ABSENT value whose `payload.map.alloc`
  is. The answer comes back in that allocator, or in the provider's own
  when it built elsewhere; either way every container records the one
  that made it, so freeing it is `guatiao_value_free` (a map or list goes
  into a `guatiao_value` with its tag first). A zeroed slot is fine: the
  provider then builds in its own allocator.
- **A slot's error message** is the caller's to free, through the
  allocator its `alloc` names, when its `cap` is not 0.
- **A tree argument without `const` is handed over**, unless the
  slot's comment says it is lent. A `guatiao_map *`, `guatiao_value *` or
  `guatiao_list *` parameter that is not `const` is a move: the slot owns
  the tree from the call on, on every path, and the caller never frees
  it. A `const` one is lent for reading.
- **A slot whose comment says "Lent for the call, not handed over"**
  receives the caller's tree to change: read it, change it, grow it
  through the allocator it records, never free it or keep the pointer. A
  C caller lending one gets back whatever the callee built; a Rust callee
  following the rule above builds it in the tree's own allocator, and
  `guatiao_value_make_local` moves home what another built elsewhere.
- **Writing a provider in C**, the same rules from the other side: build
  the answer through the carrier's `alloc` and write it over the carrier
  without freeing it; free or keep what was handed over, through the
  allocators it records.

```c
guatiao_map out = {NULL, 0, 0, &my_alloc};      /* the carrier */
guatiao_provider_error err = {0};
guatiao_status s = table->load(ctx, &out, &err);
guatiao_value answer = {GUATIAO_MAP, 0};
answer.payload.map = out;
/* A slot that ignored the carrier built it in its library's allocator:
 * move it home while the library is loaded. */
guatiao_value_make_local(&answer, &my_alloc);
/* ... read it ... */
guatiao_value_free(&answer);                    /* through what each part recorded */
```
