# Python API

Three packages on PyPI, ctypes bindings over the C ABI with nothing to
compile, one per library: `guatiao` (the value model, schemas, the
registry, kinds and providers), `guatiao-serde` (`import guatiao_serde`:
JSON, TOML and YAML) and `guatiao-intake` (`import guatiao_intake`:
forms and the flat store), the last two built on the first. Importing
one never loads a native library; the first call that needs one resolves
it from its own variable (`GUATIAO_SERDE_LIBRARY`,
`GUATIAO_INTAKE_LIBRARY`), then `GUATIAO_LIBRARY`, then the system's
library search.

## Values

::: guatiao.value

## Registry

::: guatiao.registry

## Kind tables

::: guatiao.kinds

## Providers in Python

::: guatiao.provide

## Dates, times and spans

::: guatiao.time

## Values on the wire, and channels

::: guatiao.wire

## Secrets

::: guatiao.schema

## Errors

::: guatiao.errors

## For a package over another guatiao library

::: guatiao.ffi

## Formats: guatiao-serde

::: guatiao_serde

## Forms: guatiao-intake

::: guatiao_intake
