\name{value}
\alias{value}
\alias{absent}
\alias{is_open}
\alias{number}
\alias{tag}
\alias{is_absent}
\alias{is_null}
\alias{value_data}
\alias{node_path}
\alias{push}
\alias{insert}
\alias{delete}
\alias{clear}
\alias{merge_from}
\title{Values: trees shared with the other languages}
\usage{
value(x = absent, alloc = NULL)
absent
is_open(x)
number(text)
tag(x)
is_absent(x)
is_null(x)
value_data(x, numbers = c("auto", "text", "double", "integer64"))
node_path(x)
push(x, ...)
insert(x, at, value)
delete(x, at)
clear(x)
merge_from(x, src)
}
\arguments{
\item{x}{For \code{value}, R data (below). Elsewhere a value or a view of
one; \code{is_absent}, \code{is_null} and \code{value_data} also take R data.}
\item{alloc}{The allocator every container of the tree is built through;
\code{NULL} is the package's own.}
\item{text}{A number's exact text, in JSON's grammar.}
\item{numbers}{How numbers read back: \code{"auto"} (an integer that fits
an R integer as one, else a double when it holds the number exactly, else the
exact text as a \code{number}), \code{"text"} (always the exact text),
\code{"double"} (always the nearest double), \code{"integer64"} (an integer
spelling a bit64 \code{integer64} holds as one, the rest as \code{"auto"};
needs the suggested package \pkg{bit64}, and is an error naming it when it is
not installed).}
\item{...}{Elements to append, each R data or a value.}
\item{at}{A position, from 1, or for a map a key.}
\item{value}{R data or a value to store.}
\item{src}{A map, as a value or R data, whose entries are copied in,
replacing collisions.}
}
\value{\code{value} returns a \code{guatiao_value}, which owns its tree;
\code{v$key}, \code{v[["key"]]} and \code{v[[i]]} return a view
(\code{guatiao_node}) of a node, or \code{NULL} for a key a map does not
hold. The writers return \code{x} invisibly.}
\description{
A \code{guatiao_value} owns a root node; \code{close(v)} frees it (a second
\code{close} does nothing) and a value never closed is freed by the garbage
collector. A view addresses a node by a path of keys and positions, walked
again on every access: a write beside a node can move the array it sits in.
A view of a closed value, or of a node no longer there, signals.

R data becomes a value as follows. \code{NULL} and \code{NA} are null;
\code{absent} is absent; a logical, integer, double or string of length one
is a boolean, a number (its exact text: integers as written, doubles in the
shortest text that reads back as them; NaN and infinities refused) or a
string (UTF-8); a raw vector is bytes; a longer or empty atomic vector, an
unnamed list or \code{I(x)} is a list; a named list or named vector is a map,
in order (named in full, no name twice); a \code{number} keeps its text; a
value or view is copied; a factor is its labels; \code{Date},
\code{POSIXct}, \code{difftime} and this package's time classes are their
text forms (\code{\link{time_format}}); an S4 object is a map of its slots;
anything else goes through \code{\link{lower_value}}.

Reading back (\code{value_data}, \code{as.vector}, \code{as.list}): null is
\code{NULL}, absent is \code{absent}, a boolean \code{TRUE} or \code{FALSE},
a number as \code{numbers} says, a string a UTF-8 string, bytes a raw
vector, a list an unnamed list, a map a named list. \code{as.character} of a
number is its exact text. \code{length} counts a map's entries or a list's
elements (one for any other value, zero for null and absent); \code{names}
are a map's keys.

Writing: \code{v$key <- x} and \code{v[["key"]] <- x} set an entry in place
(a new key goes last); \code{v[[i]] <- x} replaces element \code{i} of a
list, or appends at \code{length + 1}. \code{push} appends, \code{insert}
puts an element at a position, \code{delete} removes a key or position
(nothing to delete is no error), \code{clear} empties a map or list, and
\code{merge_from} copies a map's entries in. What is added is built through
the allocator the container records. A failed write leaves the tree as it was.

\code{tag} names a node's kind: \code{"absent"}, \code{"null"},
\code{"bool"}, \code{"number"}, \code{"string"}, \code{"bytes"},
\code{"list"} or \code{"map"}. \code{node_path} spells a view's place in the
intake path grammar, positions from 0 (\code{"agent[1].name"}).
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  v <- value(list(host = "10.0.0.1", port = 5900L, tags = c("a", "b")))
  v$port <- 5901L
  push(v$tags, "c")
  value_data(v$port)
  str(value_data(v))
  close(v)
}
}
