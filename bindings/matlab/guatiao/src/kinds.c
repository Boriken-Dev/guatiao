/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Kind tables through libffi: a call interface per slot, built from the
 * shapes a kind declares; a remote calls a table's slot with ffi_call, and
 * an M object is served by a table of ffi closures over one handler.
 *
 * A served slot runs M code only on the interpreter's thread while a call
 * into the gateway is in progress, and then only through
 * mexCallMATLABWithTrap: an M error comes back as a value and never
 * unwinds through the closure and the foreign frames between it and the
 * gateway call below them.
 */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kinds.h"

int gt_gateway_depth(void);

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* ---- ffi types ------------------------------------------------------------ */

static ffi_type *view_elements[3];
static ffi_type view_type; /* guatiao_str, guatiao_bytes, guatiao_bytes_mut */
static ffi_type *object_elements[4];
static ffi_type object_type; /* guatiao_object */
static int types_ready;

void gt_kinds_init(void) {
  if (types_ready) return;
  ffi_type *size_type = sizeof(size_t) == 8 ? &ffi_type_uint64 : &ffi_type_uint32;
  view_elements[0] = &ffi_type_pointer;
  view_elements[1] = size_type;
  view_elements[2] = NULL;
  view_type.size = 0;
  view_type.alignment = 0;
  view_type.type = FFI_TYPE_STRUCT;
  view_type.elements = view_elements;
  object_elements[0] = &ffi_type_pointer;
  object_elements[1] = size_type;
  object_elements[2] = &ffi_type_pointer;
  object_elements[3] = NULL;
  object_type.size = 0;
  object_type.alignment = 0;
  object_type.type = FFI_TYPE_STRUCT;
  object_type.elements = object_elements;
  types_ready = 1;
}

static ffi_type *type_of(int s) {
  switch (s) {
  case S_STRING:
  case S_BYTES:
  case S_LENT_BYTES: return &view_type;
  case S_BOOL: return &ffi_type_uint8;
  case S_I8: return &ffi_type_sint8;
  case S_U8: return &ffi_type_uint8;
  case S_I16: return &ffi_type_sint16;
  case S_U16: return &ffi_type_uint16;
  case S_I32: return &ffi_type_sint32;
  case S_U32: return &ffi_type_uint32;
  case S_I64: return &ffi_type_sint64;
  case S_U64: return &ffi_type_uint64;
  case S_F32: return &ffi_type_float;
  case S_F64: return &ffi_type_double;
  case S_SIZE: return sizeof(size_t) == 8 ? &ffi_type_uint64 : &ffi_type_uint32;
  case S_INTPTR: return sizeof(intptr_t) == 8 ? &ffi_type_sint64 : &ffi_type_sint32;
  case S_TAKEN_OBJECT: return &object_type;
  default: return &ffi_type_pointer;
  }
}

static int known_arg(int s) {
  return (s >= S_STRING && s <= S_INTPTR) || (s >= S_VALUE && s <= S_LIST) || (s >= S_LENT_VALUE && s <= S_LENT_OBJECT) ||
         (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST) || s == S_TAKEN_OBJECT;
}

static int known_ret(int s) {
  return s == R_NONE || s == S_STRING || (s >= S_BOOL && s <= S_INTPTR) || (s >= S_VALUE && s <= S_LIST) || s == R_OBJECT;
}

/* ---- addresses ------------------------------------------------------------- */

mxArray *gt_addr_mx(const void *p) {
  mxArray *x = mxCreateNumericMatrix(1, 1, mxUINT64_CLASS, mxREAL);
  *(uint64_t *)mxGetData(x) = (uint64_t)(uintptr_t)p;
  return x;
}

int gt_mx_addr(const mxArray *a, void **out) {
  *out = NULL;
  if (!a || mxIsEmpty(a)) return 1;
  if (mxGetNumberOfElements(a) != 1) return 0;
  if (mxGetClassID(a) == mxUINT64_CLASS) {
    *out = (void *)(uintptr_t) * (const uint64_t *)mxGetData(a);
    return 1;
  }
  if (mxIsDouble(a)) {
    double d = mxGetScalar(a);
    if (d < 0 || d != floor(d) || d > 9007199254740992.0) return 0;
    *out = (void *)(uintptr_t)d;
    return 1;
  }
  return 0;
}

/* ---- kinds --------------------------------------------------------------------- */

static int prep_slot(gt_call *c, gt_slot *s) {
  int n = 0;
  s->types[n++] = &ffi_type_pointer; /* ctx */
  if (!s->destroy) {
    for (int i = 0; i < s->nargs; i++) s->types[n++] = type_of(s->args[i]);
    if (s->ret != R_NONE) s->types[n++] = &ffi_type_pointer;
    if (s->fallible) s->types[n++] = &ffi_type_pointer;
  }
  s->ntypes = n;
  ffi_type *rtype = s->destroy ? &ffi_type_void : &ffi_type_uint32;
  if (ffi_prep_cif(&s->cif, FFI_DEFAULT_ABI, (unsigned)n, rtype, s->types) != FFI_OK)
    return gt_fail(c, GUATIAO_ERR_INTERNAL, "libffi refused a slot's call interface");
  return 1;
}

static int int_at(const mxArray *a, size_t i, int *out) {
  if (!a || !(mxIsNumeric(a) || mxIsLogical(a)) || i >= mxGetNumberOfElements(a)) return 0;
  if (mxIsDouble(a)) *out = (int)((const double *)mxGetData(a))[i];
  else if (mxIsLogical(a)) *out = ((const mxLogical *)mxGetData(a))[i] ? 1 : 0;
  else if (mxGetClassID(a) == mxINT32_CLASS) *out = ((const int32_t *)mxGetData(a))[i];
  else return 0;
  return 1;
}

/* kind_new(name, floor_hash, object, args, rets, fallible, required,
 * arg_kinds, ret_kinds) -> id. `args` is a cell of shape-code vectors, one
 * per slot; `arg_kinds` a cell of id vectors (0 where no kind); `ret_kinds`
 * an id per slot (0 where the answer is no object). Kept for the session:
 * tables built from it are. */
