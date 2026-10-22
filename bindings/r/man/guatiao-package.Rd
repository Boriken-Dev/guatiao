\docType{package}
\name{guatiao-package}
\alias{guatiao}
\alias{guatiao-package}
\title{guatiao: values, schemas and plugin libraries across languages}
\description{
Bindings over the \sQuote{guatiao} C library. A \code{\link{value}} is a tree
shared with C, Rust, Python, Dart, Node, Go and Julia code; its JSON Schema
description, dates and spans (\code{\link{time_parse}}) and redaction
(\code{\link{redact}}) all go through the library's own functions. JSON, TOML
and YAML text are the package \pkg{guatiao.serde}, forms \pkg{guatiao.intake},
each over its own library and built on the C interface this package registers
(\file{include/guatiao_r_api.h}).
A \code{\link{registry}} loads plugin libraries and calls their providers
through typed function tables declared with \code{\link{kind}}; a provider can
be written in R (\code{\link{provider}}).

The library is not included: it is opened at run time, from
\code{\link{use_library}} or the path in \env{GUATIAO_LIBRARY} (a directory or
a file). Installing the package needs a C compiler and libffi.

R code runs only on R's main thread: a provider written in R that a library
calls from another thread answers the status \code{Unsupported} without
running.
}
\seealso{\url{https://boriken-dev.github.io/guatiao/r/}}
