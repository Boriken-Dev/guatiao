\name{use_library}
\alias{use_library}
\alias{has_export}
\title{Name a guatiao library, and ask what it exports}
\usage{
use_library(path, library = "guatiao")
has_export(symbol, library = "guatiao")
}
\arguments{
\item{path}{A file exporting the library's functions, whatever it is called, or a
directory holding it under its platform file name (\file{guatiao.dll},
\file{libguatiao.so}, \file{libguatiao.dylib}; \file{guatiao_serde.dll} and so on).}
\item{symbol}{An export's C name.}
\item{library}{\code{"guatiao"}, \code{"guatiao_serde"} or \code{"guatiao_intake"}.}
}
\value{\code{use_library} returns \code{NULL} invisibly; \code{has_export}
returns \code{TRUE} or \code{FALSE}, loading the library first.}
\description{
Nothing native is touched until the first call that needs a library. It is then
found in this order: the path \code{use_library} named for it; for
\code{guatiao_serde} and \code{guatiao_intake}, the path in their own variable
(\env{GUATIAO_SERDE_LIBRARY}, \env{GUATIAO_INTAKE_LIBRARY}); the path in
\env{GUATIAO_LIBRARY} (a directory holding the library; or a file, taken as it
is for \code{guatiao} and when it exports the others' functions, else the
library beside it); then the platform's own search by file name. A library
stays loaded for the session. The packages \pkg{guatiao.serde} and
\pkg{guatiao.intake} call \code{guatiao_serde} and \code{guatiao_intake}.
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
