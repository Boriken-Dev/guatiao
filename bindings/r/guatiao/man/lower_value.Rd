\name{lower_value}
\alias{lower_value}
\title{How an object of a class is written as a value}
\usage{
lower_value(x)
}
\arguments{\item{x}{An object of a class \code{\link{value}} does not write itself.}}
\value{R data \code{value} takes.}
\description{
An S3 generic: define \code{lower_value.myclass} to write a class as other data.
The default writes an S4 object as a map of its slots and refuses anything
else; methods are provided for factors (their labels), \code{Date},
\code{POSIXct}, \code{difftime}, this package's time classes and bit64's
\code{integer64} (each element its exact text, \code{NA} null).
}
\examples{
lower_value.celsius <- function(x) list(celsius = unclass(x))
}
