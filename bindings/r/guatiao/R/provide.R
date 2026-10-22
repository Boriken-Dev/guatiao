# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A library written in R: providers are S4 or Reference classes with
# methods of their kinds' generics, and the library is the package
# declaring them. Its descriptor is built in C memory on first use and
# kept for the session, since a host may hold it as long as it lives.

.gt_snake <- function(s) tolower(gsub("([a-z0-9])([A-Z])", "\\1_\\2", s))

.gt_providers_var <- ".__guatiao_providers__"

# The kinds a class contains: what it serves when nothing names them.
.gt_kinds_of <- function(class) {
  def <- methods::getClassDef(class)
  if (is.null(def)) stop(sprintf("no class %s", class), call. = FALSE)
  found <- list()
  for (super in names(def@contains)) {
    k <- .gt_kind_of_class(super)
    if (!is.null(k)) found[[length(found) + 1L]] <- k
  }
  found
}

provider <- function(class, serves = NULL, name = NULL, id = NULL, display_name = "", version = "",
                     meta = NULL, config = NULL, config_schema = NULL, make = NULL, default = NULL,
                     available = NULL, where = topenv(parent.frame())) {
  cls <- .gt_class_name(class)
  kinds <- if (is.null(serves)) .gt_kinds_of(cls) else if (.gt_is_kind(serves)) list(serves) else serves
  if (length(kinds) == 0L) stop(sprintf("%s serves no kind: contain one, or name them with serves", cls), call. = FALSE)
  for (k in kinds) {
    if (!.gt_is_kind(k)) stop("serves lists kinds", call. = FALSE)
    if (k$object) stop(sprintf("%s: %s is an object kind; a provider serves provider kinds", cls, k$name), call. = FALSE)
  }
  configured <- !is.null(config) || !is.null(config_schema)
  if (!is.null(config)) config <- .gt_class_name(config)
  if (is.null(make)) {
    make <- if (configured) function(config) methods::new(cls, config = config) else function() methods::new(cls)
  }
  if (!is.null(available) && !is.function(available)) stop("available is a function", call. = FALSE)
  spec <- structure(list(class = cls, kinds = kinds, name = if (is.null(name)) .gt_snake(cls) else name, id = id,
    display_name = display_name, version = version, meta = meta, config = config, config_schema = config_schema,
    make = make, default = if (is.null(default)) !configured else isTRUE(default), available = available,
    configured = configured), class = "guatiao_provider")
  if (!is.null(where)) {
    have <- if (exists(.gt_providers_var, envir = where, inherits = FALSE)) get(.gt_providers_var, envir = where) else list()
    assign(.gt_providers_var, c(have, list(spec)), envir = where)
  }
  invisible(spec)
}

print.guatiao_provider <- function(x, ...) {
  cat(sprintf("<guatiao provider %s (%s) serving %s>\n", x$name, x$class,
    paste(vapply(x$kinds, function(k) k$name, ""), collapse = ", ")))
  invisible(x)
}

guatiao_library <- function(id, version, providers, meta = NULL) {
  specs <- lapply(providers, function(p) if (inherits(p, "guatiao_provider")) p else provider(p, where = NULL))
  if (length(specs) == 0L) stop("a library offers at least one provider", call. = FALSE)
  short <- sub(".*\\.", "", id)
  ids <- vapply(specs, function(p) if (is.null(p$id)) paste0(short, "_", p$name) else p$id, "")
  if (anyDuplicated(ids)) stop(sprintf("two providers of %s share an id: %s", id, paste(ids, collapse = ", ")), call. = FALSE)
  for (p in specs) {
    for (k in p$kinds) {
      has <- vapply(names(k$slots), function(n) isTRUE(methods::hasMethod(n, p$class, where = k$where)), logical(1))
      req <- vapply(k$slots, function(s) s$required, logical(1))
      if (any(req & !has)) {
        stop(sprintf("%s has no method of %s, which the %s kind requires", p$class,
          paste(names(k$slots)[req & !has], collapse = ", "), k$name), call. = FALSE)
      }
    }
  }
  lib <- new.env(parent = emptyenv())
  lib$id <- id
  lib$version <- as.character(version)
  lib$providers <- specs
  lib$provider_ids <- ids
  lib$meta <- meta
  lib$ptr <- NULL
  lib$entry <- NULL
  lib$defaults <- character()
  lib$closed <- FALSE
  class(lib) <- "guatiao_library"
  lib
}

print.guatiao_library <- function(x, ...) {
  cat(sprintf("<guatiao library %s %s, %d providers%s>\n", x$id, x$version, length(x$providers),
    if (x$closed) ", closed" else ""))
  invisible(x)
}

.gt_state$providers <- list()

# Builds the descriptor once; answers the entry function's address.
.gt_entry <- function(lib) {
  if (!is.null(lib$entry) && .Call(C_xptr_live, lib$entry)) return(lib$entry)
  if (lib$closed) stop(sprintf("the library %s is closed", lib$id), call. = FALSE)
  descs <- list()
  for (i in seq_along(lib$providers)) {
    p <- lib$providers[[i]]
    tables <- lapply(p$kinds, function(k) .gt_table_for(k, p$class))
    sizes <- vapply(p$kinds, function(k) .gt_kind_sizes(k)[["full"]], numeric(1))
    schema <- if (!is.null(p$config_schema)) p$config_schema else if (!is.null(p$config)) schema_of(p$config) else NULL
    ctx <- 0
    if (p$default) {
      obj <- p$make()
      id <- .gt_hold(obj)
      lib$defaults <- c(lib$defaults, id)
      ctx <- .gt_ctx_of(id)
    }
    .gt_state$providers[[length(.gt_state$providers) + 1L]] <- list(spec = p, lib = lib)
    descs[[i]] <- list(id = lib$provider_ids[[i]], display_name = p$display_name, version = p$version,
      kinds = vapply(p$kinds, function(k) k$name, ""), tables = tables, sizes = sizes, schema = schema,
      meta = p$meta, available = !is.null(p$available), configured = p$configured, ctx = ctx,
      index = length(.gt_state$providers))
  }
  lib$ptr <- .Call(C_library_new, lib$id, lib$version, lib$meta, descs)
  lib$entry <- .Call(C_library_entry, lib$ptr)
  lib$entry
}

