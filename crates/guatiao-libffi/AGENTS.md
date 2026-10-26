# guatiao-libffi — shipped API header

libffi as a guatiao `Mechanism`: function pointers for, and calls through,
kind tables built at run time from a kind's description. Depends on
`guatiao` and `libffi` (built from C source). No features. Not on `wasm32`.

## API

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Libffi;
unsafe impl guatiao::library::kind::Mechanism for Libffi;
```

Pass `&Libffi` where guatiao takes a `&dyn Mechanism`, or
`Arc::new(Libffi)` where it takes an `Arc<dyn Mechanism>`:

```rust
ServedTable::new(kind, implemented, serve, &Libffi)?;   // a table of the kind, every slot a closure
Caller::new(kind, &Libffi)?;                            // calls any table of the kind
BridgeSource::pinned(config, "lua")?.with_mechanism(Arc::new(Libffi)); // guatiao-bridge
```

What it does with each `CSignature`: one libffi call interface, `u32`
back (a status), `uint8_t` (a provider's `available`, a C `bool`) or
`void` (an object kind's or a provider's `destroy`); `CType::Struct`
becomes a libffi struct type, passed by value as the platform's C ABI
passes it. A `CType` it does not know is refused with
`TableError::Mechanism`.

## Gotchas

- **The C build**: libffi builds with MSVC's tools on a Windows MSVC
  target, where a `CC` naming a GNU-style compiler (`clang`) breaks it
  (its build runs `CC` with `cl`'s `/EP`): leave `CC` unset for cargo,
  or set it to `cl` inside a developer shell. Elsewhere it runs
  `configure` and `make`.
- **A closure's status is written widened to a word** (`ffi_arg`), as
  libffi requires of an integer return.
- **Miri cannot run libffi.** Tables can still be tested under Miri in
  guatiao itself, through a mechanism with no foreign code.
