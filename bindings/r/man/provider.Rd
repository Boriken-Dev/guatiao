\name{provider}
\alias{provider}
\alias{guatiao_library}
\alias{library_of}
\alias{live_instances}
\title{Providers and libraries written in R}
\usage{
provider(class, serves = NULL, name = NULL, id = NULL, display_name = "",
  version = "", meta = NULL, config = NULL, config_schema = NULL, make = NULL,
  default = NULL, available = NULL, where = topenv(parent.frame()))
guatiao_library(id, version, providers, meta = NULL)
library_of(pkg, id = NULL, version = NULL, meta = NULL)
live_instances(lib)
}
\arguments{
\item{class}{The provider's S4 or Reference class: a name or generator.}
\item{serves}{The kinds it serves; by default the kinds its class contains.}
\item{name}{Its name; by default the class's in snake case.}
\item{id}{For \code{provider}, its id (default \code{<library id>_<name>});
for \code{guatiao_library} and \code{library_of}, the library's.}
\item{display_name, version, meta}{What it tells a host; an empty version
inherits the library's, meta is a named list.}
\item{config}{A configuration class: an instance per configuration.}
\item{config_schema}{A configuration schema as R data, the configuration then
read as plain data.}
\item{make}{How an instance is built: \code{make()} for a provider's one
instance, \code{make(config)} for a configured one. By default
\code{new(class)} and \code{new(class, config = config)}.}
\item{default}{Also offer \code{make()} beside a configured provider's
instances; the default without a configuration.}
\item{available}{A function answering \code{NULL} when the provider can run
here and why not otherwise, asked every time a host asks.}
\item{where}{Where the declaration is kept for \code{library_of}; \code{NULL}
keeps none.}
\item{providers}{A list of \code{provider()} specs or class names.}
\item{pkg}{A package's name, or an environment.}
\item{lib}{A library.}
}
\value{
\code{provider} returns its spec, invisibly. \code{guatiao_library} and
\code{library_of} return a library (\code{close()} lets go of each provider's
one instance; a host asking for the descriptor afterwards is declined).
\code{live_instances} counts the instances a host built and has not destroyed.
}
\description{
A provider is an S4 or Reference class that contains the kinds it serves,
with methods of their generics. A required slot with no method, an object
kind among the kinds served, or two providers sharing an id is refused when
the library is made. A method that signals a \code{guatiao_error} answers its
status and words; any other error answers \code{Internal} with its message.

\code{library_of} reads the package: its id is \code{Package}, its version
\code{Version}, its providers the \code{provider()} declarations in its
namespace, in order. Meta, lowest first: \code{Title} as \code{summary},
\code{Description} as \code{description}, \code{Authors@R} (or \code{Author})
as \code{authors}, \code{Maintainer}, \code{License}, \code{URL} and
\code{BugReports} as \code{urls}, the installed package's \file{LICENSE},
\file{LICENCE}, \file{COPYING} and \file{NOTICE} files as \code{notices}, then
\code{meta}. An environment that is no package's namespace needs \code{id}
and \code{version}. Built once per package.

A library's descriptor is built when a host first asks and kept for the
session, with its \code{unload} answering \code{Busy} while an instance lives.
Registered, it is linked, not mapped: \code{unload} answers \code{WrongKind}
and \code{retire} takes it out. R code runs only on R's main thread: a host
calling a provider written in R from another thread is answered
\code{Unsupported}.
}
\examples{
\dontrun{
Hello <- setRefClass("Hello", contains = "Greeter")
setMethod("greet", "Hello", function(self, name) list(greeting = paste0("hello, ", name)))
provider(Hello, display_name = "Hello, in R")
reg$register_library(library_of("mypackage"))
}
}
