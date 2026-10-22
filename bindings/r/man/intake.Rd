\name{intake_check}
\alias{intake_check}
\alias{intake_layout}
\alias{intake_is_visible}
\alias{intake_at}
\alias{intake_for_schema}
\alias{intake_form_for}
\alias{intake_resolve}
\alias{intake_flat_keys}
\alias{intake_flatten}
\alias{intake_unflatten}
\title{Forms, paths and the flat projection}
\usage{
intake_check(schema, form)
intake_layout(schema, form)
intake_is_visible(schema, form, key, values)
intake_at(value, path)
intake_for_schema(schema)
intake_form_for(form, schema, path)
intake_resolve(schema, key)
intake_flat_keys(schema, key)
intake_flatten(schema, key, value)
intake_unflatten(schema, key, store)
}
\arguments{
\item{schema, form, value, values}{Values, views or R data.}
\item{key}{A field's key (for the flat projection, its flat key).}
\item{path}{A path in the intake grammar: \code{agent[1].name}, \code{env[a.b]}.}
\item{store}{A named character vector or list of strings: flat keys to text.}
}
\value{
\code{intake_check}: \code{NULL} when the form fits, else the error as a list
(\code{kind}, \code{at}, \code{message}, ...). \code{intake_layout}: a list of
\code{list(section, fields)}. \code{intake_is_visible}: \code{TRUE} or
\code{FALSE}. \code{intake_at} and \code{intake_resolve}: a view of the node
(a copy when the argument was R data), or \code{NULL}.
\code{intake_for_schema}: a value. \code{intake_form_for} and
\code{intake_unflatten}: a value, or \code{NULL} for nothing there.
\code{intake_flat_keys}: a character vector. \code{intake_flatten}: a named
character vector.
}
\description{
Taking a value in from a person: whether a form fits a schema and how its
fields are laid out, whether a field is shown given what was entered, a path to
one place in a value, the form a schema implies, and the flat projection of a
field onto text keys and back.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  v <- value(list(agent = list(list(name = "one"))))
  value_data(intake_at(v, "agent[0].name"))
  close(v)
}
}
