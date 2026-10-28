\name{kind}
\alias{kind}
\alias{kinds_of}
\alias{method}
\alias{lent}
\alias{taken}
\alias{bind_table}
\alias{has_slot}
\alias{call_slot}
\alias{byte_buffer}
\alias{live_objects}
\alias{guatiao_remote-class}
\title{Kinds: typed function tables, called and served}
\usage{
kind(name, floor_hash, ..., object = FALSE, class = .gt_camel(name),
  int64 = "auto", where = topenv(parent.frame()))
kinds_of(document, known = list(), class = .gt_camel, where = new.env())
method(..., returns = NULL, required = FALSE, fallible = FALSE)
lent(shape)
taken(shape)
bind_table(kind, table, size, ctx, keep = NULL)
has_slot(remote, slot)
call_slot(self, i, args, int64 = NULL)
byte_buffer(n = 0L, data = raw(n))
live_objects()
}
\arguments{
\item{name}{The kind's C name: \code{"greeter"}.}
\item{floor_hash}{The \code{<table>_FLOOR_HASH} the kind's C header defines.}
\item{...}{For \code{kind}, one \code{method()} per slot, named, in the order
the header's table declares them. For \code{method}, the slot's arguments,
named, each a shape.}
\item{object}{An object kind: its table starts with \code{destroy}, which is
\code{close()} on a remote.}
\item{class}{The virtual S4 class the kind defines; the C name in CamelCase.
For \code{kinds_of}, a function of the kind's name answering it.}
\item{int64}{How a 64-bit integer is answered: \code{"auto"} an R integer
when it fits, a double up to 2^53 (where a double holds every whole
number), else a \code{number}; \code{"number"} always a \code{number};
\code{"double"} the nearest double; \code{"integer64"} a bit64
\code{integer64} when one holds it (not \code{INT64_MIN}, which is its
\code{NA}, nor past \code{INT64_MAX}), else a \code{number}, and needs
\pkg{bit64} installed. \code{"auto"} never answers an \code{integer64}. For
\code{kind}, what its remotes answer and what its R methods are handed; for
\code{call_slot}, this call's answer (\code{NULL}: the kind's).}
\item{where}{Where the classes and generics are defined: a package's namespace
as it installs, or the global environment for a script. For \code{kinds_of},
a fresh environment by default.}
\item{document}{A kinds document, the \code{<crate>.kinds.json} a kind crate
commits beside its header: its JSON text (read by the library, so no JSON
package is needed), the R data \code{registry$kind_descriptions()} answers,
or a value holding it.}
\item{known}{Kinds, as \code{kind()} answers them, that a slot of the
document may name though the document does not describe them.}
\item{returns}{The answer's shape: \code{NULL} for none, a shape name, or an
object kind.}
\item{required}{A slot every table fills, wherever it is.}
\item{fallible}{A slot that ends in \code{guatiao_provider_error *err}.}
\item{shape}{\code{"value"}, \code{"map"}, \code{"list"}, \code{"bytes"} (lent
only) or an object kind.}
\item{kind}{A kind, as \code{kind()} answers it.}
\item{table, ctx}{External pointers: a table and the context its slots take.}
\item{size}{The table's size in bytes, as the library compiled it.}
\item{keep}{What keeps the table valid (a registry or an instance), checked
on every call.}
\item{remote}{A remote.}
\item{slot}{A slot's name.}
\item{self}{A remote.}
\item{i}{A slot's position in the kind, from 1.}
\item{args}{The slot's arguments, a list in order.}
\item{n}{A buffer's length in bytes.}
\item{data}{A buffer's first contents.}
}
\value{
\code{kind} returns the kind's description, invisibly; \code{kinds_of}
a list of them, named by each kind's C name; \code{method},
\code{lent} and \code{taken} describe a slot or shape; \code{bind_table} a
remote; \code{has_slot} \code{TRUE} or \code{FALSE}; \code{call_slot} the
slot's answer; \code{byte_buffer} a buffer; \code{live_objects} how many R
objects a library can still reach.
}
\description{
A kind is declared once, from its C header. \code{kind()} defines, in
\code{where}: a virtual S4 class (\code{Greeter}); a generic per slot, whose
arguments are \code{self} and the slot's own, dispatching on \code{self};
and a remote class (\code{GreeterRemote}, extending the kind's class and
\code{guatiao_remote}) with a method of each generic that calls the slot.
A slot is found by position. A name two kinds of one package share must take
the same arguments. It refuses what the kind attribute refuses to declare: a
slot named \code{header}, a \code{"list"} argument (a list is lent or
taken), \code{lent("bytes")} on a kind that is not an object kind, and an
object passed or answered on a slot that is not fallible.

\code{kinds_of(document)} declares, as \code{kind()} would, every kind of a
description document (each object kind before the kinds naming it), in
\code{where}: what a host binds a kind from when it has no declaration of
it, such as the document a library carries. A type it does not know, a
borrowed list or kinds referring to each other in a cycle is an error naming
the kind and the slot.

Shapes: \code{"string"} (\code{guatiao_str}: a string), \code{"bytes"} (a raw
vector), \code{"bool"}, \code{"int8"} to \code{"uint64"}, \code{"float"},
\code{"double"}, \code{"size"}, \code{"intptr"} (numbers; see below for
64-bit integers), \code{"value"}, \code{"map"} (borrowed: the caller
passes a value, a view or R data; the provider receives a read-only view,
valid for the call), \code{"list"} (an answer only), \code{lent("map")} and the like (the
caller's tree, writable for the call; what is added is built in the
allocator it records), \code{taken("map")} and the like (handed over: a
value passed is moved and holds absent afterwards, a view or R data is
copied; the provider receives a value it owns), \code{lent("bytes")} (a
\code{byte_buffer()} the provider writes; what it writes is copied back),
\code{taken(K)} and \code{lent(K)} for an object kind \code{K} (an S4 or
Reference Class object with methods of \code{K}'s generics, or a remote;
the provider receives a remote, owned or valid for the call). An answer is a
map, list or value (any R data or a value, built in the allocator the
caller's carrier names), a string, a number, a logical, or an object of an
object kind.

An R number holds whole numbers exactly only to 2^53, and base R has no
64-bit integer. An integer shape takes an R integer or logical, a whole
double, a bit64 \code{integer64}, a \code{number}, or a string spelling an
integer
(\code{"-9223372036854775808"}), read exactly and range-checked; anything
else is refused before the call with \code{BadValue} naming the kind, the
slot and the argument (\dQuote{wide.echo_i64(x): out of range for int64}).
A 64-bit answer (\code{"int64"}, \code{"uint64"}, \code{"size"},
\code{"intptr"}), and a 64-bit argument an R method is handed, is what
\code{int64} says. An R method answers in any spelling an argument takes,
and an answer out of range is refused naming the slot.

A remote's call signals a \code{guatiao_error} with the provider's status
and words; an optional slot the table leaves null is \code{Unsupported}
(\code{has_slot} says whether a table has it). A remote from a registry is
valid while the registry is open and the library loaded; one from an
instance until the instance is closed. An object remote this side owns is
destroyed by \code{close()} or, never closed, by the garbage collector.

A served object's table is built once per kind and class (a slot for each
generic the class has a method of) and kept for the session. What C holds
of the object is a number shaped like an address, never reused: a call
naming one released answers \code{Gone}. It is held until its holder
destroys it, and then closed when its class has a \code{close} method, or
until the call returns when it was lent.

R code runs only on R's main thread. A slot an R object serves, called on
any other thread, answers \code{Unsupported} with \dQuote{R code runs only on
R's main thread} and touches no R object; a destroy asked for there is
queued and done on R's thread at its next call into the package.
}
\examples{
\dontrun{
Greeter <- kind("greeter", floor_hash = 2032737757,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"))
}
}
