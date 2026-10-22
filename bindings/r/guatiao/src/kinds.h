/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Kinds as the glue holds them: shapes, slots with their call interfaces,
 * remotes, and the answer slot a served call writes through. */

#ifndef GT_R_KINDS_H
#define GT_R_KINDS_H

#include <ffi.h>

#include "glue.h"

/* Shapes, the codes R's kind() writes. */
enum {
  R_NONE = 0,
  S_STRING = 1,
  S_BYTES = 2,
  S_BOOL = 3,
  S_I8 = 4,
  S_U8 = 5,
  S_I16 = 6,
  S_U16 = 7,
  S_I32 = 8,
  S_U32 = 9,
  S_I64 = 10,
  S_U64 = 11,
  S_F32 = 12,
  S_F64 = 13,
  S_SIZE = 14,
  S_INTPTR = 15,
  S_VALUE = 20, /* borrowed: const guatiao_value * */
  S_MAP = 21,
  S_LIST = 22,
  S_LENT_VALUE = 30, /* the caller's, for the call */
  S_LENT_MAP = 31,
  S_LENT_LIST = 32,
  S_LENT_BYTES = 33, /* guatiao_bytes_mut */
  S_LENT_OBJECT = 34, /* const guatiao_object * */
  S_TAKEN_VALUE = 40, /* handed over */
  S_TAKEN_MAP = 41,
  S_TAKEN_LIST = 42,
  S_TAKEN_OBJECT = 44, /* guatiao_object by value */
  R_OBJECT = 50        /* an answer: guatiao_object *out */
};

#define GT_MAX_ARGS 16

struct gt_kind;

typedef struct gt_slot {
  int nargs;
  int args[GT_MAX_ARGS];
  struct gt_kind *arg_kind[GT_MAX_ARGS]; /* the object kind an object shape names */
  int ret;
  int fallible;
  int required;
  int destroy; /* an object kind's first slot: void (*)(void *ctx) */
  int index_r; /* its position in R's kind(), from 1; 0 for destroy */
  const char *name;                     /* for refusals: the slot's name and its arguments' */
  const char *arg_names[GT_MAX_ARGS];
  struct gt_kind *kind;
  ffi_cif cif;
  ffi_type *types[GT_MAX_ARGS + 3];
  int ntypes;
  ffi_closure *closure; /* serving it, made on first use */
  void *code;
} gt_slot;

typedef struct gt_kind {
  char *name;
  uint32_t floor_hash;
  int object;
  int nslots; /* destroy included */
  size_t size;
  gt_slot *slots;
  char *names; /* every slot's and argument's name, NUL-separated */
  int int64;   /* how a 64-bit integer an R method is handed is answered */
  SEXP spec;   /* the R description, preserved once a table serves it */
} gt_kind;

/* A table this side calls: owned objects are destroyed when let go. */
typedef struct gt_remote {
  gt_kind *kind;
  const void *table;
  size_t size;
  void *ctx;
  int owned;
  int live;
} gt_remote;

/* The out parameter of a served call, valid until the call returns. */
typedef struct gt_answer {
  int live;
  int ret;
  void *out;
  const gt_slot *slot;
} gt_answer;

double gt_id_of_ctx(const void *ctx);
void gt_queue_destroy(double id);
void gt_write_err(guatiao_provider_error *err, guatiao_status st, const char *msg);
extern const char *const GT_WRONG_THREAD;

#endif