int cmd_kind_new(gt_call *c) {
  guatiao_str name;
  double hash = 0;
  int obj = 0;
  if (!gt_arg_text(c, 0, "the kind's name", &name) || !gt_arg_double(c, 1, "the floor hash", &hash) ||
      !gt_arg_bool(c, 2, "object", &obj))
    return 0;
  const mxArray *args = gt_arg(c, 3), *rets = gt_arg(c, 4), *fallible = gt_arg(c, 5), *required = gt_arg(c, 6);
  const mxArray *arg_kinds = gt_arg(c, 7), *ret_kinds = gt_arg(c, 8);
  if (!args || !mxIsCell(args) || !arg_kinds || !mxIsCell(arg_kinds))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a kind's slots are cells");
  int user = (int)mxGetNumberOfElements(args);
  int n = user + (obj ? 1 : 0);
  if (n > 4096) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "too many slots");
  gt_kinds_init();
  gt_kind *k = (gt_kind *)calloc(1, sizeof(gt_kind));
  gt_slot *slots = (gt_slot *)calloc((size_t)(n ? n : 1), sizeof(gt_slot));
  char *name_copy = (char *)malloc(name.len + 1);
  if (!k || !slots || !name_copy) {
    free(k);
    free(slots);
    free(name_copy);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  memcpy(name_copy, name.ptr, name.len);
  name_copy[name.len] = '\0';
  k->name = name_copy;
  k->floor_hash = (uint32_t)hash;
  k->object = obj;
  k->nslots = n;
  k->size = 8 + sizeof(void *) * (size_t)n;
  k->slots = slots;
  int at = 0;
  if (obj) {
    slots[0].destroy = 1;
    slots[0].required = 1;
    slots[0].kind = k;
    slots[0].index_m = 0;
    at = 1;
  }
  int ok = 1;
  for (int i = 0; ok && i < user; i++, at++) {
    gt_slot *s = &slots[at];
    const mxArray *a = mxGetCell(args, (mwIndex)i);
    const mxArray *ak = mxGetCell(arg_kinds, (mwIndex)i);
    s->nargs = a ? (int)mxGetNumberOfElements(a) : 0;
    if (s->nargs > GT_MAX_ARGS) {
      ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a slot takes at most %d arguments", GT_MAX_ARGS);
      break;
    }
    for (int j = 0; ok && j < s->nargs; j++) {
      int code = 0, kid = 0;
      if (!int_at(a, (size_t)j, &code) || !known_arg(code)) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "an unknown argument shape");
      s->args[j] = code;
      if (ok && (code == S_LENT_OBJECT || code == S_TAKEN_OBJECT)) {
        if (!int_at(ak, (size_t)j, &kid) || !(s->arg_kind[j] = (gt_kind *)gt_handle_get((uint64_t)kid, GT_H_KIND)))
          ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "an object shape names its kind");
        s->arg_kind_id[j] = (uint64_t)kid;
      }
    }
    int ret = 0, rk = 0;
    if (ok && (!int_at(rets, (size_t)i, &ret) || !known_ret(ret))) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "an unknown answer shape");
    s->ret = ret;
    if (ok && ret == R_OBJECT) {
      if (!int_at(ret_kinds, (size_t)i, &rk) || !(s->ret_kind = (gt_kind *)gt_handle_get((uint64_t)rk, GT_H_KIND)))
        ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "an object answer names its kind");
      s->ret_kind_id = (uint64_t)rk;
    }
    int f = 0, r = 0;
    int_at(fallible, (size_t)i, &f);
    int_at(required, (size_t)i, &r);
    s->fallible = f;
    s->required = r;
    s->kind = k;
    s->index_m = i + 1;
  }
  for (int i = 0; ok && i < n; i++) ok = prep_slot(c, &slots[i]);
  if (!ok) {
    free(slots);
    free(name_copy);
    free(k);
    return 0;
  }
  k->id = gt_handle_add(GT_H_KIND, k);
  if (!k->id) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  gt_answer(c, 0, mxCreateDoubleScalar((double)k->id));
  return 1;
}

int cmd_table_floor_hash(gt_call *c) {
  void *t = NULL;
  if (!gt_mx_addr(gt_arg(c, 0), &t) || !t) return gt_fail(c, GUATIAO_ERR_NULL, "no table");
  gt_answer(c, 0, mxCreateDoubleScalar((double)((const guatiao_kind_header *)t)->floor_hash));
  return 1;
}

/* ---- remotes ---------------------------------------------------------------------- */

void gt_destroy_object(const void *table, size_t size, void *ctx) {
  if (!table || size < 16) return;
  void (*destroy)(void *) = *(void (*const *)(void *))((const char *)table + 8);
  if (destroy) destroy(ctx);
}

static uint64_t remote_add(gt_kind *k, const void *table, size_t size, void *ctx, int owned) {
  gt_remote *r = (gt_remote *)calloc(1, sizeof(gt_remote));
  if (!r) return 0;
  r->kind = k;
  r->table = table;
  r->size = size;
  r->ctx = ctx;
  r->owned = owned && k->object;
  r->live = 1;
  uint64_t id = gt_handle_add(GT_H_REMOTE, r);
  if (!id) free(r);
  return id;
}

