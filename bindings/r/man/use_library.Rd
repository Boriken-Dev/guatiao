\name{use_library}
\alias{use_library}
\alias{has_export}
\title{Name the guatiao library, and ask what it exports}
\usage{
use_library(path)
has_export(symbol, library = "guatiao")
}
\arguments{
\item{path}{A file exporting the C ABI, whatever it is called, or a directory
holding \file{guatiao.dll}, \file{libguatiao.so} or \file{libguatiao.dylib}.}
\item{symbol}{An export's C name.}
\item{library}{\code{"guatiao"}, \code{"guatiao_serde"} or \code{"guatiao_intake"}.}
}
\value{\code{use_library} returns \code{NULL} invisibly; \code{has_export}
returns \code{TRUE} or \code{FALSE}, loading the library first.}
\description{
Nothing native is touched until the first call that needs a library. It is then
found in this order: the path \code{use_library} named, the path in
\env{GUATIAO_LIBRARY}, then the platform's own search by file name.
\code{guatiao_serde} and \code{guatiao_intake} are looked for beside
\code{guatiao} under their own names. A library stays loaded for the session.
Not found signals a \code{guatiao_library_not_found} condition (\code{basename},
\code{looked}); an export the loaded build lacks, when it is used, a
\code{guatiao_missing_symbol} (\code{path}, \code{symbol}, \code{feature}).
Naming another file once one is loaded is an error; naming the same one is not.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  has_export("guatiao_value_map")
}
}
