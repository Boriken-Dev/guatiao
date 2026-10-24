# Kinds in every language

A kind is a function table every language agrees on: Rust declares it as
a trait, the C header lays it out, and its
[description](encodings.md#a-kind-described) says the same as data, beside
the header. Each binding writes a kind and a provider of it the way its
language writes an interface and a class. This page compares them; each
binding's own page has the detail.

| language | a kind is | a provider is | the slots' shapes come from | checked before it runs | an appended slot is |
| --- | --- | --- | --- | --- | --- |
| Rust | `#[guatiao::kind] pub trait Greeter` | a struct with `#[derive(Provider)]` and `#[provider(Greeter)]`, implementing the trait | the trait itself, which writes the header and the description | everything: it is the trait | a method with a default body, which the proxy runs for an older table |
| C | the header's `greeter_vtable` | a `static const greeter_vtable` offered from a `guatiao_provider_info` | the header | the slots' function-pointer types | a null slot, or a table shorter than the struct |
| C++ | `GUATIAO_KIND(greeter, greeter_vtable, (required, greet, guatiao::map(std::string_view)), ...)` | a class whose methods are the slots, by name, offered by `library::single<P, Kinds...>` or `configured` | the macro's slot list, written from the header | the compiler: a missing required slot, a method that does not fit its slot, and a signature that does not stand for its C field | an `(optional, ...)` slot, which the class may leave out |
| TypeScript | an interface and a value of one name, written by `guatiao-kind` from the description; a kind of its own is written as interfaces (`u32`, `Lent<MapRef>`, `extends ObjectKind`) that `guatiao-kind` reads into one | `class Hello extends Provider implements Greeter`, listed in `static kinds` | the description | `tsc --strict`: a missing or mistyped required method, a mistyped appended one | an optional method of the interface |
| JavaScript | the same module, `guatiao-kind`'s `.js`, or `kindsOf()` over the description; a kind of its own is a class extending `Kind`, which `describeKinds()` writes the description of | the same class, without the types | the description, or `static slots` | with `// @ts-check` and `@implements`, what TypeScript checks; at run time, a missing required method is a `TypeError` naming it | an optional method |
| Dart | an abstract interface class, written by `dart run guatiao:kind` from the description | `class Hello extends Provider implements Greeter, GreeterShout`, offered in a `Library` | the description | the analyzer: a missing or mistyped method, appended ones included; at run time, an `ArgumentError` naming a missing one | an interface of its own (`GreeterShout`) |
| Python | `class Greeter(KindProtocol, Protocol, kind=..., floor_hash=...)` | `class Hello(Provider, Greeter)` | the class's annotations, written from the header | mypy and pyright on the methods; at class creation, a missing required one | a method not marked `@required` |
| Go | an interface marked `//guatiao:kind`, its glue written by `guatiao-kind` | any value in `guatiao.Single` or `guatiao.Configured` | the interfaces and the header | `var _ Greeter = (*Hello)(nil)`; the generator, a signature that is not the header's | an interface of its own (`Shouter`) |
| Julia | `@kind Greeter floor_hash = N begin ... end` | `struct Hello <: Greeter`, its methods, and `@provider` | the `@kind` block, written from the header | at run time, when the library is made | a slot not marked `@required` |
| R | `kind("greeter", floor_hash = N, greet = method(...))` | a class containing the kind, `setMethod` per slot, and `provider()` | the `method()` list, written from the header | at run time | `required = FALSE` |
| MATLAB, Octave | `classdef Greeter < guatiao.Kind` with a `guatiao.Kind.describe(...)` constant | `classdef Hello < greeterkind.Greeter` with a `Provider` constant | the `guatiao.slot` list, written from the header | at run time, when the library is made | `'Required'` not true |
| Fortran | an abstract type with a deferred procedure per required slot | a type extending it, offered with `lib%single` | the kind module, written from the header | the compiler: a missing deferred procedure, a wrong interface | an overridable procedure, named where the provider is served |

**The description is the one source after the header.** TypeScript,
JavaScript and Dart read it, so a kind that exists is never declared in
them by hand, and a slot added to the trait reaches them by regenerating
one file. A kind that exists nowhere else is declared once, in TypeScript
as interfaces or in JavaScript as a class, and written out as the same
description, which Dart and the rest read as they read a trait's. Go's generator reads the header, and C++ checks each slot's
signature against the header's when it compiles; Python, Julia, R and
MATLAB compare the floor hash they carry with the header's in their own
tests.

**Where a provider's identity comes from**: Rust's crate name and
version; C++'s `library(id, version)`; TypeScript's and JavaScript's nearest `package.json`; Dart's
`pubspec.yaml`, written into the program by `dart run guatiao:kind
identity`; Python's module and its `__version__`; Go's module path and
build version; Julia's `Project.toml`; R's and MATLAB's `DESCRIPTION`;
Fortran's literals, checked against `fpm.toml` by a test.