/* remote_new(kind, table, size, ctx, owned) -> id */
int cmd_remote_new(gt_call *c) {
  gt_kind *k = NULL;
  void *table = NULL, *ctx = NULL;
  double size = 0;
  int owned = 0;
  if (!gt_arg_handle(c, 0, GT_H_KIND, "kind", (void **)&k)) return 0;
  if (!gt_mx_addr(gt_arg(c, 1), &table) || !gt_mx_addr(gt_arg(c, 3), &ctx))
    return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a table and a ctx are addresses");
  if (!table) return gt_fail(c, GUATIAO_ERR_NULL, "no table");
  if (!gt_arg_double(c, 2, "the size", &size) || !gt_arg_bool(c, 4, "owned", &owned)) return 0;
  uint64_t id = remote_add(k, table, (size_t)size, ctx, owned);
  if (!id) {
    if (owned && k->object) gt_destroy_object(table, (size_t)size, ctx);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

static int live_remote(gt_call *c, int i, gt_remote **out) {
  if (!gt_arg_handle(c, i, GT_H_REMOTE, "remote", (void **)out)) return 0;
  if (!(*out)->live) return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the remote is closed");
  return 1;
}

/* The slot's function in a table, or NULL when the table predates it. */
static void *slot_fn(const gt_remote *r, int at) {
  size_t off = 8 + sizeof(void *) * (size_t)at;
  if (off + sizeof(void *) > r->size) return NULL;
  return *(void *const *)((const char *)r->table + off);
}

static int c_index(const gt_kind *k, int i) { return k->object ? i : i - 1; }

int cmd_remote_has(gt_call *c) {
  gt_remote *r = NULL;
  double i = 0;
  if (!live_remote(c, 0, &r) || !gt_arg_double(c, 1, "the slot", &i)) return 0;
  int at = c_index(r->kind, (int)i);
  if (at < 0 || at >= r->kind->nslots) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "no such slot");
  gt_answer(c, 0, mxCreateLogicalScalar(slot_fn(r, at) != NULL));
  return 1;
}

/* remote_close(id): destroys an owned object once; the entry goes. */
int cmd_remote_close(gt_call *c) {
  uint64_t id = 0;
  if (!gt_arg_id(c, 0, &id)) {
    c->status = GUATIAO_OK;
    return 1;
  }
  gt_remote *r = (gt_remote *)gt_handle_take(id, GT_H_REMOTE);
  if (!r) return 1;
  if (r->live && r->owned) gt_destroy_object(r->table, r->size, r->ctx);
  free(r);
  return 1;
}

int cmd_remote_live(gt_call *c) {
  uint64_t id = 0;
  gt_remote *r = gt_arg_id(c, 0, &id) ? (gt_remote *)gt_handle_get(id, GT_H_REMOTE) : NULL;
  c->status = GUATIAO_OK;
  gt_answer(c, 0, mxCreateLogicalScalar(r && r->live));
  return 1;
}

/* ---- scalars -------------------------------------------------------------------- */

typedef union arg_store {
  guatiao_str s;
  guatiao_bytes b;
  guatiao_bytes_mut bm;
  guatiao_object o;
  uint8_t u8;
  int8_t i8;
  uint16_t u16;
  int16_t i16;
  uint32_t u32;
  int32_t i32;
  uint64_t u64;
  int64_t i64;
  float f32;
  double f64;
  void *p;
} arg_store;

typedef union out_store {
  guatiao_string s;
  guatiao_value v;
  guatiao_object o;
  uint8_t u8;
  int8_t i8;
  uint16_t u16;
  int16_t i16;
  uint32_t u32;
  int32_t i32;
  uint64_t u64;
  int64_t i64;
  float f32;
  double f64;
} out_store;

/* A scalar from MATLAB into its C width; NULL, or a message on refusal. */
static const char *scalar_in(int shape, const mxArray *x, arg_store *a) {
  if (!x || mxGetNumberOfElements(x) != 1 || !(mxIsNumeric(x) || mxIsLogical(x)) || mxIsComplex(x))
    return shape == S_BOOL ? "want true or false" : "want a number";
  if (shape == S_BOOL) {
    if (!mxIsLogical(x)) return "want true or false";
    a->u8 = *(const mxLogical *)mxGetData(x) ? 1 : 0;
    return NULL;
  }
  mxClassID id = mxGetClassID(x);
  if (id == mxINT64_CLASS || id == mxUINT64_CLASS) {
    /* Exact, whatever the width asks. */
    int neg = id == mxINT64_CLASS && *(const int64_t *)mxGetData(x) < 0;
    uint64_t mag = id == mxINT64_CLASS ? (neg ? (uint64_t)(-(*(const int64_t *)mxGetData(x) + 1)) + 1
                                              : (uint64_t) * (const int64_t *)mxGetData(x))
                                       : *(const uint64_t *)mxGetData(x);
    switch (shape) {
    case S_I64:
    case S_INTPTR:
      if (!neg && mag > 9223372036854775807ULL) return "out of range for int64";
      a->i64 = neg ? (int64_t)(0 - mag) : (int64_t)mag;
      return NULL;
    case S_U64:
    case S_SIZE:
      if (neg) return "out of range for uint64";
      a->u64 = mag;
      return NULL;
    case S_F32: a->f32 = neg ? -(float)mag : (float)mag; return NULL;
    case S_F64: a->f64 = neg ? -(double)mag : (double)mag; return NULL;
    default: break; /* narrower widths: through a double, which holds them */
    }
  }
  double d = mxGetScalar(x);
  if (isnan(d)) return "want a number, not NaN";
  if (shape == S_F32) {
    a->f32 = (float)d;
    return NULL;
  }
  if (shape == S_F64) {
    a->f64 = d;
    return NULL;
  }
  if (d != floor(d)) return "want a whole number";
  switch (shape) {
  case S_I8: if (d < -128 || d > 127) return "out of range for int8"; a->i8 = (int8_t)d; return NULL;
  case S_U8: if (d < 0 || d > 255) return "out of range for uint8"; a->u8 = (uint8_t)d; return NULL;
  case S_I16: if (d < -32768 || d > 32767) return "out of range for int16"; a->i16 = (int16_t)d; return NULL;
  case S_U16: if (d < 0 || d > 65535) return "out of range for uint16"; a->u16 = (uint16_t)d; return NULL;
  case S_I32: if (d < -2147483648.0 || d > 2147483647.0) return "out of range for int32"; a->i32 = (int32_t)d; return NULL;
  case S_U32: if (d < 0 || d > 4294967295.0) return "out of range for uint32"; a->u32 = (uint32_t)d; return NULL;
  case S_I64:
  case S_INTPTR:
    if (d < -9223372036854775808.0 || d >= 9223372036854775808.0) return "out of range for int64";
    a->i64 = (int64_t)d;
    return NULL;
  case S_U64:
  case S_SIZE:
    if (d < 0 || d >= 18446744073709551616.0) return "out of range for uint64";
    a->u64 = (uint64_t)d;
    return NULL;
  default: return "not a scalar";
  }
}

/* A 64-bit integer as a double up to 2^53, past it as int64 or uint64. */
static mxArray *wide_out(int is_signed, int64_t sv, uint64_t uv) {
  if (is_signed) {
    if (sv >= -9007199254740992LL && sv <= 9007199254740992LL) return mxCreateDoubleScalar((double)sv);
    mxArray *x = mxCreateNumericMatrix(1, 1, mxINT64_CLASS, mxREAL);
    *(int64_t *)mxGetData(x) = sv;
    return x;
  }
  if (uv <= 9007199254740992ULL) return mxCreateDoubleScalar((double)uv);
  mxArray *x = mxCreateNumericMatrix(1, 1, mxUINT64_CLASS, mxREAL);
  *(uint64_t *)mxGetData(x) = uv;
  return x;
}

static mxArray *scalar_out(int shape, const void *p) {
  switch (shape) {
  case S_BOOL: return mxCreateLogicalScalar(*(const uint8_t *)p ? true : false);
  case S_I8: return mxCreateDoubleScalar(*(const int8_t *)p);
  case S_U8: return mxCreateDoubleScalar(*(const uint8_t *)p);
  case S_I16: return mxCreateDoubleScalar(*(const int16_t *)p);
  case S_U16: return mxCreateDoubleScalar(*(const uint16_t *)p);
  case S_I32: return mxCreateDoubleScalar(*(const int32_t *)p);
  case S_U32: return mxCreateDoubleScalar((double)*(const uint32_t *)p);
  case S_I64:
  case S_INTPTR: return wide_out(1, *(const int64_t *)p, 0);
  case S_U64:
  case S_SIZE: return wide_out(0, 0, *(const uint64_t *)p);
  case S_F32: return mxCreateDoubleScalar((double)*(const float *)p);
  case S_F64: return mxCreateDoubleScalar(*(const double *)p);
  default: return mxCreateDoubleMatrix(0, 0, mxREAL);
  }
}

static int tree_tag(int shape) {
  switch (shape % 10) {
  case 1: return GUATIAO_MAP;
  case 2: return GUATIAO_LIST;
  default: return -1; /* any value */
  }
}

/* The pointer a tree shape passes for a node: the node, or its map or list. */
static void *tree_pointer(int shape, guatiao_value *v, const char **why) {
  int want = tree_tag(shape);
  if (want < 0) return v;
  if ((int)v->tag != want) {
    *why = want == GUATIAO_MAP ? "want a map" : "want a list";
    return NULL;
  }
  return want == GUATIAO_MAP ? (void *)&v->payload.map : (void *)&v->payload.list;
}

/* A tree answered by a slot, moved into the binding's allocator so it
 * outlives the library: a slot that ignored the carrier built it in its
 * own, wholly or in part. A core older than guatiao_value_make_local
 * copies a tree whose top container records another allocator. */
static guatiao_status into_binding(guatiao_value *v) {
  if (gt_core.guatiao_value_make_local) return gt_core.guatiao_value_make_local(v, &gt_binding_alloc);
  const guatiao_alloc *a = NULL;
  switch (v->tag) {
  case GUATIAO_MAP: a = v->payload.map.alloc; break;
  case GUATIAO_LIST: a = v->payload.list.alloc; break;
  case GUATIAO_STRING:
  case GUATIAO_NUMBER: a = v->payload.text.alloc; break;
  case GUATIAO_BYTES: a = v->payload.bytes.alloc; break;
  default: return GUATIAO_OK;
  }
  if (!a || a == &gt_binding_alloc) return GUATIAO_OK;
  guatiao_value copy;
  set_absent(&copy);
  guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, v, &copy);
  if (st == GUATIAO_OK) {
    gt_free(v);
    *v = copy;
  }
  return st;
}

/* The allocator a container node records when it can build, else `fallback`. */
static const guatiao_alloc *recorded_alloc(const guatiao_value *node, const guatiao_alloc *fallback) {
  const guatiao_alloc *a = NULL;
  switch (node->tag) {
  case GUATIAO_MAP: a = node->payload.map.alloc; break;
  case GUATIAO_LIST: a = node->payload.list.alloc; break;
  case GUATIAO_STRING:
  case GUATIAO_NUMBER: a = node->payload.text.alloc; break;
  case GUATIAO_BYTES: a = node->payload.bytes.alloc; break;
  default: break;
  }
  if (a && a->struct_size >= offsetof(guatiao_alloc, free) + sizeof a->free && a->alloc && a->free) return a;
  return fallback;
}

/* What a slot answered and nobody will read, freed: a refused settle
 * makes the call fail after the slot answered. */
static void drop_answer(const gt_slot *sl, out_store *out) {
  guatiao_value v;
  switch (sl->ret) {
  case S_STRING:
    set_absent(&v);
    v.tag = GUATIAO_STRING;
    v.payload.text = out->s;
    gt_free(&v);
    break;
  case S_VALUE:
  case S_MAP:
  case S_LIST:
    v = out->v;
    if (sl->ret == S_MAP) v.tag = GUATIAO_MAP;
    if (sl->ret == S_LIST) v.tag = GUATIAO_LIST;
    gt_free(&v);
    break;
  case R_OBJECT:
    if (out->o.table) gt_destroy_object(out->o.table, out->o.size, out->o.ctx);
    break;
  default: break;
  }
}

