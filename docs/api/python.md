# Python API

The `guatiao` package on PyPI: ctypes bindings over the C ABI, with no
dependencies and nothing to compile. Importing it never loads a native
library; the first call that needs one resolves it from
`GUATIAO_LIBRARY`, then the system's library search.

## Values

::: guatiao.value

## Registry

::: guatiao.registry

## Kind tables

::: guatiao.kinds

## Formats

::: guatiao.serde

## Forms

::: guatiao.intake

## Errors

::: guatiao.errors
