# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A host's registry of libraries, and the instances its providers build.
# Every answer that is data comes back as plain R data, already freed.

.gt_reg <- function(self) {
  .gt_drain()
  if (!.Call(C_value_is_open, self$handle)) .gt_closed()
  self$handle
}

.gt_registry_class <- methods::setRefClass("guatiao_registry",
  fields = list(handle = "externalptr", keep = "list"),
  methods = list(
    close = function() {
      "Releases the registry; the libraries it loaded stay mapped."
      .Call(C_registry_free, handle)
      invisible(NULL)
    },
    is_open = function() {
      "Whether the registry is open."
      .Call(C_value_is_open, handle)
    },
    load_file = function(path) {
      "Loads one library file: list(loaded = ...), list(skipped = ...) or list(failed = ...)."
      .Call(C_registry_load_file, .gt_reg(.self), path.expand(path))
    },
    register_entry = function(name, entry) {
      "Registers a library linked rather than loaded, from its entry function's address."
      .Call(C_registry_register_entry, .gt_reg(.self), name, entry)
    },
    register_library = function(lib) {
      "Registers a library written in R (a library_of() or guatiao_library()) under its id."
      if (is.character(lib) || is.environment(lib) && !inherits(lib, "guatiao_library")) lib <- library_of(lib)
      if (!inherits(lib, "guatiao_library")) stop("lib is a library", call. = FALSE)
      keep[[length(keep) + 1L]] <<- lib
      register_entry(lib$id, .gt_entry(lib))
    },
    scan_dir = function(dir, rules = "", descending = FALSE) {
      "Scans a directory: list(loaded, skipped, failed)."
      .Call(C_registry_scan, .gt_reg(.self), path.expand(dir), rules, isTRUE(descending), FALSE)
    },
    scan_path = function(spec, rules = "", descending = FALSE) {
      "Walks a search path of directories and files."
      .Call(C_registry_scan, .gt_reg(.self), spec, rules, isTRUE(descending), TRUE)
    },
    libraries = function() {
      "Every library loaded, as a list of lists."
      .Call(C_registry_query, .gt_reg(.self), 0L, "")
    },
    providers = function(kind = "") {
      "Every provider serving kind (every one when empty), best first."
      .Call(C_registry_query, .gt_reg(.self), 1L, kind)
    },
    available = function(kind = "") {
      "Every provider serving kind that can run here now, best first."
      .Call(C_registry_query, .gt_reg(.self), 2L, kind)
    },
    why_not = function(kind) {
      "Why nothing can serve kind: list(available, why, providers)."
      .Call(C_registry_query, .gt_reg(.self), 3L, kind)
    },
    provider = function(key) {
      "One provider by the key the registry filed it under."
      .Call(C_registry_query, .gt_reg(.self), 4L, key)
    },
    best = function(kind) {
      "The best provider serving kind that can run here."
      .Call(C_registry_query, .gt_reg(.self), 5L, kind)
    },
    provider_available = function(key) {
      "list(available, reason) for one provider, asked now."
      .Call(C_registry_provider_available, .gt_reg(.self), key)
    },
    keyed_by = function(template) {
      "Names providers with template instead of %id."
      invisible(.Call(C_registry_keyed_by, .gt_reg(.self), template, FALSE))
    },
    libraries_keyed_by = function(template) {
      "Names libraries with template instead of %id."
      invisible(.Call(C_registry_keyed_by, .gt_reg(.self), template, TRUE))
    },
    set_priority = function(id, priority) {
      "Ranks every provider with this id."
      invisible(.Call(C_registry_set_priority, .gt_reg(.self), id, as.integer(priority)))
    },
    priority = function(id) {
      "What the host ranked that id; 0 unless it said otherwise."
      .Call(C_registry_priority, .gt_reg(.self), id)
    },
    provider_table = function(key, kind = NULL) {
      "list(table, size, ctx) of the table a provider speaks kind through; table NULL for none."
      .Call(C_registry_provider_table, .gt_reg(.self), key, if (is.null(kind)) NULL else kind)
    },
    provider_config = function(key) {
      "A copy of the provider's configuration schema, or NULL."
      r <- .Call(C_registry_provider_config, .gt_reg(.self), key)
      if (is.null(r)) NULL else .gt_root(r)
    },
    host = function() {
      "The registry's guatiao_host_info, as an external pointer."
      .Call(C_registry_host, .gt_reg(.self))
    },
    create = function(key, config = structure(list(), names = character())) {
      "Builds an instance of a provider from a configuration, checked against its schema first."
      if (is.list(config) && length(config) == 0L) config <- structure(list(), names = character())
      ctx <- .Call(C_registry_create, .gt_reg(.self), key, config)
      .gt_instance_class$new(registry = .self, key = key, ctx = ctx)
    },
    bind = function(key, kind) {
      "A remote of kind over the provider filed under key, valid while the registry is open."
      if (!.gt_is_kind(kind)) stop("kind is a kind()", call. = FALSE)
      t <- provider_table(key, kind$name)
      if (is.null(t$table)) {
        stop(guatiao_error("WrongKind", sprintf("%s does not serve %s through a table", key, kind$name)))
      }
      .gt_check_floor(kind, t)
      .gt_remote(kind, .Call(C_remote_new, .gt_kind_ptr(kind), t$table, t$size, t$ctx, FALSE), .self)
    },
    retire = function(key) {
      "Takes a library out and leaves it mapped until nothing uses it."
      invisible(.Call(C_registry_retire, .gt_reg(.self), key))
    },
    collect = function() {
      "Closes every retired library whose own unload agrees; answers their keys."
      unlist(.Call(C_registry_collect, .gt_reg(.self)))
    },
    unload = function(key, unchecked = FALSE) {
      "Unmaps a library when it agrees: Busy, Unsupported, WrongKind, NotFound otherwise."
      invisible(.Call(C_registry_unload, .gt_reg(.self), key, isTRUE(unchecked)))
    },
    add_source_file = function(path, config = structure(list(), names = character())) {
      "Maps a file exporting guatiao_source and adds the source it answers."
      if (is.list(config) && length(config) == 0L) config <- structure(list(), names = character())
      invisible(.Call(C_registry_add_source_file, .gt_reg(.self), path.expand(path), config))
    },
    watch_dir = function(dir, rules = "") {
      "Watches a directory: each refresh loads what appeared and retires what vanished."
      invisible(.Call(C_registry_watch_dir, .gt_reg(.self), path.expand(dir), rules))
    },
    refresh = function() {
      "Makes the registry agree with every source: list(loaded, retired, skipped, refused)."
      .Call(C_registry_refresh, .gt_reg(.self))
    },
    show = function() {
      cat(if (is_open()) "<guatiao registry>\n" else "<guatiao registry, closed>\n")
    }
  ))

