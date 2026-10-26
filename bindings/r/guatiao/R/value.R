# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Values: a `guatiao_value` owns a root node; a `guatiao_node` addresses a
# node by its owner and a path of keys and positions, walked again on
# every access. Both are list(owner, path) with a class; the owner is an
# external pointer the C glue frees.

.gt_node <- function(owner, path = list(), cls = "guatiao_node") structure(list(owner, path), class = cls)
.gt_owner <- function(x) .subset2(x, 1L)
.gt_path <- function(x) .subset2(x, 2L)
.gt_root <- function(owner) .gt_node(owner, list(), c("guatiao_value", "guatiao_node"))

# What reads as no value: the answer to a lookup that found nothing, and
# an optional slot left empty. Distinct from NULL, which is the stored null.
absent <- structure(list(), class = "guatiao_absent")

print.guatiao_absent <- function(x, ...) {
  cat("<absent>\n")
  invisible(x)
}

format.guatiao_absent <- function(x, ...) "<absent>"

value <- function(x = absent, alloc = NULL) .gt_root(.Call(C_value_new, x, alloc))

is_open <- function(x) {
  if (inherits(x, "guatiao_node")) return(.Call(C_value_is_open, .gt_owner(x)))
  if (methods::is(x, "guatiao_remote")) return(.Call(C_remote_live, x@handle))
  if (methods::is(x, "guatiao_registry") || methods::is(x, "guatiao_instance")) return(x$is_open())
  stop("not a guatiao object", call. = FALSE)
}

close.guatiao_value <- function(con, ...) {
  .Call(C_value_close, .gt_owner(con))
  invisible(NULL)
}

number <- function(text) {
  stopifnot(is.character(text), length(text) == 1L, !is.na(text))
  structure(text, class = "guatiao_number")
}

print.guatiao_number <- function(x, ...) {
  cat("<number ", unclass(x), ">\n", sep = "")
  invisible(x)
}

format.guatiao_number <- function(x, ...) unclass(x)

.gt_tags <- c("absent", "null", "bool", "number", "string", "bytes", "list", "map")

tag <- function(x) {
  t <- .Call(C_node_tag, x) + 1L
  if (t > length(.gt_tags)) "unknown" else .gt_tags[t]
}

is_absent <- function(x) {
  if (inherits(x, "guatiao_node")) return(.Call(C_node_tag, x) == 0L)
  inherits(x, "guatiao_absent")
}

is_null <- function(x) {
  if (inherits(x, "guatiao_node")) return(.Call(C_node_tag, x) == 1L)
  is.null(x)
}

.gt_numbers <- c(auto = 0L, text = 1L, double = 2L, integer64 = 3L)

# How a 64-bit integer is answered: an R integer when it fits, a double
# up to 2^53, else a `number` (auto); always a `number`; always the
# nearest double; a bit64 integer64 when one holds it, else a `number`.
.gt_int64_modes <- c(auto = 0L, number = 1L, double = 2L, integer64 = 3L)

.gt_int64_code <- function(int64) {
  code <- if (is.character(int64) && length(int64) == 1L) .gt_int64_modes[int64] else NA
  if (is.na(code)) stop("int64 is one of \"auto\", \"number\", \"double\", \"integer64\"", call. = FALSE)
  if (code == 3L) .gt_need_bit64("int64 = \"integer64\"")
  unname(code)
}

# An integer64 is bit64's class: answering one needs that package to be
# installed, though nothing here calls it.
.gt_need_bit64 <- function(what) {
  if (!requireNamespace("bit64", quietly = TRUE)) {
    stop(sprintf("%s needs the bit64 package, which is not installed: install.packages(\"bit64\")", what),
      call. = FALSE)
  }
  invisible(TRUE)
}

value_data <- function(x, numbers = c("auto", "text", "double", "integer64")) {
  numbers <- match.arg(numbers)
  if (numbers == "integer64") .gt_need_bit64("numbers = \"integer64\"")
  if (!inherits(x, "guatiao_node")) return(x)
  .Call(C_node_read, x, .gt_numbers[[numbers]])
}

as.vector.guatiao_node <- function(x, mode = "any") {
  data <- value_data(x)
  if (identical(mode, "any")) data else as.vector(data, mode)
}

as.list.guatiao_node <- function(x, ...) {
  data <- value_data(x)
  if (is.list(data) && !inherits(data, "guatiao_absent")) data else list(data)
}

as.character.guatiao_node <- function(x, ...) {
  data <- value_data(x, numbers = "text")
  if (inherits(data, "guatiao_number")) unclass(data) else as.character(data)
}

as.double.guatiao_node <- function(x, ...) as.double(value_data(x, numbers = "double"))
as.numeric.guatiao_node <- as.double.guatiao_node
as.integer.guatiao_node <- function(x, ...) as.integer(value_data(x))
as.logical.guatiao_node <- function(x, ...) as.logical(value_data(x))
as.raw.guatiao_node <- function(x) as.raw(value_data(x))

length.guatiao_node <- function(x) .Call(C_node_length, x)
names.guatiao_node <- function(x) .Call(C_node_names, x)

