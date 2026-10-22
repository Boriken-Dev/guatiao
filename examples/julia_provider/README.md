# Providers written in Julia

`JlGreeter/` is a guatiao library written in Julia: a package serving the
`greeter` and `counter` kinds:

- `Hello`, its one instance, serving both kinds and every optional slot
  of the greeter; its `start` hands back a conversation (`Chat`), an
  object that tells the listener it was handed and closes it when it is
  closed.
- `Shouter`, built per configuration from a `ShouterConfig`, a struct
  declared with `Guatiao.@config` whose fields give the schema the host
  checks first.

The kinds are declared in
[`../greeter_kind/julia/GreeterKind`](../greeter_kind/julia/GreeterKind/src/GreeterKind.jl)
with `@kind`. Each provider is a struct with methods for the kind's
functions (`greet(::Hello, name::String)`), declared with `@provider`;
a required function with no method for the struct is refused when the
library is built.

The package is a library as the binding reads one:
`Guatiao.library_of(JlGreeter)` names it after the module, takes its
version, authors, description and licence from its `Project.toml`, and
its providers from the `@provider` declarations, in order.

## In a Julia host

```julia
using Guatiao, JlGreeter
reg = Registry("host", "1.0")
Guatiao.register_library(reg, JlGreeter)
```

The example host, [`../julia_host`](../julia_host/), does so. Any host in
the same process, written in any language, calls these providers through
the same tables it calls a Rust or C library's through: the binding's
tests have the Rust caller in `greeter_caller` do it.