static int remote_answer(gt_call *c, const gt_slot *sl, out_store *outs) {
  out_store out = *outs;
  mxArray *x = NULL;
  switch (sl->ret) {
  case R_NONE: return 1;
  case S_STRING: {
    guatiao_value v;
    set_absent(&v);
    v.tag = GUATIAO_STRING;
    v.payload.text = out.s;
    int ok = gt_text_out(c, guatiao_string_text(&v), &x);
    gt_free(&v);
    if (!ok) return 0;
    break;
  }
  case S_VALUE:
  case S_MAP:
  case S_LIST: {
    guatiao_value v = out.v;
    if (sl->ret == S_MAP) v.tag = GUATIAO_MAP;
    if (sl->ret == S_LIST) v.tag = GUATIAO_LIST;
    guatiao_status moved = into_binding(&v);
    if (moved != GUATIAO_OK) {
      gt_free(&v);
      return gt_fail(c, moved, "moving the answer into this binding's allocator");
    }
    uint64_t id = gt_owner_new(&v, &gt_binding_alloc);
    if (!id) {
      gt_free(&v);
      return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    }
    x = mxCreateDoubleScalar((double)id);
    break;
  }
  case R_OBJECT: {
    if (!out.o.table) return gt_fail(c, GUATIAO_ERR_NULL, "the slot answered no object");
    uint64_t id = remote_add(sl->ret_kind, out.o.table, out.o.size, out.o.ctx, 1);
    if (!id) {
      gt_destroy_object(out.o.table, out.o.size, out.o.ctx);
      return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    }
    x = mxCreateDoubleScalar((double)id);
    break;
  }
  default: x = scalar_out(sl->ret, &out); break;
  }
  gt_answer(c, 0, x);
  return 1;
}

/* An object argument as the M side passes it: a struct with `remote` (the
 * id of a remote of the kind) or `table`, `size`, `ctx` (an M object served
 * by a table of the kind). */
static int object_arg(gt_call *c, const mxArray *x, int j, int taken, guatiao_object *o, gt_remote **rr) {
  *rr = NULL;
  memset(o, 0, sizeof *o);
  if (!x || !mxIsStruct(x) || mxGetNumberOfElements(x) != 1)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d: want an object of the kind", j + 1);
  const mxArray *rid = mxGetField(x, 0, "remote");
  if (rid && !mxIsEmpty(rid)) {
    double d = mxGetScalar(rid);
    gt_remote *r = (gt_remote *)gt_handle_get((uint64_t)d, GT_H_REMOTE);
    if (!r || !r->live) return gt_fail(c, GUATIAO_ERR_GONE, "argument %d: the remote is closed", j + 1);
    if (taken && !r->owned)
      return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "argument %d: only a remote this side owns is handed over", j + 1);
    o->table = r->table;
    o->size = r->size;
    o->ctx = r->ctx;
    *rr = r;
    return 1;
  }
  void *t = NULL, *ctx = NULL;
  const mxArray *size = mxGetField(x, 0, "size");
  if (!gt_mx_addr(mxGetField(x, 0, "table"), &t) || !gt_mx_addr(mxGetField(x, 0, "ctx"), &ctx) || !t || !size)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d: want an object of the kind", j + 1);
  o->table = t;
  o->size = (size_t)mxGetScalar(size);
  o->ctx = ctx;
  return 1;
}

/* remote_call(remote, slot, args) -> [answer, buffers]. A refusal before
 * the call leaves everything as it was; a status the callee answered has
 * the detail "provider". */
