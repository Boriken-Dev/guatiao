\name{schema_of}
\alias{schema_of}
\alias{config_class}
\alias{field}
\alias{from_value}
\alias{redact}
\alias{validate}
\title{Schemas from S4 classes, typed reading, checks and redaction}
\usage{
schema_of(type)
config_class(Class, ..., where = topenv(parent.frame()))
field(type, default, doc = "", ...)
from_value(x, type)
redact(schema, value, placeholder = "[redacted]", alloc = NULL)
validate(schema, value)
}
\arguments{
\item{type}{A class name or generator: an S4 class, or one of \code{"character"},
\code{"integer"}, \code{"numeric"}, \code{"logical"}, \code{"raw"},
\code{"list"}, \code{"ANY"}, \code{"Date"}, \code{"POSIXct"}, \code{"difftime"}.
For \code{field}, the slot's class.}
\item{Class}{The configuration class's name.}
\item{...}{For \code{config_class}, the fields, each a \code{field()}; for
\code{field}, further schema keywords (\code{minLength = 1}).}
\item{where}{The environment the class is defined in.}
\item{default}{The field's default: it is then optional, and the schema states it.}
\item{doc}{The field's \code{description}.}
\item{x}{A value, a view or R data.}
\item{schema, value}{Values, views or R data.}
\item{placeholder}{What replaces each secret.}
\item{alloc}{The allocator the answer is built through.}
}
\value{
\code{schema_of}: the JSON Schema (2020-12) as a list. \code{config_class}: the
class generator, invisibly. \code{field}: a field description.
\code{from_value}: an object of \code{type}. \code{redact}: a new value.
\code{validate}: \code{NULL} when the schema accepts the value, else the
refusal as a list (\code{key}, \code{message}, ...), which never quotes the value.
}
\description{
A configuration is an S4 class: \code{schema_of} reads an object schema from
its slots, in order, every key it does not declare refused, a slot required
unless its prototype holds a value or \code{config_class} gave it a default; a
class union of a type with \code{NULL} may be null. \code{config_class} is
\code{setClass} that also records each field's default, \code{doc} and
keywords. \code{from_value} reads a value back as a type: integers exactly
(a fraction, an exponent or a value past an R integer refused), a missing key
the prototype's or \code{NULL} where allowed, keys a class does not declare
ignored; a refusal is a \code{guatiao_error} \code{BadValue} naming the path
(\code{at agent.port: want an integer, got string}).

\code{redact} replaces every field a schema marks (\code{x-sensitive} or
\code{writeOnly}) at any depth, and whatever the schema does not describe.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  config_class("ShouterConfig", prefix = field("character",
    doc = "What every greeting starts with.", minLength = 1), where = globalenv())
  str(schema_of("ShouterConfig"))
  from_value(list(prefix = "hey"), "ShouterConfig")@prefix
}
}
