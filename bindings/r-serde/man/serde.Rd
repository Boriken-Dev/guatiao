\name{serde_parse}
\alias{serde_parse}
\alias{serde_emit}
\title{JSON, TOML and YAML}
\usage{
serde_parse(text, format = "json", read_data_uris = FALSE, alloc = NULL)
serde_emit(x, format = "json", pretty = FALSE,
  bytes = c("data_uri", "base64", "array", "refuse"))
}
\arguments{
\item{text}{A document.}
\item{format}{\code{"json"}, \code{"toml"} or \code{"yaml"}.}
\item{read_data_uris}{Read a \code{data:} URI string as bytes.}
\item{alloc}{The allocator the value is built through; \code{NULL} is the package's own.}
\item{x}{A value, a view or R data.}
\item{pretty}{Indent JSON.}
\item{bytes}{How bytes are written.}
}
\value{\code{serde_parse} returns a \code{\link[guatiao]{value}}; \code{serde_emit} a string.}
\description{
A number keeps its exact text both ways. A document that does not parse
signals a \code{guatiao_parse_error} (\code{format}, \code{line} and
\code{column} from 1, the column in characters, 0 when the parser did not say;
\code{reason}, which never quotes the document). TOML writes only a map, and
no null. TOML and YAML need a \code{guatiao_serde} built with them.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  v <- serde_parse("{\"port\": 5900}")
  serde_emit(v, format = "yaml")
  close(v)
}
}
