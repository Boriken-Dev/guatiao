# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Kinds: a kind is a virtual S4 class plus a generic per slot, declared
# once with kind() from its C header; its remote is an S4 class extending
# it whose methods call the slots. The C glue builds a libffi call
# interface per slot, calls a table's slots with it, and serves R objects
# through closures that dispatch back here.

.gt_shapes <- c(string = 1L, bytes = 2L, bool = 3L, int8 = 4L, uint8 = 5L, int16 = 6L, uint16 = 7L,
  int32 = 8L, uint32 = 9L, int64 = 10L, uint64 = 11L, float = 12L, double = 13L, size = 14L, intptr = 15L,
  value = 20L, map = 21L, list = 22L)
.gt_lent_shapes <- c(value = 30L, map = 31L, list = 32L, bytes = 33L)
.gt_taken_shapes <- c(value = 40L, map = 41L, list = 42L)
.GT_LENT_OBJECT <- 34L
.GT_INT64_SHAPES <- c(10L, 11L, 14L, 15L)
.GT_TAKEN_OBJECT <- 44L
.GT_RET_OBJECT <- 50L

# A shape that wraps another: lent (the caller's, for the call) or taken
# (handed over with the call).
lent <- function(shape) structure(list(shape = shape), class = c("guatiao_lent", "guatiao_shape"))
taken <- function(shape) structure(list(shape = shape), class = c("guatiao_taken", "guatiao_shape"))

method <- function(..., returns = NULL, required = FALSE, fallible = FALSE) {
  args <- list(...)
  if (length(args) && (is.null(names(args)) || any(!nzchar(names(args))))) {
    stop("every argument of a slot is named", call. = FALSE)
  }
  structure(list(args = args, returns = returns, required = isTRUE(required), fallible = isTRUE(fallible)),
    class = "guatiao_slot")
}

.gt_is_kind <- function(x) inherits(x, "guatiao_kind")

# The code of an argument shape, and the object kind it names, if any.
.gt_arg_code <- function(s, where) {
  if (.gt_is_kind(s)) stop(sprintf("%s: an object kind is passed lent(...) or taken(...)", where), call. = FALSE)
  if (inherits(s, "guatiao_lent")) {
    if (.gt_is_kind(s$shape)) {
      if (!s$shape$object) stop(sprintf("%s: %s is not an object kind", where, s$shape$name), call. = FALSE)
      return(list(code = .GT_LENT_OBJECT, kind = s$shape))
    }
    code <- .gt_lent_shapes[s$shape]
  } else if (inherits(s, "guatiao_taken")) {
    if (.gt_is_kind(s$shape)) {
      if (!s$shape$object) stop(sprintf("%s: %s is not an object kind", where, s$shape$name), call. = FALSE)
      return(list(code = .GT_TAKEN_OBJECT, kind = s$shape))
    }
    code <- .gt_taken_shapes[s$shape]
  } else {
    code <- .gt_shapes[s]
  }
  if (length(code) != 1L || is.na(code)) stop(sprintf("%s: no such shape", where), call. = FALSE)
  list(code = unname(code), kind = NULL)
}

.gt_ret_code <- function(s, where) {
  if (is.null(s)) return(list(code = 0L, kind = NULL))
  if (.gt_is_kind(s)) {
    if (!s$object) stop(sprintf("%s: %s is not an object kind", where, s$name), call. = FALSE)
    return(list(code = .GT_RET_OBJECT, kind = s))
  }
  code <- .gt_shapes[s]
  if (length(s) != 1L || is.na(code) || s == "bytes") stop(sprintf("%s: no such answer shape", where), call. = FALSE)
  list(code = unname(code), kind = NULL)
}

.gt_camel <- function(name) {
  parts <- strsplit(name, "[_-]")[[1L]]
  paste0(toupper(substring(parts, 1, 1)), substring(parts, 2), collapse = "")
}

.gt_kind_var <- function(class) paste0(".__guatiao_kind_", class)

