\name{registry}
\alias{registry}
\title{A host's registry of plugin libraries}
\usage{
registry(id, version, alloc = NULL)
}
\arguments{
\item{id, version}{How the host introduces itself to a library.}
\item{alloc}{The allocator the registry offers libraries; \code{NULL} is the package's own.}
}
\value{A registry: a Reference Class object whose methods are called with \code{$}.}
\description{
Every answer that is data comes back as plain R data, already freed. A
registry is not safe for concurrent use. \code{reg$close()} releases it; the
libraries it loaded stay mapped.

\describe{
\item{\code{load_file(path)}, \code{scan_dir(dir, rules = "", descending = FALSE)},
\code{scan_path(spec, rules = "", descending = FALSE)}}{Load libraries.
\code{rules}: one \code{[!]KEY=VALUE} per line, matched against what a library
declares before it is mapped (\code{"kind=greeter"}).}
\item{\code{register_entry(name, entry)}, \code{register_library(lib)}}{Register a
library the host links: an entry function's address, or a library written in
R (\code{\link{library_of}}).}
\item{\code{libraries()}, \code{providers(kind = "")}, \code{available(kind = "")},
\code{why_not(kind)}, \code{provider(key)}, \code{best(kind)},
\code{provider_available(key)}}{Ask what is loaded: lists of lists, best first.}
\item{\code{keyed_by(template)}, \code{libraries_keyed_by(template)},
\code{set_priority(id, priority)}, \code{priority(id)}}{File and rank providers.}
\item{\code{set_import_search(search)}, \code{import_search()}}{Where Windows
finds what a file the registry opens imports: \code{"library"}, the default,
looks beside the file, beside the host's executable and in the system
directory; \code{"path"} also in the current directory and on \code{PATH}, as
an R host adding a Python bridge needs. Unix ignores it.}
\item{\code{provider_table(key, kind = NULL)}, \code{provider_config(key)},
\code{host()}}{A provider's table (\code{list(table, size, ctx)}), a copy of its
configuration schema (or \code{NULL}), and the registry's host block.}
\item{\code{kind_descriptions(key)}}{The document of the kinds the provider
serves and of every object kind their slots take or answer, as R data
\code{kinds_of()} loads, or \code{NULL} when it carries none.}
\item{\code{provider_meta(key)}}{What the provider declared beyond its schema,
its notices or its form under \code{guatiao.intake}'s
\code{intake_provider_form_key}: a copy, outliving the library, or \code{NULL}
when it declares nothing; \code{NotFound} for an unknown key.}
\item{\code{create(key, config)}}{An instance built from a configuration,
checked against the provider's schema first (\code{BadValue} naming the field).
\code{inst$close()} reaches the provider that built it, even after a retire or a
refresh loaded a new build; \code{inst$bind(kind)} answers a remote of it. An
instance never closed is closed by the garbage collector.}
\item{\code{bind(key, kind)}}{A remote of \code{kind} over the provider filed under
\code{key}; \code{guatiao_floor_mismatch} when its table is not the kind's.}
\item{\code{retire(key)}, \code{collect()}, \code{unload(key, unchecked = FALSE)}}{Take a
library out (by its library key): retired libraries stay mapped until nothing
uses them; \code{unload} answers \code{Busy}, \code{Unsupported}, \code{WrongKind}
(linked, not mapped) or \code{NotFound} when it cannot unmap.}
\item{\code{add_source_file(path, config)}, \code{watch_dir(dir, rules = "")},
\code{refresh()}}{Sources of libraries that come and go: \code{refresh()} makes the
registry agree with every source, answering \code{list(loaded, retired, skipped,
refused)}. Nothing refreshes on its own.}
}
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  reg <- registry("my-host", "1.0")
  lib <- Sys.getenv("GUATIAO_LIBRARY")
  reg$scan_dir(if (dir.exists(lib)) lib else dirname(lib), rules = "kind=greeter")
  length(reg$providers("greeter"))
  reg$close()
}
}