.gt_child <- function(x, sel) {
  if (length(sel) != 1L) stop("one key or position at a time", call. = FALSE)
  step <- .Call(C_node_child, x, sel)
  if (is.null(step)) return(NULL)
  .gt_node(.gt_owner(x), c(.gt_path(x), list(step)))
}

`$.guatiao_node` <- function(x, name) .gt_child(x, name)
`[[.guatiao_node` <- function(x, i, ...) .gt_child(x, i)

.gt_set <- function(x, sel, value) {
  if (length(sel) != 1L) stop("one key or position at a time", call. = FALSE)
  # x$k[[1]] <- v assigns x$k's own view back to x$k: nothing to do.
  if (inherits(value, "guatiao_node") && identical(.gt_owner(value), .gt_owner(x))) {
    p <- .gt_path(value)
    if (length(p) == length(.gt_path(x)) + 1L && identical(p[seq_along(.gt_path(x))], .gt_path(x))) {
      step <- p[[length(p)]]
      same <- if (is.character(sel)) identical(step, sel) else is.numeric(sel) && is.integer(step) && step == sel - 1
      if (same) return(invisible(x))
    }
  }
  .Call(C_node_set, x, sel, value)
  invisible(x)
}

`$<-.guatiao_node` <- function(x, name, value) .gt_set(x, name, value)
`[[<-.guatiao_node` <- function(x, i, ..., value) .gt_set(x, i, value)

push <- function(x, ...) {
  .Call(C_node_push, x, list(...))
  invisible(x)
}

insert <- function(x, at, value) {
  .Call(C_node_insert, x, at, value)
  invisible(x)
}

delete <- function(x, at) {
  .Call(C_node_delete, x, at)
  invisible(x)
}

clear <- function(x) {
  .Call(C_node_clear, x)
  invisible(x)
}

merge_from <- function(x, src) {
  .Call(C_node_merge, x, src)
  invisible(x)
}

node_path <- function(x) {
  p <- .gt_path(x)
  if (length(p) == 0L) return("the root")
  out <- character()
  for (i in seq_along(p)) {
    k <- p[[i]]
    if (is.integer(k)) {
      out <- c(out, sprintf("[%d]", k))
    } else if (grepl("^[A-Za-z_][A-Za-z0-9_-]*$", k)) {
      out <- c(out, if (i > 1L) paste0(".", k) else k)
    } else {
      out <- c(out, sprintf("[%s]", k))
    }
  }
  paste(out, collapse = "")
}

print.guatiao_node <- function(x, ...) {
  if (!is_open(x)) {
    cat("<guatiao value, closed>\n")
    return(invisible(x))
  }
  t <- tag(x)
  what <- switch(t, map = sprintf("map of %d", length(x)), list = sprintf("list of %d", length(x)), t)
  cat("<guatiao ", what, if (length(.gt_path(x))) paste0(" at ", node_path(x)), ">\n", sep = "")
  if (t %in% c("map", "list")) utils::str(value_data(x), give.head = FALSE, no.list = TRUE)
  else if (!(t %in% c("absent", "null"))) print(value_data(x))
  invisible(x)
}

format.guatiao_node <- function(x, ...) {
  if (!is_open(x)) return("<guatiao value, closed>")
  paste0("<guatiao ", tag(x), ">")
}

str.guatiao_node <- function(object, ...) utils::str(value_data(object), ...)

# ---- writing other data -----------------------------------------------------

# How a value of a class is written: methods answer data `value()` takes.
lower_value <- function(x) UseMethod("lower_value")

lower_value.default <- function(x) {
  if (isS4(x) && !methods::is(x, "envRefClass")) {
    slots <- methods::slotNames(x)
    out <- lapply(slots, function(s) methods::slot(x, s))
    names(out) <- slots
    keep <- !vapply(out, function(v) is.null(v), logical(1))
    return(out[keep])
  }
  stop(sprintf("cannot write a %s: define lower_value.%s", class(x)[1L], class(x)[1L]), call. = FALSE)
}

# Each element its exact text: a scalar, or a list (a map when named); NA is null.
lower_value.integer64 <- function(x) {
  items <- lapply(.Call(C_integer64_text, x), function(s) if (is.na(s)) NULL else number(s))
  if (length(items) == 1L && is.null(names(x))) return(items[[1L]])
  names(items) <- names(x)
  items
}

lower_value.factor <- function(x) {
  labels <- as.character(x)
  if (length(labels) == 1L) labels else as.list(labels)
}

lower_value.data.frame <- function(x) {
  stop("cannot write a data.frame: give its rows (split) or columns (as.list) as a list", call. = FALSE)
}

.gt_lower_safely <- function(x) {
  tryCatch(list(TRUE, lower_value(x)), error = function(e) list(FALSE, conditionMessage(e)))
}

# An allocator that counts the blocks it handed out and has not had back,
# kept for the session; for tests proving every path frees.
.gt_counting <- function() .Call(C_alloc_counting)
.gt_live <- function(a) .Call(C_alloc_live, a)
.gt_alloc_refuse <- function(a, on) invisible(.Call(C_alloc_refuse, a, on))