kind <- function(name, floor_hash, ..., object = FALSE, class = .gt_camel(name), int64 = "auto",
                 where = topenv(parent.frame())) {
  .gt_int64_code(int64)
  slots <- list(...)
  if (length(slots) && (is.null(names(slots)) || any(!nzchar(names(slots))))) {
    stop("every slot is named", call. = FALSE)
  }
  for (s in slots) if (!inherits(s, "guatiao_slot")) stop("each slot is a method()", call. = FALSE)
  if (object && "close" %in% names(slots)) stop("close is an object kind's destroy slot", call. = FALSE)
  spec <- new.env(parent = emptyenv())
  spec$name <- name
  spec$floor_hash <- as.numeric(floor_hash)
  spec$object <- isTRUE(object)
  spec$class <- class
  spec$int64 <- int64
  spec$remote_class <- paste0(class, "Remote")
  spec$where <- where
  spec$slots <- slots
  spec$codes <- lapply(names(slots), function(n) {
    s <- slots[[n]]
    args <- lapply(names(s$args), function(a) .gt_arg_code(s$args[[a]], sprintf("%s.%s(%s)", name, n, a)))
    list(args = args, ret = .gt_ret_code(s$returns, sprintf("%s.%s", name, n)))
  })
  names(spec$codes) <- names(slots)
  required <- vapply(slots, function(s) s$required, logical(1))
  if (length(required) && any(required) && any(diff(as.integer(!required)) < 0L)) {
    stop("required slots come first: they are the kind's original ones", call. = FALSE)
  }
  spec$ptr <- NULL
  class(spec) <- "guatiao_kind"

  methods::setClass(class, representation("VIRTUAL"), where = where)
  for (n in names(slots)) {
    formals <- c(alist(self = ), sapply(names(slots[[n]]$args), function(a) quote(expr = ), simplify = FALSE))
    def <- eval(call("function", as.pairlist(formals), call("standardGeneric", n)))
    existing <- if (exists(n, envir = where, inherits = FALSE)) get(n, envir = where, inherits = FALSE) else NULL
    if (methods::is(existing, "standardGeneric")) {
      if (!identical(names(formals(existing)), names(formals))) {
        stop(sprintf("%s is already a generic here with other arguments: two kinds of one package share a slot name only with the same arguments", n), call. = FALSE)
      }
    } else {
      methods::setGeneric(n, def, signature = "self", where = where)
    }
  }
  methods::setClass(spec$remote_class, contains = c(class, methods::className("guatiao_remote", "guatiao")),
    where = where)
  i <- 0L
  for (n in names(slots)) {
    i <- i + 1L
    arg_names <- names(slots[[n]]$args)
    formals <- c(alist(self = ), sapply(arg_names, function(a) quote(expr = ), simplify = FALSE))
    body <- call("call_slot", quote(self), i, as.call(c(list(as.name("list")), lapply(arg_names, as.name))))
    body[[1L]] <- quote(guatiao::call_slot)
    methods::setMethod(n, spec$remote_class, eval(call("function", as.pairlist(formals), body)), where = where)
  }
  assign(.gt_kind_var(class), spec, envir = where)
  invisible(spec)
}

print.guatiao_kind <- function(x, ...) {
  cat(sprintf("<guatiao kind %s (%s), %d slots%s>\n", x$name, x$class, length(x$slots),
    if (x$object) ", an object kind" else ""))
  invisible(x)
}

# The kind a class name declares, found where the class was defined.
.gt_kind_of_class <- function(class) {
  def <- methods::getClassDef(class)
  if (is.null(def)) return(NULL)
  env <- .gt_class_env(def)
  var <- .gt_kind_var(def@className)
  if (exists(var, envir = env, inherits = FALSE)) get(var, envir = env, inherits = FALSE) else NULL
}

# The generic a slot dispatches to (an object kind's destroy is `close`).
.gt_generic <- function(kind, i) {
  n <- names(kind$slots)[i]
  get(n, envir = kind$where, mode = "function")
}