registry <- function(id, version, alloc = NULL) {
  .gt_registry_class$new(handle = .Call(C_registry_new, id, version, alloc), keep = list())
}

.gt_check_floor <- function(kind, t) {
  sizes <- .gt_kind_sizes(kind)
  hash <- .Call(C_table_floor_hash, t$table)
  if (t$size < sizes[["floor"]] || !isTRUE(all.equal(hash, kind$floor_hash))) {
    stop(structure(class = c("guatiao_floor_mismatch", "error", "condition"),
      list(message = sprintf("the table is not the %s kind's: size %d (at least %d), floor hash %.0f (want %.0f)",
        kind$name, as.integer(t$size), as.integer(sizes[["floor"]]), hash, kind$floor_hash), call = NULL,
        kind = kind$name, size = t$size, floor_hash = hash)))
  }
}

# ---- instances ---------------------------------------------------------------------

.gt_instance_class <- methods::setRefClass("guatiao_instance",
  fields = list(registry = "ANY", key = "character", ctx = "ANY", closed = "logical", state = "ANY"),
  methods = list(
    initialize = function(...) {
      initFields(closed = FALSE, ...)
      if (!is.null(ctx)) {
        reg <- registry
        k <- key
        c <- ctx
        flag <- new.env(parent = emptyenv())
        flag$done <- FALSE
        state <<- flag
        reg.finalizer(flag, function(e) if (!e$done) .gt_destroy_instance(reg, k, c), onexit = FALSE)
      }
      invisible(.self)
    },
    close = function() {
      "Releases the instance through the provider that built it."
      if (!closed) {
        closed <<- TRUE
        state$done <<- TRUE
        .gt_destroy_instance(registry, key, ctx)
      }
      invisible(NULL)
    },
    is_open = function() {
      "Whether the instance is open."
      !closed && registry$is_open()
    },
    bind = function(kind) {
      "A remote of kind over this instance, valid until the instance is closed."
      if (closed) .gt_closed()
      t <- registry$provider_table(key, kind$name)
      if (is.null(t$table)) {
        stop(guatiao_error("WrongKind", sprintf("%s does not serve %s through a table", key, kind$name)))
      }
      .gt_check_floor(kind, t)
      .gt_remote(kind, .Call(C_remote_new, .gt_kind_ptr(kind), t$table, t$size, ctx, FALSE), .self)
    },
    show = function() cat(sprintf("<guatiao instance of %s%s>\n", key, if (closed) ", closed" else ""))
  ))

.gt_destroy_instance <- function(reg, key, ctx) {
  if (reg$is_open()) .Call(C_registry_destroy, reg$handle, key, ctx)
  invisible(NULL)
}
