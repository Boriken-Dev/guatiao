/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * What the gateway's files share: a call and the refusal it ends with,
 * scratch memory freed when the call returns, text between UTF-8 and
 * character arrays, the table of handles, owners of trees and the walk to
 * a node, and the conversions between MATLAB data and trees.
 *
 * Nothing here raises a MATLAB error. A command records a refusal in its
 * call and returns 0; only `mexFunction` turns it into an error, so no
 * error ever unwinds through a frame that is not the gateway's own.
 */

#ifndef GT_MX_GT_H
#define GT_MX_GT_H

#include <stdarg.h>

#include "mex.h"
#include "native.h"

#if defined(__GNUC__) || defined(__clang__)
#define GT_PRINTF(a, b) __attribute__((format(__printf__, a, b))) /* mex.h may define printf */
#else
#define GT_PRINTF(a, b)
#endif

/* ---- a call ---------------------------------------------------------------- */

typedef struct gt_block {
  struct gt_block *next;
} gt_block;

/* One call into the gateway: its arguments (after the command name), the
 * answers it writes, the refusal it ends with, and its scratch memory. */
typedef struct gt_call {
  int nlhs;
  mxArray **plhs;
  int nrhs;
  const mxArray **prhs;
  guatiao_status status;
  char detail[32]; /* the identifier's third part: "parse", "closed", ... */
  char msg[2048];
  gt_block *scratch;
} gt_call;

/* Records a refusal; answers 0 so a command can `return gt_fail(...)`. */
int gt_fail(gt_call *c, guatiao_status st, const char *fmt, ...) GT_PRINTF(3, 4);
int gt_fail_detail(gt_call *c, guatiao_status st, const char *detail, const char *fmt, ...) GT_PRINTF(4, 5);

/* A refusal carrying a provider's words, which are freed. */
int gt_fail_err(gt_call *c, guatiao_status st, guatiao_provider_error *err, const char *fallback);

/* Frees the words of a guatiao_provider_error, if it holds any. */
void gt_err_free(guatiao_provider_error *err);

/* Memory freed when the call returns, refused or not; NULL when out of
 * memory (and the call refused). */
void *gt_scratch(gt_call *c, size_t n);
void gt_scratch_release(gt_call *c);

/* The name of a status as an identifier part: "BadValue", "Gone", ... */
const char *gt_status_name(guatiao_status st);

/* ---- arguments --------------------------------------------------------------- */

/* Argument `i` (0 is the first after the command), or NULL past the end. */
const mxArray *gt_arg(gt_call *c, int i);

int gt_arg_text(gt_call *c, int i, const char *what, guatiao_str *out);
int gt_arg_double(gt_call *c, int i, const char *what, double *out);
int gt_arg_bool(gt_call *c, int i, const char *what, int *out);
int gt_arg_id(gt_call *c, int i, uint64_t *out);

/* Writes answer `i`; extra answers the caller did not ask for are dropped. */
void gt_answer(gt_call *c, int i, mxArray *x);

/* ---- text -------------------------------------------------------------------- */

/* A character array (a row, or empty) as UTF-8 in the call's scratch. */
int gt_mx_utf8(gt_call *c, const mxArray *a, const char *what, guatiao_str *out);

/* UTF-8 bytes as a character row. Answers NULL when they are not UTF-8
 * (MATLAB holds text as UTF-16). */
mxArray *gt_utf8_mx(guatiao_str t);

/* A string node's text, or a refusal. */
int gt_text_out(gt_call *c, guatiao_str t, mxArray **out);

/* A C string as a character row. */
mxArray *gt_cstr_mx(const char *s);

/* ---- libraries ---------------------------------------------------------------- */

/* Opens `lib` (GT_LIB_*) when it is not open: the path use_library named,
 * then GUATIAO_LIBRARY (a directory or a file), then the platform's search
 * by file name. */
int gt_need(gt_call *c, int lib);

/* Whether the gateway has locked itself (a library is open). */
int gt_locked(void);

/* Keeps the gateway loaded for the session: called before anything that
 * hands out code in its image (a closure, a table of them). */
void gt_lock_gateway(void);

/* Refuses for an export the loaded build lacks, naming its feature. */
int gt_missing(gt_call *c, int lib, const char *name);

