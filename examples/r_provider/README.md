# Providers written in R

`rgreeter/` is a guatiao library written in R: a package serving the
`greeter` and `counter` kinds:

- `Hello`, a Reference Class and its one instance, serving both kinds
  and every optional slot of the greeter; its `start` hands back a
  conversation (`Chat`), an object that tells the listener it was handed
  and closes it when it is closed.
- `Shouter`, an S4 class built per configuration from a
  `ShouterConfig`, a class declared with `config_class()` whose slots give
  the schema the host checks first.

The kinds are declared in
[`../greeter_kind/r/greeterkind`](../greeter_kind/r/greeterkind/R/kinds.R)
with `kind()`. Each provider is a class containing the kinds it serves,
with methods of the kind's generics (`setMethod("greet", "Hello", ...)`),
declared with `provider()`; a required generic with no method for the
class is refused when the library is made.

The package is a library as the binding reads one: `library_of("rgreeter")`
names it after the package, takes its version, authors, description and
licence from its `DESCRIPTION`, its notices from the files installed beside
it, and its providers from the `provider()` declarations, in order.

## In an R host

From the repository root, after building the workspace:

```bash
R CMD INSTALL bindings/r/guatiao
R CMD INSTALL examples/greeter_kind/r/greeterkind
R CMD INSTALL examples/r_provider/rgreeter
```

```r
library(guatiao)
reg <- registry("host", "1.0")
reg$register_library(library_of("rgreeter"))
```

The example host, [`../r_host`](../r_host/), does so. Any host in the same
R session, written in any language, calls these providers through the same
tables it calls a Rust or C library's through, on R's main thread: the
binding's tests have the Rust caller in `greeter_caller` do it, and show a
call from any other thread refused.