# The int64 option a kind was declared with: "auto" when its description
# carries none.
.gt_kind_int64 <- function(kind) if (is.null(kind$int64)) "auto" else kind$int64

# The C description of a kind, built once a session.
.gt_kind_ptr <- function(kind) {
  if (is.null(kind$ptr) || !.Call(C_xptr_live, kind$ptr)) {
    codes <- kind$codes
    kind$ptr <- .Call(C_kind_new, kind$name, kind$floor_hash, kind$object,
      lapply(codes, function(c) vapply(c$args, function(a) a$code, integer(1))),
      vapply(codes, function(c) c$ret$code, integer(1)),
      vapply(kind$slots, function(s) s$fallible, logical(1)),
      vapply(kind$slots, function(s) s$required, logical(1)),
      lapply(codes, function(c) lapply(c$args, function(a) if (is.null(a$kind)) NULL else .gt_kind_ptr(a$kind))),
      as.character(names(kind$slots)),
      lapply(kind$slots, function(s) as.character(names(s$args))),
      .gt_int64_code(.gt_kind_int64(kind)))
  }
  kind$ptr
}

# What a kind's table needs to be: its size in bytes with every slot, and
# with its required ones.
.gt_kind_sizes <- function(kind) {
  n <- length(kind$slots) + if (kind$object) 1L else 0L
  req <- sum(vapply(kind$slots, function(s) s$required, logical(1))) + if (kind$object) 1L else 0L
  c(full = 8 + 8 * n, floor = 8 + 8 * req)
}

# ---- remotes -----------------------------------------------------------------

methods::setClass("guatiao_remote", representation("VIRTUAL", handle = "externalptr", keep = "ANY"))

.gt_remote <- function(kind, handle, keep = NULL) {
  methods::new(kind$remote_class, handle = handle, keep = keep)
}

.gt_remote_kind <- function(x) .gt_kind_of_class(sub("Remote$", "", class(x)[1L]))

bind_table <- function(kind, table, size, ctx, keep = NULL) {
  if (!.gt_is_kind(kind)) stop("kind is a kind()", call. = FALSE)
  .gt_remote(kind, .Call(C_remote_new, .gt_kind_ptr(kind), table, as.numeric(size), ctx, FALSE), keep)
}

has_slot <- function(remote, slot) {
  kind <- .gt_remote_kind(remote)
  i <- match(slot, names(kind$slots))
  if (is.na(i)) stop(sprintf("the %s kind has no slot %s", kind$name, slot), call. = FALSE)
  .Call(C_remote_has, remote@handle, i)
}

.gt_keep_open <- function(keep) {
  if (is.null(keep)) return(invisible(TRUE))
  if (!is_open(keep)) .gt_closed()
  invisible(TRUE)
}

# Calls slot `i` (from 1, in the kind's order) of a remote with `args`, the
# slot's arguments in order. The primitive every remote method calls.
call_slot <- function(self, i, args, int64 = NULL) {
  .gt_drain()
  .gt_keep_open(self@keep)
  kind <- .gt_remote_kind(self)
  mode <- .gt_int64_code(if (is.null(int64)) .gt_kind_int64(kind) else int64)
  codes <- kind$codes[[i]]
  handed <- character()
  lent_ids <- character()
  buffers <- list()
  for (j in seq_along(codes$args)) {
    a <- codes$args[[j]]
    if (a$code == 33L) {
      if (!inherits(args[[j]], "guatiao_buffer")) stop(sprintf("argument %d is a byte_buffer()", j), call. = FALSE)
      buffers[[as.character(j)]] <- args[[j]]
      args[[j]] <- args[[j]]$data
    } else if (a$code == .GT_TAKEN_OBJECT || a$code == .GT_LENT_OBJECT) {
      parts <- .gt_object_parts(a$kind, args[[j]])
      if (!is.null(parts$id)) {
        if (a$code == .GT_TAKEN_OBJECT) handed <- c(handed, parts$id) else lent_ids <- c(lent_ids, parts$id)
      }
      args[[j]] <- parts
    }
  }
  on.exit(for (id in lent_ids) .gt_release(id, close = FALSE), add = TRUE)
  out <- tryCatch(.Call(C_remote_call, self@handle, i, args, mode), error = function(e) e)
  if (inherits(out, "error")) {
    # A call that never reached the table leaves what it would have handed over here.
    if (!isTRUE(out$called)) for (id in handed) .gt_release(id, close = FALSE)
    stop(out)
  }
  if (length(buffers)) {
    for (j in names(buffers)) buffers[[j]]$data <- out[[2L]][[as.integer(j)]]
    out <- out[[1L]]
  }
  if (codes$ret$code == .GT_RET_OBJECT) {
    return(.gt_remote(codes$ret$kind, .Call(C_remote_new, .gt_kind_ptr(codes$ret$kind), out[[1L]], out[[2L]],
      out[[3L]], TRUE), NULL))
  }
  if (codes$ret$code %in% c(20L, 21L, 22L)) return(.gt_root(out))
  out
}