#define GT_NEED_CORE(c, name) \
  (gt_need((c), GT_LIB_CORE) && (gt_core.name ? 1 : gt_missing((c), GT_LIB_CORE, #name)))
#define GT_NEED_SERDE(c, name) \
  (gt_need((c), GT_LIB_SERDE) && (gt_serde.name ? 1 : gt_missing((c), GT_LIB_SERDE, #name)))
#define GT_NEED_INTAKE(c, name) \
  (gt_need((c), GT_LIB_INTAKE) && (gt_intake.name ? 1 : gt_missing((c), GT_LIB_INTAKE, #name)))

/* ---- the table of handles ----------------------------------------------------- */

enum {
  GT_H_OWNER = 1,    /* gt_owner: a tree */
  GT_H_COUNTING = 2, /* gt_counting: an allocator that counts */
  GT_H_REGISTRY = 3,
  GT_H_INSTANCE = 4,
  GT_H_REMOTE = 5,
  GT_H_KIND = 6,
  GT_H_LIBRARY = 7,
  GT_H_SOURCE_ENTRY = 8
};

/* Files `ptr` under a new id (never reused); 0 when out of memory. */
uint64_t gt_handle_add(int type, void *ptr);

/* What `id` names when it is of `type`, else NULL. */
void *gt_handle_get(uint64_t id, int type);

/* gt_handle_get, and the entry removed. */
void *gt_handle_take(uint64_t id, int type);

/* How many entries of `type` are filed. */
size_t gt_handle_count(int type);

/* The id an argument names for `type`, refusing a stale or foreign one. */
int gt_arg_handle(gt_call *c, int i, int type, const char *what, void **out);

/* ---- trees and their owners ------------------------------------------------------ */

#define GT_OWNED 1    /* the owner frees the tree and its root */
#define GT_READONLY 2 /* every write through it is refused */

typedef struct gt_owner {
  guatiao_value *root;
  const guatiao_alloc *alloc; /* for containers built where nothing is recorded */
  int flags;
} gt_owner;

/* A new owner of a fresh root holding `v` (moved), filed; its id, or 0
 * (and `v` left as it was) when out of memory. */
uint64_t gt_owner_new(const guatiao_value *v, const guatiao_alloc *a);

/* An owner borrowing `node`, which someone else frees; removed with
 * gt_owner_end when the lender's call returns. */
uint64_t gt_owner_borrow(guatiao_value *node, const guatiao_alloc *a, int readonly);
void gt_owner_end(uint64_t id);

/* Frees an owner the table no longer files. */
void gt_owner_drop(gt_owner *o);

/* The node (owner id, path) at arguments `i` and `i + 1` addresses,
 * walked now. A path is a cell of keys (text) and positions (from 0). */
int gt_arg_node(gt_call *c, int i, gt_owner **owner, guatiao_value **node);

/* The node again after code ran that may have moved it. */
int gt_node_again(gt_call *c, int i, guatiao_value **node);

/* The allocator to build in for a write into `node`. */
const guatiao_alloc *gt_alloc_for(const guatiao_value *node, const gt_owner *o);

/* An allocator argument: [] (the binding's) or a counting allocator's id. */
int gt_arg_alloc(gt_call *c, int i, const guatiao_alloc **out);

/* Frees a value this gateway built or received, leaving it null-tagged. */
void gt_free(guatiao_value *v);

/* ---- conversions ------------------------------------------------------------------ */

#define GT_DEPTH 128

/* Builds MATLAB data `x` in `a` into `out` (written absent first). On
 * refusal nothing built is left. May run M code (an object's lowering). */
int gt_build(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out);

/* Options for reading a tree as MATLAB data. */
typedef struct gt_read_opts {
  int numbers; /* 0 auto, 1 text, 2 double */
  int maps;    /* 0 auto (a struct where it can be), 1 always containers.Map */
} gt_read_opts;

/* A tree as plain MATLAB data. */
int gt_read(gt_call *c, const guatiao_value *v, const gt_read_opts *o, mxArray **out);

/* Tree arguments at positions `at`: a guatiao.Value or view read in
 * place, or MATLAB data built into `tmp`, which the caller frees with
 * gt_free. MATLAB data is built for every argument before any node is
 * found, since building may run M code. */
int gt_tree_args(gt_call *c, int n, const int *at, guatiao_value *tmp, const guatiao_value **out);

/* When `x` is a guatiao.Value or view, `*ref` is {'node', owner id, path}
 * (destroyed by the caller); NULL for anything else. */
int gt_node_ref(gt_call *c, const mxArray *x, mxArray **ref);

/* The node an owner id and a path address, walked now. */
int gt_walk_mx(gt_call *c, const mxArray *owner_id, const mxArray *path, guatiao_value **node);

/* The number a text reads as with the "auto" rule. */
mxArray *gt_number_mx(gt_call *c, guatiao_str t, int numbers);

/* Calls an M function with trapped errors: 0 when it raised (the message
 * recorded in `c`), 1 otherwise. */
int gt_call_m(gt_call *c, const char *fn, int nout, mxArray **out, int nin, mxArray **in);

#endif