int cmd_remote_call(gt_call *c) {
  gt_remote *r = NULL;
  double idx = 0;
  if (!GT_NEED_CORE(c, guatiao_value_clone) || !live_remote(c, 0, &r) || !gt_arg_double(c, 1, "the slot", &idx)) return 0;
  const mxArray *args = gt_arg(c, 2);
  gt_kind *k = r->kind;
  int i = (int)idx;
  int at = c_index(k, i);
  if (at < (k->object ? 1 : 0) || at >= k->nslots) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "no such slot");
  gt_slot *sl = &k->slots[at];
  void *fn = slot_fn(r, at);
  if (!fn) return gt_fail(c, GUATIAO_ERR_UNSUPPORTED, "the table has no %s slot %d: built before it, or left out", k->name, i);
  if (!args || !mxIsCell(args) || (int)mxGetNumberOfElements(args) != sl->nargs)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the slot takes %d arguments", sl->nargs);

  arg_store store[GT_MAX_ARGS];
  guatiao_value tmp[GT_MAX_ARGS];
  int built[GT_MAX_ARGS];
  guatiao_value *lent[GT_MAX_ARGS]; /* a lent tree of the caller's, settled once the call returns */
  const guatiao_alloc *lent_home[GT_MAX_ARGS];
  mxArray *refs[GT_MAX_ARGS];
  guatiao_object objs[GT_MAX_ARGS];
  gt_remote *obj_remote[GT_MAX_ARGS];
  void *values[GT_MAX_ARGS + 3];
  memset(store, 0, sizeof store);
  memset(built, 0, sizeof built);
  memset(lent, 0, sizeof lent);
  memset(refs, 0, sizeof refs);
  for (int j = 0; j < sl->nargs; j++) set_absent(&tmp[j]);
  int ok = 1;
  int has_buffers = 0;

  /* First what can fail or run M code: text, scalars, trees built from data. */
  for (int j = 0; ok && j < sl->nargs; j++) {
    const mxArray *x = mxGetCell(args, (mwIndex)j);
    int s = sl->args[j];
    if (s == S_STRING) {
      guatiao_str t;
      if (!x || !gt_mx_utf8(c, x, "a text argument", &t)) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d: want text", j + 1);
      else store[j].s = t;
    } else if (s == S_BYTES || s == S_LENT_BYTES) {
      if (!x || mxGetClassID(x) != mxUINT8_CLASS) {
        ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d: want %s", j + 1, s == S_BYTES ? "uint8 bytes" : "a guatiao.Buffer");
      } else {
        size_t n = mxGetNumberOfElements(x);
        uint8_t *p = (uint8_t *)gt_scratch(c, n ? n : 1);
        if (!p) ok = 0;
        else {
          if (n) memcpy(p, mxGetData(x), n);
          if (s == S_BYTES) store[j].b = guatiao_bytes_from(n ? p : NULL, n);
          else {
            /* A copy of the buffer's bytes, written and handed back. */
            store[j].bm.ptr = n ? p : NULL;
            store[j].bm.len = n;
            has_buffers = 1;
          }
        }
      }
    } else if (s >= S_BOOL && s <= S_INTPTR) {
      const char *why = scalar_in(s, x, &store[j]);
      if (why) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d: %s", j + 1, why);
    } else if (s == S_LENT_OBJECT || s == S_TAKEN_OBJECT) {
      ok = object_arg(c, x, j, s == S_TAKEN_OBJECT, &objs[j], &obj_remote[j]);
    } else {
      if (!x) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "argument %d is missing", j + 1);
      else if (!gt_node_ref(c, x, &refs[j])) ok = 0;
      else if (!refs[j]) {
        if (!gt_build(c, x, &gt_binding_alloc, &tmp[j])) ok = 0;
        else built[j] = 1;
      }
    }
  }
  /* Then the nodes, found after any M code ran. */
  int moving[GT_MAX_ARGS];
  memset(moving, 0, sizeof moving);
  for (int j = 0; ok && j < sl->nargs; j++) {
    int s = sl->args[j];
    if (!((s >= S_VALUE && s <= S_LIST) || (s >= S_LENT_VALUE && s <= S_LENT_LIST) || (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST)))
      continue;
    guatiao_value *node = &tmp[j];
    const guatiao_alloc *owner_alloc = &gt_binding_alloc;
    if (refs[j]) {
      const mxArray *steps = mxGetCell(refs[j], 2);
      double oid = mxGetScalar(mxGetCell(refs[j], 1));
      gt_owner *o = (gt_owner *)gt_handle_get((uint64_t)oid, GT_H_OWNER);
      if (!gt_walk_mx(c, mxGetCell(refs[j], 1), steps, &node)) {
        ok = 0;
        break;
      }
      owner_alloc = o->alloc; /* found: the walk refuses a closed owner */
      if (s >= S_LENT_VALUE && s <= S_LENT_LIST && (o->flags & GT_READONLY)) {
        ok = gt_fail(c, GUATIAO_ERR_UNSUPPORTED, "argument %d: a read-only view is not lent", j + 1);
        break;
      }
      if (s >= S_TAKEN_VALUE) {
        moving[j] = (o->flags & GT_OWNED) && (!steps || mxIsEmpty(steps));
        if (!moving[j]) {
          /* A view is handed over as a copy; its value stays the caller's. */
          guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, node, &tmp[j]);
          if (st) {
            ok = gt_fail(c, st, "argument %d: the copy was refused", j + 1);
            break;
          }
          built[j] = 1;
          node = &tmp[j];
        }
      }
    }
    const char *why = NULL;
    void *p = tree_pointer(s, node, &why);
    if (!p) {
      ok = gt_fail(c, GUATIAO_ERR_WRONG_KIND, "argument %d: %s", j + 1, why);
      break;
    }
    store[j].p = p;
    if (s >= S_LENT_VALUE && s <= S_LENT_LIST && refs[j]) {
      lent[j] = node;
      lent_home[j] = recorded_alloc(node, owner_alloc);
    }
  }
  if (!ok) {
    for (int j = 0; j < sl->nargs; j++) {
      gt_free(&tmp[j]);
      if (refs[j]) mxDestroyArray(refs[j]);
    }
    return 0;
  }
  /* Nothing below fails before the call: what is handed over crosses now. */
  guatiao_object lent_objects[GT_MAX_ARGS];
  for (int j = 0; j < sl->nargs; j++) {
    int s = sl->args[j];
    if (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST && moving[j]) {
      /* The value's own root moves: it holds absent afterwards. */
      gt_owner *o = (gt_owner *)gt_handle_get((uint64_t)mxGetScalar(mxGetCell(refs[j], 1)), GT_H_OWNER);
      tmp[j] = *o->root;
      set_absent(o->root);
      store[j].p = tree_tag(s) < 0 ? (void *)&tmp[j]
                   : tree_tag(s) == GUATIAO_MAP ? (void *)&tmp[j].payload.map
                                                : (void *)&tmp[j].payload.list;
    }
    if (s == S_LENT_OBJECT || s == S_TAKEN_OBJECT) {
      if (s == S_TAKEN_OBJECT) {
        store[j].o = objs[j];
        if (obj_remote[j]) {
          /* Ownership crosses with the call. */
          obj_remote[j]->live = 0;
          obj_remote[j]->owned = 0;
        }
      } else {
        lent_objects[j] = objs[j];
        store[j].p = &lent_objects[j];
      }
    }
    if (refs[j]) {
      mxDestroyArray(refs[j]);
      refs[j] = NULL;
    }
  }

  void *ctx = r->ctx;
  int n = 0;
  values[n++] = &ctx;
  for (int j = 0; j < sl->nargs; j++) values[n++] = &store[j];
  out_store out;
  memset(&out, 0, sizeof out);
  void *outp = &out;
  if (sl->ret == S_VALUE) {
    out.v.payload.map.alloc = &gt_binding_alloc; /* the carrier: an ABSENT value naming our allocator */
  } else if (sl->ret == S_MAP || sl->ret == S_LIST) {
    out.v.payload.map.alloc = &gt_binding_alloc;
    outp = &out.v.payload.map;
  }
  if (sl->ret != R_NONE) values[n++] = &outp;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_provider_error *errp = &err;
  if (sl->fallible) values[n++] = &errp;
  ffi_arg rc = 0;
  ffi_call(&sl->cif, FFI_FN(fn), &rc, values);
  guatiao_status st = (guatiao_status)rc;
  /* What the callee grew a lent tree with may be in its library's
   * allocator: moved into the one each recorded when it was lent. The
   * first refusal is the error of a call that answered. */
  guatiao_status settled = GUATIAO_OK;
  if (gt_core.guatiao_value_make_local)
    for (int j = 0; j < sl->nargs; j++)
      if (lent[j]) {
        guatiao_status moved = gt_core.guatiao_value_make_local(lent[j], lent_home[j]);
        if (moved != GUATIAO_OK && settled == GUATIAO_OK) settled = moved;
      }
  for (int j = 0; j < sl->nargs; j++) {
    int s = sl->args[j];
    if (s < S_TAKEN_VALUE && built[j]) gt_free(&tmp[j]); /* what was handed over is the callee's */
  }
  if (st) {
    gt_fail_err(c, st, &err, NULL);
    snprintf(c->detail, sizeof c->detail, "provider");
    return 0;
  }
  gt_err_free(&err);
  if (settled) {
    drop_answer(sl, &out);
    return gt_fail(c, settled, "moving what slot %d of %s put into a lent tree into that tree's allocator",
                   sl->index_m, sl->kind->name);
  }
  if (!remote_answer(c, sl, &out)) return 0;
  if (has_buffers) {
    mxArray *bufs = mxCreateCellMatrix(1, (mwSize)sl->nargs);
    for (int j = 0; j < sl->nargs; j++) {
      if (sl->args[j] != S_LENT_BYTES) continue;
      mxArray *b = mxCreateNumericMatrix(1, (mwSize)store[j].bm.len, mxUINT8_CLASS, mxREAL);
      if (store[j].bm.len) memcpy(mxGetData(b), store[j].bm.ptr, store[j].bm.len);
      mxSetCell(bufs, (mwIndex)j, b);
    }
    gt_answer(c, 1, bufs);
  }
  return 1;
}

/* ---- serving M objects ------------------------------------------------------------- */

/* Ids a library's thread asked to destroy, released on the interpreter's
 * thread later. */
static double *pending = NULL;
static size_t pending_len = 0, pending_cap = 0;

void gt_queue_destroy(double id) {
  gt_lock();
  if (pending_len == pending_cap) {
    size_t cap = pending_cap ? pending_cap * 2 : 64;
    double *grown = (double *)realloc(pending, cap * sizeof(double));
    if (grown) {
      pending = grown;
      pending_cap = cap;
    }
  }
  if (pending_len < pending_cap) pending[pending_len++] = id;
  gt_unlock();
}

int cmd_pending_destroys(gt_call *c) {
  gt_lock();
  size_t n = pending_len;
  mxArray *out = mxCreateDoubleMatrix(1, (mwSize)n, mxREAL);
  if (n) memcpy(mxGetData(out), pending, n * sizeof(double));
  pending_len = 0;
  gt_unlock();
  gt_answer(c, 0, out);
  return 1;
}

double gt_id_of_ctx(const void *ctx) {
  uintptr_t v = (uintptr_t)ctx;
  if (v < 65536 || (v - 65536) % 16) return -1;
  return (double)((v - 65536) / 16);
}

