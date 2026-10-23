\name{wire}
\alias{wire}
\alias{from_wire}
\alias{to_wire}
\alias{wire_announce}
\alias{wire_send}
\alias{wire_close}
\alias{wire_receiver}
\title{Values as bytes, and channels}
\usage{
from_wire(bytes, alloc = NULL)
to_wire(x)
wire_announce(channel, schema)
wire_send(channel, value)
wire_close(channel)
wire_receiver()
}
\arguments{
\item{bytes}{A raw vector holding one value in the wire encoding.}
\item{alloc}{The allocator the tree is built through; \code{NULL} is the
package's own.}
\item{x}{A value, a view of one, or R data (as \code{\link{value}} takes).}
\item{channel}{A channel number, 0 to 2^64 - 1: an integer, a whole double,
a bit64 \code{integer64}, a \code{number} or a string spelling one.}
\item{schema, value}{A value, a view of one, or R data.}
}
\value{\code{to_wire} and the frame builders return a raw vector;
\code{from_wire} a \code{guatiao_value}, which owns its tree;
\code{wire_receiver} a receiver: a Reference Class object whose methods are
called with \code{$}.}
\description{
The wire encoding carries a tree exactly: a tag byte per node, LEB128
lengths, a number as its own text, bytes as bytes, a map in its order, the
same bytes in every language. \code{from_wire} signals \code{BadValue} for
bytes that are not exactly one value.

A channel announces a schema before its values, and a receiver checks every
value against it. A frame is \code{kind:u8 channel:LEB128 payload} (kind 0 a
schema, 1 a value, 2 the channel's end); the transport says where one ends.
\code{wire_announce}, \code{wire_send} and \code{wire_close} write frames. A
later announcement on a channel replaces its schema; closing it forgets it.

\describe{
\item{\code{rcv$accept(frame)}}{Reads one frame (a raw vector) and answers an
event, a list of class \code{guatiao_wire_event}: \code{kind}
(\code{"schema"}, \code{"value"} or \code{"close"}), \code{channel} (an R
integer when it fits, a double up to 2^53, else a \code{number}) and
\code{value}, the schema now the channel's or the value its schema accepted,
as a \code{guatiao_value} to close, or \code{NULL} for a close.}
\item{\code{rcv$close()}, \code{rcv$is_open()}}{Free the receiver and the
schemas it keeps (a second call does nothing; a receiver never closed is freed
by the garbage collector), and ask whether it is open.}
}

A refused frame signals a condition of class \code{guatiao_wire_malformed}
(bytes that are not a frame; \code{at} is the byte of the frame it went wrong
at), \code{guatiao_wire_no_schema} (a value on a channel with no schema),
\code{guatiao_wire_not_a_schema} (a schema frame whose value is not a
schema) or \code{guatiao_wire_invalid} (a value its schema refuses;
\code{key} is the field, \code{expected} what it accepts or \code{known} the
fields there are), each also a \code{guatiao_wire_refused} and a
\code{\link{guatiao_error}} with the status: \code{BadValue}, \code{NotFound},
\code{WrongKind} and \code{BadValue}. Each carries \code{channel} and
\code{kind}. A refusal changes nothing the receiver keeps and quotes nothing
it was given.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  bytes <- to_wire(list(n = number("1.10"), s = "text"))
  v <- from_wire(bytes)
  value_data(v, numbers = "text")
  close(v)
  schema <- list(type = "object", properties = list(greeting = list(type = "string")))
  rcv <- wire_receiver()
  rcv$accept(wire_announce(1, schema))$kind
  event <- rcv$accept(wire_send(1, list(greeting = "hi")))
  close(event$value)
  rcv$accept(wire_close(1))$kind
  rcv$close()
}
}