close.guatiao_remote <- function(con, ...) {
  .Call(C_remote_close, con@handle)
  invisible(NULL)
}

# ---- serving R objects -------------------------------------------------------------

.gt_state$objects <- new.env(parent = emptyenv())
.gt_state$next_id <- 0
.gt_state$tables <- new.env(parent = emptyenv())

.gt_hold <- function(obj) {
  .gt_state$next_id <- .gt_state$next_id + 1
  id <- format(.gt_state$next_id, scientific = FALSE)
  assign(id, obj, envir = .gt_state$objects)
  id
}

.gt_ctx_of <- function(id) 65536 + 16 * as.numeric(id)

.gt_has_close <- function(obj) {
  cls <- class(obj)[1L]
  isTRUE(tryCatch(methods::existsMethod("close", cls) || methods::hasMethod("close", cls), error = function(e) FALSE)) ||
    !is.null(utils::getS3method("close", cls, optional = TRUE))
}

# Lets go of a held object, closing it when its holder destroyed it.
.gt_release <- function(id, close = TRUE) {
  if (!exists(id, envir = .gt_state$objects, inherits = FALSE)) return(invisible(FALSE))
  obj <- get(id, envir = .gt_state$objects, inherits = FALSE)
  rm(list = id, envir = .gt_state$objects)
  if (close && .gt_has_close(obj)) {
    tryCatch(base::close(obj), error = function(e) .gt_warn(sprintf("close failed: %s", conditionMessage(e))))
  }
  invisible(TRUE)
}

.gt_warn <- function(message) {
  w <- getOption("guatiao.warn")
  if (is.function(w)) w(message) else message("guatiao: ", message)
}

live_objects <- function() {
  .gt_drain()
  length(ls(.gt_state$objects, all.names = TRUE))
}

# Destroys a library's thread asked for, which waited for R's thread.
.gt_drain <- function() {
  for (id in .Call(C_pending_destroys)) .gt_release(format(id, scientific = FALSE), close = TRUE)
  invisible(NULL)
}

# The table serving `kind` for objects of `class`: a slot for each generic
# the class has a method of, null for the rest; built once a session.
.gt_table_for <- function(kind, class, refuse_missing = TRUE) {
  key <- paste(kind$name, kind$class, class, sep = "\r")
  t <- .gt_state$tables[[key]]
  if (!is.null(t) && .Call(C_xptr_live, t)) return(t)
  has <- vapply(seq_along(kind$slots), function(i) {
    isTRUE(methods::hasMethod(names(kind$slots)[i], class, where = kind$where))
  }, logical(1))
  required <- vapply(kind$slots, function(s) s$required, logical(1))
  if (refuse_missing && any(required & !has)) {
    stop(sprintf("%s has no method of %s, which the %s kind requires", class,
      paste(names(kind$slots)[required & !has], collapse = ", "), kind$name), call. = FALSE)
  }
  t <- .Call(C_served_table, .gt_kind_ptr(kind), kind, has)
  assign(key, t, envir = .gt_state$tables)
  t
}

