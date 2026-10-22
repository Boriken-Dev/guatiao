\name{guatiao_error}
\alias{guatiao_error}
\alias{statuses}
\title{Statuses and the condition a failing one becomes}
\usage{
guatiao_error(status, message = "")
statuses
}
\arguments{
\item{status}{A status name (one of \code{names(statuses)}) or code.}
\item{message}{The words; may be empty.}
}
\value{A condition of class \code{c("guatiao_error", "error", "condition")}
with \code{status} (the name), \code{code} (the number) and \code{message}
(the other side's own words, empty when it gave none).}
\description{
\code{statuses} is the named integer vector of the codes a call answers:
\code{OK} 0, \code{BadValue} 7, \code{AllocFailed} 8, \code{WrongKind} 9,
\code{NotFound} 10, \code{NullArgument} 11, \code{Gone} 12, \code{Unsupported}
13, \code{Busy} 14, \code{Internal} 100.

Every failing status signals a \code{guatiao_error}. A provider's method
signals one to answer that status with its words:
\code{stop(guatiao_error("BadValue", "nobody to greet"))}; any other error
answers \code{Internal} with its message. \code{guatiao_parse_error}
(\code{format}, \code{line}, \code{column}, \code{reason}) and
\code{guatiao_unparsed} (\code{form}, \code{at}, \code{expected}) are
\code{guatiao_error}s with status \code{BadValue}. A closed value, registry or
handle signals \code{guatiao_closed}.
}
\examples{
e <- guatiao_error("BadValue", "nobody to greet")
conditionMessage(e)
statuses[["Busy"]]
}