/* A status and words written into a provider error, from any thread. */
void gt_write_err(guatiao_provider_error *err, guatiao_status st, const char *msg) {
  if (!err) return;
  err->status = st;
  memset(&err->message, 0, sizeof err->message);
  size_t n = msg ? strlen(msg) : 0;
  if (!n) return;
  uint8_t *p = (uint8_t *)gt_malloc(n);
  if (!p) return;
  memcpy(p, msg, n);
  err->message.ptr = p;
  err->message.len = n;
  err->message.cap = n;
  err->message.alloc = &gt_binding_alloc;
}

const char *const GT_WRONG_THREAD = "M code runs only on the interpreter's thread, inside a call into guatiao";

int gt_may_run_m(void) { return gt_on_main_thread() && gt_gateway_depth() > 0; }

int gt_call_m_quiet(const char *fn, int nout, mxArray **out, int nin, mxArray **in, char *msg, size_t msglen) {
  gt_call tmp;
  memset(&tmp, 0, sizeof tmp);
  int ok = gt_call_m(&tmp, fn, nout, out, nin, in);
  if (!ok && msglen) {
    /* Cut to the caller's buffer: the words are for a person. */
    size_t n = strlen(tmp.msg);
    if (n >= msglen) n = msglen - 1;
    memcpy(msg, tmp.msg, n);
    msg[n] = '\0';
  }
  gt_scratch_release(&tmp);
  return ok;
}

/* What a refused call was handed is let go: trees freed, objects destroyed. */
static void free_handed(const gt_slot *sl, void **args) {
  for (int j = 0; j < sl->nargs; j++) {
    int s = sl->args[j];
    if (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST) {
      void *p = *(void **)args[1 + j];
      if (!p || !gt_core.guatiao_value_free) continue;
      guatiao_value v;
      set_absent(&v);
      if (tree_tag(s) < 0) {
        v = *(guatiao_value *)p;
      } else if (tree_tag(s) == GUATIAO_MAP) {
        v.tag = GUATIAO_MAP;
        v.payload.map = *(guatiao_map *)p;
      } else {
        v.tag = GUATIAO_LIST;
        v.payload.list = *(guatiao_list *)p;
      }
      gt_core.guatiao_value_free(&v);
    } else if (s == S_TAKEN_OBJECT) {
      guatiao_object o = *(guatiao_object *)args[1 + j];
      gt_destroy_object(o.table, o.size, o.ctx);
    }
  }
}

typedef struct served_job {
  gt_slot *slot;
  void **args;
  void *ctx;
  guatiao_status status;
  char msg[1024];
  int lent_tag[GT_MAX_ARGS]; /* a lent map or list copied into lent_tmp */
  guatiao_value lent_tmp[GT_MAX_ARGS];
  guatiao_value view_tmp[GT_MAX_ARGS]; /* a borrowed map or list, as a node */
  uint64_t lent_ids[GT_MAX_ARGS];      /* owners and remotes lent for the call */
  uint64_t handed_ids[GT_MAX_ARGS];    /* owners and remotes of what was handed over */
  gt_pending answer;
  uint64_t answer_id;
} served_job;

/* What was handed over and filed for the dispatcher, let go when the call
 * is refused before the dispatcher took it: trees freed, objects destroyed. */
static void release_handed(served_job *j) {
  for (int k = 0; k < j->slot->nargs; k++) {
    uint64_t id = j->handed_ids[k];
    if (!id) continue;
    j->handed_ids[k] = 0;
    if (j->slot->args[k] == S_TAKEN_OBJECT) {
      gt_remote *r = (gt_remote *)gt_handle_take(id, GT_H_REMOTE);
      if (r && r->live && r->owned) gt_destroy_object(r->table, r->size, r->ctx);
      free(r);
    } else {
      gt_owner *o = (gt_owner *)gt_handle_take(id, GT_H_OWNER);
      if (o) gt_owner_drop(o);
    }
  }
}

/* What the handed-over argument k holds, as a node: absent for none. */
static guatiao_value handed_tree(const gt_slot *sl, void **args, int k) {
  int s = sl->args[k];
  void *p = *(void **)args[1 + k];
  guatiao_value v;
  set_absent(&v);
  if (!p) return v;
  if (tree_tag(s) < 0) {
    v = *(guatiao_value *)p;
  } else if (tree_tag(s) == GUATIAO_MAP) {
    v.tag = GUATIAO_MAP;
    v.payload.map = *(guatiao_map *)p;
  } else {
    v.tag = GUATIAO_LIST;
    v.payload.list = *(guatiao_list *)p;
  }
  return v;
}

/* The arguments as the dispatcher reads them: text, bytes, numbers, and
 * the ids of owners and remotes the dispatcher wraps. */
static int served_args(served_job *j, mxArray **out, char *msg, size_t msglen) {
  gt_slot *sl = j->slot;
  mxArray *args = mxCreateCellMatrix(1, (mwSize)sl->nargs);
  /* Every handed-over argument first, trees and objects, and every one of
   * them past a refusal too: the call owns all of it, and nothing below
   * can leave one without an owner. */
  guatiao_status refused = GUATIAO_OK;
  for (int k = 0; k < sl->nargs; k++) {
    int s = sl->args[k];
    if (s == S_TAKEN_OBJECT) {
      guatiao_object o = *(guatiao_object *)j->args[1 + k];
      uint64_t id = refused ? 0 : remote_add(sl->arg_kind[k], o.table, o.size, o.ctx, 1);
      if (!id) {
        gt_destroy_object(o.table, o.size, o.ctx);
        if (!refused) {
          refused = GUATIAO_ERR_ALLOC;
          snprintf(msg, msglen, "out of memory");
        }
        continue;
      }
      j->handed_ids[k] = id;
      mxSetCell(args, (mwIndex)k, mxCreateDoubleScalar((double)id));
      continue;
    }
    if (s < S_TAKEN_VALUE || s > S_TAKEN_LIST) continue;
    if (!*(void **)j->args[1 + k]) {
      if (!refused) {
        refused = GUATIAO_ERR_NULL;
        snprintf(msg, msglen, "the handed-over %s is null",
                 tree_tag(s) == GUATIAO_MAP ? "map" : tree_tag(s) == GUATIAO_LIST ? "list" : "value");
      }
      continue;
    }
    guatiao_value v = handed_tree(sl, j->args, k);
    if (refused) {
      gt_free(&v);
      continue;
    }
    /* The method owns it and may keep it: moved out of the caller's
     * allocators, which may go with the caller's library. */
    guatiao_status moved = into_binding(&v);
    if (moved != GUATIAO_OK) {
      gt_free(&v);
      refused = moved;
      snprintf(msg, msglen, "a handed-over tree could not be moved into this binding's allocator");
      continue;
    }
    uint64_t id = gt_owner_new(&v, &gt_binding_alloc);
    if (!id) {
      gt_free(&v);
      refused = GUATIAO_ERR_ALLOC;
      snprintf(msg, msglen, "out of memory");
      continue;
    }
    j->handed_ids[k] = id;
    mxSetCell(args, (mwIndex)k, mxCreateDoubleScalar((double)id));
  }
  if (refused) {
    j->status = refused;
    release_handed(j);
    mxDestroyArray(args);
    return 0;
  }
  for (int k = 0; k < sl->nargs; k++) {
    int s = sl->args[k];
    void *a = j->args[1 + k];
    mxArray *x = NULL;
    if (s == S_STRING || s == S_BYTES) {
      guatiao_str t = *(guatiao_str *)a;
      if (s == S_STRING) {
        x = gt_utf8_mx(t);
        if (!x) {
          j->status = GUATIAO_ERR_BAD_VALUE;
          snprintf(msg, msglen, "argument %d is not UTF-8", k + 1);
          release_handed(j);
          mxDestroyArray(args);
          return 0;
        }
      } else {
        x = mxCreateNumericMatrix(1, (mwSize)t.len, mxUINT8_CLASS, mxREAL);
        if (t.len) memcpy(mxGetData(x), t.ptr, t.len);
      }
    } else if (s == S_LENT_BYTES) {
      guatiao_bytes_mut bm = *(guatiao_bytes_mut *)a;
      x = mxCreateNumericMatrix(1, (mwSize)bm.len, mxUINT8_CLASS, mxREAL);
      if (bm.len) memcpy(mxGetData(x), bm.ptr, bm.len);
    } else if (s >= S_BOOL && s <= S_INTPTR) {
      x = scalar_out(s, a);
    } else if ((s >= S_VALUE && s <= S_LIST) || (s >= S_LENT_VALUE && s <= S_LENT_LIST)) {
      void *p = *(void **)a;
      int lent = s >= S_LENT_VALUE;
      guatiao_value *node;
      if (!p) {
        set_absent(&j->view_tmp[k]);
        j->view_tmp[k].tag = GUATIAO_NULL;
        node = &j->view_tmp[k];
      } else if (tree_tag(s) < 0) {
        node = (guatiao_value *)p;
      } else {
        guatiao_value *t = lent ? &j->lent_tmp[k] : &j->view_tmp[k];
        set_absent(t);
        t->tag = (uint32_t)tree_tag(s);
        if (t->tag == GUATIAO_MAP) t->payload.map = *(guatiao_map *)p;
        else t->payload.list = *(guatiao_list *)p;
        if (lent) j->lent_tag[k] = (int)t->tag;
        node = t;
      }
      uint64_t id = gt_owner_borrow(node, &gt_binding_alloc, !lent);
      if (!id) {
        snprintf(msg, msglen, "out of memory");
        release_handed(j);
        mxDestroyArray(args);
        return 0;
      }
      j->lent_ids[k] = id;
      x = mxCreateDoubleScalar((double)id);
    } else if ((s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST) || s == S_TAKEN_OBJECT) {
      continue;
    } else if (s == S_LENT_OBJECT) {
      guatiao_object o = **(guatiao_object **)a;
      uint64_t id = remote_add(sl->arg_kind[k], o.table, o.size, o.ctx, 0);
      if (!id) {
        snprintf(msg, msglen, "out of memory");
        release_handed(j);
        mxDestroyArray(args);
        return 0;
      }
      j->lent_ids[k] = id;
      x = mxCreateDoubleScalar((double)id);
    }
    mxSetCell(args, (mwIndex)k, x);
  }
  *out = args;
  return 1;
}

