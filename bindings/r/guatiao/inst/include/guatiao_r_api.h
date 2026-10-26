/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * What a package over another guatiao C library builds on, from C: the
 * guatiao.serde and guatiao.intake packages call their own libraries and
 * pass trees to and from this package through it.
 *
 * A package lists `LinkingTo: guatiao` and `Imports: guatiao`, includes
 * its own copy of guatiao.h before this file, and asks for the table once
 * its library is loaded:
 *
 *   const guatiao_r_api *api =
 *     ((const guatiao_r_api *(*)(void))R_GetCCallable("guatiao", "guatiao_r_api"))();
 *
 * Fields are appended, never reordered: a package built against an older
 * table reads `size` before a field past its own end.
 *
 * A tree crosses as its root guatiao_value: built by one library, it is
 * read and freed by any other, since a value records its own allocator;
 * the library that built it stays loaded for the session.
 */

#ifndef GUATIAO_R_API_H
#define GUATIAO_R_API_H

#include <Rinternals.h>

#ifndef GUATIAO_ABI_VERSION
#error "include guatiao.h before guatiao_r_api.h"
#endif

typedef struct guatiao_r_api {
  /* sizeof(guatiao_r_api) as this package built it. */
  uint32_t size;

  /* The export `name` of the library carrying `surface` ("guatiao",
   * "guatiao_serde", "guatiao_intake"), the library opened on first use
   * by this package's lookup: what use_library(path, library = surface)
   * named, the surface's own variable (GUATIAO_SERDE_LIBRARY,
   * GUATIAO_INTAKE_LIBRARY), then GUATIAO_LIBRARY (a directory; or a file,
   * used when it exports the surface, else the file beside it), then the
   * platform's search. NULL when the library lacks it; an R error
   * (guatiao_library_not_found) when no library is found. */
  void *(*export_of)(const char *surface, const char *name);
  /* Signals guatiao_missing_symbol for an export of `surface` the loaded
   * build lacks, naming the build `feature` ("" for none). Never returns. */
  void (*missing)(const char *surface, const char *name, const char *feature);
  /* Signals the guatiao_error for `status`, carrying `message`. Never
   * returns. */
  void (*raise)(guatiao_status status, const char *message);

  /* This package's allocator, over the C library's malloc: what a tree is
   * built in when nobody names one. */
  const guatiao_alloc *binding_alloc;
  /* The allocator an `alloc` argument names (NULL for binding_alloc). */
  const guatiao_alloc *(*alloc_arg)(SEXP alloc);
  /* A one-string argument as UTF-8 text, valid during the call; an R
   * error naming `what` otherwise. */
  guatiao_str (*str_arg)(SEXP s, const char *what);
  /* Text as an R string (CHARSXP), UTF-8; NULL when it is not UTF-8. */
  SEXP (*mkchar)(guatiao_str text);

  /* The node an argument names: a guatiao value or view as it is, R data
   * built into `tmp` (free it with `free` after the call). An R error for a
   * closed value or data that cannot be a value. */
  const guatiao_value *(*tree_arg)(SEXP x, guatiao_value *tmp);
  /* tree_arg for two or three arguments at once, freeing what it built
   * when a later one fails; pass R_NilValue and NULLs for an unused third. */
  void (*tree_args)(SEXP a, guatiao_value *ta, const guatiao_value **pa, SEXP b, guatiao_value *tb,
                    const guatiao_value **pb, SEXP c, guatiao_value *tc, const guatiao_value **pc);
  /* Frees what a node owns, through the guatiao library, leaving it absent. */
  void (*free)(guatiao_value *v);

  /* A guatiao value (the R object) taking over the root `*root` a call
   * wrote, built in `alloc`; `keep` is held alive with it (R_NilValue
   * for nothing). The root is moved: `*root` is the value's now. */
  SEXP (*value_new)(guatiao_value *root, const guatiao_alloc *alloc, SEXP keep);
  /* A tree read as R data, then freed. `numbers`: 0 as value_data's
   * default. */
  SEXP (*read_and_free)(guatiao_value *v, int numbers);
  /* A string node's text as an R string, the node freed. */
  SEXP (*take_text)(guatiao_value *v);
  /* What a lookup found inside an argument: for a guatiao value or view
   * (whose node is `root`), a view of `found`; for R data (built into
   * `tmp`), a copy of `found`, and `tmp` freed. R_NilValue when `found`
   * is NULL. */
  SEXP (*view_at)(SEXP arg, const guatiao_value *root, const guatiao_value *found, guatiao_value *tmp);

  /* A document with no tree in between: R data as wire bytes, what
   * tree_arg would build, written with no tree, in memory R keeps until
   * the .Call returns (a status, and why in `msg`); and the wire bytes of a
   * BYTES value read as R data, as read_and_free reads the tree they hold,
   * the value freed either way. */
  guatiao_status (*wire_of)(SEXP x, guatiao_bytes *out, char *msg, size_t msglen);
  SEXP (*read_wire_and_free)(guatiao_value *bytes, int numbers);
} guatiao_r_api;

#endif
