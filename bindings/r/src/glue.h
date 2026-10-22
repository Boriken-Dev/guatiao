/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * What the R-facing glue files share: raising a status as an R condition,
 * text in and out, the owner of a tree and the walk to a node in it, and
 * the conversions between R data and trees.
 *
 * Every function here runs on R's main thread, inside a `.Call` or inside
 * the protected evaluation a served slot makes; none is called from a
 * thread R did not start.
 */

#ifndef GT_R_GLUE_H
#define GT_R_GLUE_H

#define R_NO_REMAP
#define STRICT_R_HEADERS
#include <R.h>
#include <Rinternals.h>

#include "native.h"

/* ---- raising ------------------------------------------------------------- */

/* Signals a guatiao_error carrying `status` and the message; never returns.
 * Callers free what they hold first: the signal unwinds this frame. */
void gt_raise(guatiao_status status, const char *fmt, ...)
#if defined(__GNUC__) || defined(__clang__)
    __attribute__((noreturn, format(printf, 2, 3)))
#endif
    ;

/* Signals with the words of a guatiao_provider_error, freeing them. */
void gt_raise_err(guatiao_status status, guatiao_provider_error *err, const char *fallback)
#if defined(__GNUC__) || defined(__clang__)
    __attribute__((noreturn))
#endif
    ;

/* Frees the words of a guatiao_provider_error, if it holds any. */
void gt_raise_err_free(guatiao_provider_error *err);

/* Opens `lib` through the R side (use_library, GUATIAO_LIBRARY, the
 * platform's search) when it is not open yet. */
void gt_need(int lib);

/* Raises MissingSymbol for an export the loaded build lacks. */
void gt_missing_symbol(int lib, const char *name)
#if defined(__GNUC__) || defined(__clang__)
    __attribute__((noreturn))
#endif
    ;