# What a C caller is given for an object of an object kind: a remote this
# side owns (handed over or lent as it is), or an R object served by a
# table of the kind: list(table, size, ctx, id, handle).
.gt_object_parts <- function(kind, x) {
  if (methods::is(x, "guatiao_remote")) {
    return(list(table = NULL, size = 0, ctx = NULL, id = NULL, handle = x@handle))
  }
  if (!isS4(x)) stop(sprintf("an object of the %s kind is an S4 or Reference Class object", kind$name), call. = FALSE)
  table <- .gt_table_for(kind, class(x)[1L])
  id <- .gt_hold(x)
  list(table = table, size = .gt_kind_sizes(kind)[["full"]], ctx = .gt_ctx_of(id), id = id, handle = NULL)
}

.gt_release_id <- function(id) .gt_release(format(id, scientific = FALSE), close = TRUE)

.gt_raise_called <- function(status, message) {
  e <- guatiao_error(as.integer(status), message)
  e$called <- TRUE
  stop(e)
}

# A buffer a lent-bytes slot writes: an environment holding `data`, so a
# call can change it in place.
byte_buffer <- function(n = 0L, data = raw(n)) {
  b <- new.env(parent = emptyenv())
  b$data <- as.raw(data)
  class(b) <- "guatiao_buffer"
  b
}

.gt_buffer_of <- function(data) byte_buffer(data = data)

length.guatiao_buffer <- function(x) length(x$data)
as.raw.guatiao_buffer <- function(x) x$data
`[.guatiao_buffer` <- function(x, i) x$data[i]
`[<-.guatiao_buffer` <- function(x, i, value) {
  d <- x$data
  if (max(i) > length(d)) stop("a buffer does not grow", call. = FALSE)
  d[i] <- as.raw(value)
  x$data <- d
  x
}
print.guatiao_buffer <- function(x, ...) {
  cat(sprintf("<guatiao buffer of %d bytes>\n", length(x$data)))
  invisible(x)
}

# Called by a served slot, on R's thread, inside a protected evaluation:
# finds the object, calls the generic, writes the answer.
.gt_dispatch <- function(kind, i, ctx, args, out) {
  id <- format((ctx - 65536) / 16, scientific = FALSE)
  obj <- if (ctx >= 65536 && (ctx - 65536) %% 16 == 0) .gt_state$objects[[id]] else NULL
  codes <- kind$codes[[i]]
  for (j in seq_along(codes$args)) {
    a <- codes$args[[j]]
    if (a$code == .GT_TAKEN_OBJECT || a$code == .GT_LENT_OBJECT) args[[j]] <- .gt_remote(a$kind, args[[j]], NULL)
    else if (a$code %in% c(20L, 21L, 22L, 30L, 31L, 32L)) args[[j]] <- .gt_node(args[[j]], list())
    else if (a$code %in% c(40L, 41L, 42L)) args[[j]] <- .gt_root(args[[j]])
  }
  if (is.null(obj)) return(list(12L, "the object was released"))
  answer <- tryCatch({
    result <- do.call(.gt_generic(kind, i), c(list(obj), args))
    ret <- kind$codes[[i]]$ret
    if (ret$code == .GT_RET_OBJECT) result <- .gt_object_parts(ret$kind, result)
    .Call(C_answer, out, result)
    list(0L, "")
  }, guatiao_error = function(e) list(as.integer(e$code), e$message),
  error = function(e) list(100L, conditionMessage(e)))
  # What a method left in a lent buffer goes back to the caller's.
  bufs <- lapply(seq_along(codes$args), function(j) {
    if (codes$args[[j]]$code == 33L && inherits(args[[j]], "guatiao_buffer")) args[[j]]$data else NULL
  })
  c(answer, list(bufs))
}