/* Calls guatiao.internal.dispatch(kind, slot, ctx, args, answer) and reads
 * {status, message, buffers}. */
static void served_body(served_job *j) {
  gt_slot *sl = j->slot;
  mxArray *args = NULL;
  if (!served_args(j, &args, j->msg, sizeof j->msg)) return;
  j->answer_id = gt_handle_add(GT_H_ANSWER, &j->answer);
  double ctx = gt_id_of_ctx(j->ctx) < 0 ? -1 : (double)(uintptr_t)j->ctx;
  mxArray *in[5] = {mxCreateDoubleScalar((double)sl->kind->id), mxCreateDoubleScalar((double)sl->index_m),
                    mxCreateDoubleScalar(ctx), args, mxCreateDoubleScalar((double)j->answer_id)};
  mxArray *res = NULL;
  char msg[1024];
  int ok = gt_call_m_quiet("guatiao.internal.dispatch", 1, &res, 5, in, msg, sizeof msg);
  for (int i = 0; i < 5; i++) mxDestroyArray(in[i]);
  if (!ok) {
    j->status = GUATIAO_ERR_INTERNAL;
    snprintf(j->msg, sizeof j->msg, "%s", msg);
    return;
  }
  if (!res || !mxIsCell(res) || mxGetNumberOfElements(res) < 2) {
    j->status = GUATIAO_ERR_INTERNAL;
    snprintf(j->msg, sizeof j->msg, "the dispatcher answered something else");
    if (res) mxDestroyArray(res);
    return;
  }
  j->status = (guatiao_status)mxGetScalar(mxGetCell(res, 0));
  const mxArray *m = mxGetCell(res, 1);
  j->msg[0] = '\0';
  if (m && mxIsChar(m)) {
    gt_call tmp;
    memset(&tmp, 0, sizeof tmp);
    guatiao_str t;
    if (gt_mx_utf8(&tmp, m, "a message", &t)) snprintf(j->msg, sizeof j->msg, "%.*s", (int)t.len, t.ptr);
    gt_scratch_release(&tmp);
  }
  /* A lent buffer is written back with what the method left in it. */
  const mxArray *bufs = mxGetNumberOfElements(res) > 2 ? mxGetCell(res, 2) : NULL;
  for (int k = 0; k < sl->nargs && bufs && mxIsCell(bufs) && (int)mxGetNumberOfElements(bufs) == sl->nargs; k++) {
    if (sl->args[k] != S_LENT_BYTES) continue;
    guatiao_bytes_mut bm = *(guatiao_bytes_mut *)j->args[1 + k];
    const mxArray *data = mxGetCell(bufs, (mwIndex)k);
    if (data && mxGetClassID(data) == mxUINT8_CLASS && bm.len) {
      size_t n = mxGetNumberOfElements(data) < bm.len ? mxGetNumberOfElements(data) : bm.len;
      memcpy(bm.ptr, mxGetData(data), n);
    }
  }
  mxDestroyArray(res);
}

/* Clears what M was lent, and writes lent maps and lists back. */
static void served_settle(served_job *j) {
  gt_slot *sl = j->slot;
  for (int k = 0; k < sl->nargs; k++) {
    if (!j->lent_ids[k]) continue;
    if (sl->args[k] == S_LENT_OBJECT) free(gt_handle_take(j->lent_ids[k], GT_H_REMOTE));
    else gt_owner_end(j->lent_ids[k]);
  }
  for (int k = 0; k < sl->nargs; k++) {
    if (!j->lent_tag[k]) continue;
    void *p = *(void **)j->args[1 + k];
    if (j->lent_tag[k] == GUATIAO_MAP) *(guatiao_map *)p = j->lent_tmp[k].payload.map;
    else *(guatiao_list *)p = j->lent_tmp[k].payload.list;
  }
  if (j->answer_id) gt_handle_take(j->answer_id, GT_H_ANSWER);
  j->answer.live = 0;
}

static guatiao_provider_error *err_arg(const gt_slot *sl, void **args) {
  if (!sl->fallible) return NULL;
  return *(guatiao_provider_error **)args[1 + sl->nargs + (sl->ret != R_NONE ? 1 : 0)];
}

static void *out_arg(const gt_slot *sl, void **args) {
  if (sl->ret == R_NONE) return NULL;
  return *(void **)args[1 + sl->nargs];
}

static void served_slot(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  gt_slot *sl = (gt_slot *)user;
  guatiao_provider_error *err = err_arg(sl, args);
  if (err) gt_write_err(err, GUATIAO_OK, NULL);
  if (!gt_may_run_m()) {
    free_handed(sl, args);
    gt_write_err(err, GUATIAO_ERR_UNSUPPORTED, GT_WRONG_THREAD);
    *(ffi_arg *)ret = GUATIAO_ERR_UNSUPPORTED;
    return;
  }
  served_job *j = (served_job *)calloc(1, sizeof(served_job));
  if (!j) {
    free_handed(sl, args);
    *(ffi_arg *)ret = GUATIAO_ERR_ALLOC;
    return;
  }
  j->slot = sl;
  j->args = args;
  j->ctx = *(void **)args[0];
  j->status = GUATIAO_ERR_INTERNAL;
  j->answer.live = 1;
  j->answer.ret = sl->ret;
  j->answer.out = out_arg(sl, args);
  served_body(j);
  served_settle(j);
  guatiao_status st = j->status;
  if (st && err) gt_write_err(err, st, j->msg);
  else if (st) fprintf(stderr, "guatiao: a slot that cannot fail failed: %s\n", j->msg);
  free(j);
  *(ffi_arg *)ret = st;
}