/* The pointer to call, after checking it is there. */
#define GT_CORE_FN(name) \
  (gt_need(GT_LIB_CORE), gt_core.name ? gt_core.name : (gt_missing_symbol(GT_LIB_CORE, #name), gt_core.name))
#define GT_SERDE_FN(name) \
  (gt_need(GT_LIB_SERDE), gt_serde.name ? gt_serde.name : (gt_missing_symbol(GT_LIB_SERDE, #name), gt_serde.name))
#define GT_INTAKE_FN(name) \
  (gt_need(GT_LIB_INTAKE), gt_intake.name ? gt_intake.name : (gt_missing_symbol(GT_LIB_INTAKE, #name), gt_intake.name))

/* ---- text ---------------------------------------------------------------- */

/* A character(1) as UTF-8 bytes, valid until the .Call returns. */
guatiao_str gt_str_arg(SEXP s, const char *what);

/* A character(1) from bytes refused when they are not UTF-8 or hold a NUL. */
SEXP gt_text_sexp(guatiao_str t);

/* A CHARSXP from UTF-8 bytes; NULL when they hold a NUL. */
SEXP gt_mkchar(guatiao_str t);

/* Moves a string value's text into R and frees the value. */
SEXP gt_take_text(guatiao_value *v);

/* ---- trees and their owners ---------------------------------------------- */

#define GT_OWNED 1   /* the owner frees the tree and its root */
#define GT_READONLY 2 /* every write through it is refused */

/* What an R external pointer to a tree addresses. */
typedef struct gt_owner {
  guatiao_value *root;
  const guatiao_alloc *alloc; /* for containers built where nothing is recorded */
  int flags;
} gt_owner;

/* A new owner of a fresh root node holding `v` (moved), as an external
 * pointer with a finalizer; `keep` is kept alive with it. */
SEXP gt_owner_new(const guatiao_value *v, const guatiao_alloc *a, SEXP keep);

/* An owner borrowing `node`, which someone else frees; cleared with
 * gt_owner_clear when the lender's call returns. */
SEXP gt_owner_borrow(guatiao_value *node, const guatiao_alloc *a, int readonly, SEXP keep);
void gt_owner_clear(SEXP owner);

/* The owner behind an external pointer, or a raise when it is closed. */
gt_owner *gt_owner_of(SEXP owner);

/* The node a guatiao_node (list(owner, path)) addresses, walked now. */
guatiao_value *gt_node_ptr(SEXP node, gt_owner **owner_out);

/* The allocator to build in for a write into `node`: the one its
 * container records, else its owner's. */
const guatiao_alloc *gt_alloc_for(const guatiao_value *node, const gt_owner *o);

/* An allocator argument: NULL or an external pointer to one. */
const guatiao_alloc *gt_alloc_arg(SEXP a);

/* ---- conversions ---------------------------------------------------------- */

/* Builds `x` in `a` into `out` (written ABSENT first). Answers a status;
 * on failure nothing built is left and `msg` says why. */
guatiao_status gt_build(SEXP x, const guatiao_alloc *a, guatiao_value *out, char *msg, size_t msglen);

/* Builds `x` or raises. */
void gt_build_or_raise(SEXP x, const guatiao_alloc *a, guatiao_value *out);

/* A tree as plain R data. `numbers`: 0 auto, 1 text, 2 double, 3 integer64. */
SEXP gt_read(const guatiao_value *v, int numbers);

/* gt_read, freeing `v` whether reading finishes or signals. */
SEXP gt_read_and_free(guatiao_value *v, int numbers);

/* gt_node_ptr without raising: 0 found, 1 closed, 2 no longer there. */
int gt_node_try(SEXP node, gt_owner **owner_out, guatiao_value **out);

/* Frees a value this glue built or received, leaving it null-tagged. */
void gt_free(guatiao_value *v);

/* A tree argument that may be a node (read in place), or R data (built
 * into `tmp`, which the caller frees with gt_free). */
const guatiao_value *gt_tree_arg(SEXP x, guatiao_value *tmp);

/* gt_tree_arg for two or three arguments, each tmp freed by the caller. */
void gt_tree_args(SEXP a, guatiao_value *ta, const guatiao_value **pa, SEXP b, guatiao_value *tb,
                  const guatiao_value **pb, SEXP c, guatiao_value *tc, const guatiao_value **pc);

/* ---- 64-bit integers ------------------------------------------------------ */

/* An integer as a sign and a magnitude: every int64 and uint64, and a flag
 * for a magnitude past 2^64 - 1. Zero is never negative. */
typedef struct gt_wide {
  int neg;
  uint64_t mag;
  int over;
} gt_wide;

/* The `int64 =` option's codes: how a 64-bit integer is answered. */
enum { GT_INT64_AUTO = 0, GT_INT64_NUMBER = 1, GT_INT64_DOUBLE = 2, GT_INT64_INTEGER64 = 3 };

/* The integer one R value names: an integer or logical, a whole double,
 * a bit64 integer64, a `number` or a string spelling one (`-?[0-9]+`).
 * NULL, or why not. */
const char *gt_wide_in(SEXP x, gt_wide *w);

/* The integer a text spells: `-?[0-9]+` and nothing else, past 2^64 - 1
 * setting `over`. NULL, or why not. */
const char *gt_wide_text(const char *p, gt_wide *w);

/* `w` as an integer of `bits` bits (8, 16, 32 or 64); NULL, or why not. */
const char *gt_wide_signed(gt_wide w, int bits, int64_t *out);
const char *gt_wide_unsigned(gt_wide w, int bits, uint64_t *out);

/* A 64-bit integer as `mode` answers it: auto an R integer when it fits,
 * a double up to 2^53, else a `number`; number its exact text;
 * double the nearest double; integer64 a bit64 integer64 when one holds
 * it (not INT64_MIN, which is bit64's NA, nor past INT64_MAX), else a
 * `number`. */
SEXP gt_i64_sexp(int64_t v, int mode);
SEXP gt_u64_sexp(uint64_t v, int mode);

/* A `number` holding `text`, which is the caller's to have checked. */
SEXP gt_number_sexp(const char *text);

/* Whether `x` is a bit64 integer64: a double vector whose bits are int64s. */
int gt_is_integer64(SEXP x);

/* An integer64 holding `v`, which is not INT64_MIN. */
SEXP gt_integer64_sexp(int64_t v);

/* The package's namespace. */
SEXP gt_namespace(void);

/* Calls an R function of the package's namespace with up to three
 * arguments. */
SEXP gt_call_r(const char *fn, int n, SEXP a, SEXP b, SEXP c);

#endif