close.guatiao_library <- function(con, ...) {
  if (con$closed) return(invisible(NULL))
  con$closed <- TRUE
  if (!is.null(con$ptr) && .Call(C_xptr_live, con$ptr)) .Call(C_library_close, con$ptr)
  for (id in con$defaults) .gt_release(id, close = TRUE)
  con$defaults <- character()
  invisible(NULL)
}

live_instances <- function(lib) {
  .gt_drain()
  if (is.null(lib$ptr) || !.Call(C_xptr_live, lib$ptr)) return(0)
  .Call(C_library_live, lib$ptr)
}

# ---- what a host's calls reach, on R's thread --------------------------------------

.gt_available <- function(index) {
  p <- .gt_state$providers[[index]]$spec
  why <- tryCatch(p$available(), error = function(e) conditionMessage(e))
  if (is.null(why)) NULL else paste(as.character(why), collapse = " ")
}

.gt_create <- function(index, config) {
  rec <- .gt_state$providers[[index]]
  p <- rec$spec
  tryCatch({
    if (rec$lib$closed) stop(guatiao_error("Gone", "the library is closed"))
    cfg <- if (!is.null(p$config)) from_value(config, p$config) else value_data(config)
    obj <- p$make(cfg)
    id <- .gt_hold(obj)
    list(0L, "", .gt_ctx_of(id))
  }, guatiao_error = function(e) list(as.integer(e$code), e$message, 0),
  error = function(e) list(100L, conditionMessage(e), 0))
}

# ---- a package as a library ----------------------------------------------------------

.gt_state$by_package <- new.env(parent = emptyenv())

.gt_license_files <- c("LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "LICENCE.md", "COPYING", "COPYING.md", "NOTICE", "NOTICE.md")

.gt_package_meta <- function(pkg) {
  d <- utils::packageDescription(pkg)
  if (!is.list(d) && !inherits(d, "packageDescription")) return(list())
  meta <- list()
  squish <- function(s) gsub("\\s+", " ", trimws(s))
  if (!is.null(d$Title)) meta$summary <- squish(d$Title)
  if (!is.null(d$Description)) meta$description <- squish(d$Description)
  authors <- NULL
  if (!is.null(d[["Authors@R"]])) {
    people <- tryCatch(eval(parse(text = d[["Authors@R"]])), error = function(e) NULL)
    if (!is.null(people)) authors <- as.list(format(people, include = c("given", "family", "email")))
  }
  if (is.null(authors) && !is.null(d$Author)) authors <- list(squish(d$Author))
  if (!is.null(authors)) meta$authors <- authors
  if (!is.null(d$Maintainer)) meta$maintainer <- squish(d$Maintainer)
  if (!is.null(d$License)) meta$license <- squish(d$License)
  urls <- list()
  if (!is.null(d$URL)) urls$homepage <- trimws(strsplit(d$URL, "[,[:space:]]+")[[1L]][1L])
  if (!is.null(d$BugReports)) urls$issues <- trimws(d$BugReports)
  if (length(urls)) meta$urls <- urls
  dir <- system.file(package = pkg)
  if (nzchar(dir)) {
    notices <- list()
    for (f in .gt_license_files) {
      path <- file.path(dir, f)
      if (file.exists(path)) {
        notices[[length(notices) + 1L]] <- list(component = pkg,
          text = paste(readLines(path, warn = FALSE, encoding = "UTF-8"), collapse = "\n"), format = "text")
      }
    }
    if (length(notices)) meta$notices <- notices
  }
  meta
}

library_of <- function(pkg, id = NULL, version = NULL, meta = NULL) {
  env <- if (is.environment(pkg)) pkg else asNamespace(pkg)
  name <- if (isNamespace(env)) unname(getNamespaceName(env)) else NULL
  key <- if (!is.null(name)) name else if (!is.null(id)) id else NULL
  if (!is.null(key) && exists(key, envir = .gt_state$by_package, inherits = FALSE)) {
    return(get(key, envir = .gt_state$by_package))
  }
  if (is.null(name) && (is.null(id) || is.null(version))) {
    stop("an environment that is no package's namespace has no id or version: say both", call. = FALSE)
  }
  specs <- if (exists(.gt_providers_var, envir = env, inherits = FALSE)) get(.gt_providers_var, envir = env) else list()
  lib_id <- if (!is.null(id)) id else name
  if (length(specs) == 0L) stop(sprintf("%s declares no provider (provider())", lib_id), call. = FALSE)
  v <- if (!is.null(version)) as.character(version) else as.character(utils::packageVersion(name))
  m <- if (!is.null(name)) .gt_package_meta(name) else list()
  for (k in names(meta)) m[[k]] <- meta[[k]]
  lib <- guatiao_library(lib_id, v, specs, if (length(m)) m else NULL)
  assign(key, lib, envir = .gt_state$by_package)
  lib
}