static void served_destroy(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)ret;
  (void)user;
  double id = gt_id_of_ctx(*(void **)args[0]);
  if (id < 0) return;
  if (!gt_may_run_m()) {
    gt_queue_destroy(id);
    return;
  }
  mxArray *in[1] = {mxCreateDoubleScalar(id)};
  char msg[512];
  if (!gt_call_m_quiet("guatiao.internal.release_id", 0, NULL, 1, in, msg, sizeof msg))
    fprintf(stderr, "guatiao: releasing an object failed: %s\n", msg);
  mxDestroyArray(in[0]);
}

/* A closure over the served handler for one slot, made once a session. */
static void *closure_code(gt_call *c, gt_slot *sl) {
  if (sl->code) return sl->code;
  void *code = NULL;
  ffi_closure *cl = (ffi_closure *)ffi_closure_alloc(sizeof(ffi_closure), &code);
  if (!cl) {
    gt_fail(c, GUATIAO_ERR_ALLOC, "libffi could not allocate a closure");
    return NULL;
  }
  if (ffi_prep_closure_loc(cl, &sl->cif, sl->destroy ? served_destroy : served_slot, sl, code) != FFI_OK) {
    ffi_closure_free(cl);
    gt_fail(c, GUATIAO_ERR_INTERNAL, "libffi refused a closure");
    return NULL;
  }
  sl->closure = cl;
  sl->code = code;
  return code;
}

/* served_table(kind, has) -> the address of a table serving `kind` with a
 * closure for each slot `has` marks; kept for the session, as hosts may
 * hold it. */
int cmd_served_table(gt_call *c) {
  gt_kind *k = NULL;
  if (!gt_arg_handle(c, 0, GT_H_KIND, "kind", (void **)&k)) return 0;
  const mxArray *has = gt_arg(c, 1);
  int user = k->nslots - (k->object ? 1 : 0);
  if (!has || (int)mxGetNumberOfElements(has) != user || !(mxIsLogical(has) || (user == 0)))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "one flag per slot");
  void *table = calloc(1, k->size);
  if (!table) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  /* The slots follow the 8-byte header, a pointer each. */
  void **slots = (void **)((char *)table + sizeof(guatiao_kind_header));
  gt_lock_gateway();
  guatiao_kind_header *h = (guatiao_kind_header *)table;
  h->struct_size = (uint32_t)k->size;
  h->floor_hash = k->floor_hash;
  const mxLogical *flags = user ? (const mxLogical *)mxGetData(has) : NULL;
  for (int at = 0; at < k->nslots; at++) {
    gt_slot *sl = &k->slots[at];
    int wanted = sl->destroy || flags[sl->index_m - 1];
    if (wanted && !(slots[at] = closure_code(c, sl))) {
      free(table);
      return 0;
    }
  }
  gt_answer(c, 0, gt_addr_mx(table));
  return 1;
}

/* ---- answers -------------------------------------------------------------------------- */

static int write_scalar(gt_call *c, int shape, const mxArray *x, void *out) {
  arg_store a;
  const char *why = scalar_in(shape, x, &a);
  if (why) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the answer: %s", why);
  switch (shape) {
  case S_BOOL:
  case S_U8: *(uint8_t *)out = a.u8; break;
  case S_I8: *(int8_t *)out = a.i8; break;
  case S_I16: *(int16_t *)out = a.i16; break;
  case S_U16: *(uint16_t *)out = a.u16; break;
  case S_I32: *(int32_t *)out = a.i32; break;
  case S_U32: *(uint32_t *)out = a.u32; break;
  case S_I64:
  case S_INTPTR: *(int64_t *)out = a.i64; break;
  case S_U64:
  case S_SIZE: *(uint64_t *)out = a.u64; break;
  case S_F32: *(float *)out = a.f32; break;
  case S_F64: *(double *)out = a.f64; break;
  default: break;
  }
  return 1;
}

/* answer(id, result): writes a method's answer into the slot's out
 * parameter, in the allocator the carrier names. Called by the dispatcher
 * during the served call. */
int cmd_answer(gt_call *c) {
  gt_pending *ans = NULL;
  if (!gt_arg_handle(c, 0, GT_H_ANSWER, "call", (void **)&ans)) return 0;
  if (!ans->live) return gt_fail(c, GUATIAO_ERR_GONE, "the call has returned");
  const mxArray *result = gt_arg(c, 1);
  void *out = ans->out;
  if (ans->ret == R_NONE || !out || !result) return 1;
  if (ans->ret >= S_BOOL && ans->ret <= S_INTPTR) {
    if (!write_scalar(c, ans->ret, result, out)) return 0;
    ans->live = 0;
    return 1;
  }
  if (ans->ret == R_OBJECT) {
    guatiao_object o;
    gt_remote *rr = NULL;
    if (!object_arg(c, result, -1, 1, &o, &rr)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the answer is an object of the kind");
    if (rr) {
      rr->live = 0;
      rr->owned = 0;
    }
    *(guatiao_object *)out = o;
    ans->live = 0;
    return 1;
  }
  const guatiao_alloc *a = &gt_binding_alloc;
  if (ans->ret == S_VALUE) {
    const guatiao_alloc *carried = ((guatiao_value *)out)->payload.map.alloc;
    if (carried) a = carried;
  } else if (ans->ret == S_MAP || ans->ret == S_LIST) {
    const guatiao_alloc *carried = ((guatiao_map *)out)->alloc;
    if (carried) a = carried;
  }
  guatiao_value tmp;
  if (!gt_build(c, result, a, &tmp)) return 0;
  if (!ans->live) { /* building ran M code, which may have answered already */
    gt_free(&tmp);
    return gt_fail(c, GUATIAO_ERR_GONE, "the call has an answer");
  }
  switch (ans->ret) {
  case S_STRING:
    if (tmp.tag != GUATIAO_STRING) {
      gt_free(&tmp);
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the answer is text");
    }
    *(guatiao_string *)out = tmp.payload.text;
    break;
  case S_VALUE: *(guatiao_value *)out = tmp; break;
  case S_MAP:
  case S_LIST: {
    int want = ans->ret == S_MAP ? GUATIAO_MAP : GUATIAO_LIST;
    int other = want == GUATIAO_MAP ? GUATIAO_LIST : GUATIAO_MAP;
    int empty = (tmp.tag == (uint32_t)other) &&
                (other == GUATIAO_LIST ? guatiao_list_items(&tmp).len == 0 : guatiao_map_entries(&tmp).len == 0);
    if (empty || tmp.tag == GUATIAO_NULL) {
      /* [] or {} or struct(): the empty container the slot wants. */
      gt_free(&tmp);
      guatiao_status st = want == GUATIAO_MAP ? gt_core.guatiao_value_map(a, &tmp) : gt_core.guatiao_value_list(a, &tmp);
      if (st) return gt_fail(c, st, "the allocator refused");
    }
    if (tmp.tag != (uint32_t)want) {
      gt_free(&tmp);
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s", want == GUATIAO_MAP ? "the answer is a map (a struct)" : "the answer is a list (a cell)");
    }
    if (want == GUATIAO_MAP) *(guatiao_map *)out = tmp.payload.map;
    else *(guatiao_list *)out = tmp.payload.list;
    break;
  }
  default: gt_free(&tmp); return gt_fail(c, GUATIAO_ERR_INTERNAL, "an unknown answer shape");
  }
  ans->live = 0; /* one answer per call */
  return 1;
}
