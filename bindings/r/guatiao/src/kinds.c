/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Kind tables through libffi: a call interface per slot, built from the
 * shapes a kind declares; a remote calls a table's slot with ffi_call, and
 * an R object is served by a table of ffi closures over one handler.
 *
 * A served slot touches R only on R's main thread, and there only inside
 * R_ToplevelExec: an R error, an interrupt or a condition nobody handles
 * ends in that context and never unwinds through the closure and the
 * foreign frames between it and the R host's own .Call.
 */

#include <ffi.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"
#include "kinds.h"

/* ---- ffi types ------------------------------------------------------------ */

static ffi_type *view_elements[3];
static ffi_type view_type; /* guatiao_str, guatiao_bytes, guatiao_bytes_mut */
static ffi_type *object_elements[4];
static ffi_type object_type; /* guatiao_object */

void gt_kinds_init(void) {
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

/* ---- kinds --------------------------------------------------------------------- */

SEXP C_xptr_live(SEXP x) { return Rf_ScalarLogical(TYPEOF(x) == EXTPTRSXP && R_ExternalPtrAddr(x) != NULL); }

static gt_kind *kind_of(SEXP x) {
  if (TYPEOF(x) != EXTPTRSXP || !R_ExternalPtrAddr(x)) gt_raise(GUATIAO_ERR_WRONG_KIND, "not a kind");
  return (gt_kind *)R_ExternalPtrAddr(x);
}

static void prep_slot(gt_slot *s) {
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
    gt_raise(GUATIAO_ERR_INTERNAL, "libffi refused a slot's call interface");
}

/* A kind's C description, kept for the session: tables built from it are. */
SEXP C_kind_new(SEXP name, SEXP floor_hash, SEXP object, SEXP args, SEXP rets, SEXP fallible, SEXP required,
                SEXP arg_kinds, SEXP slot_names, SEXP arg_names, SEXP int64) {
  int obj = Rf_asLogical(object) == 1;
  int user = (int)XLENGTH(rets);
  int n = user + (obj ? 1 : 0);
  if (n > 4096) gt_raise(GUATIAO_ERR_BAD_VALUE, "too many slots");
  if (TYPEOF(slot_names) != STRSXP || XLENGTH(slot_names) != user || XLENGTH(arg_names) != user)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "a name per slot and per argument");
  size_t names_len = 0;
  for (int i = 0; i < user; i++) {
    SEXP a = VECTOR_ELT(args, i);
    SEXP an = VECTOR_ELT(arg_names, i);
    if (XLENGTH(a) > GT_MAX_ARGS) gt_raise(GUATIAO_ERR_BAD_VALUE, "a slot takes at most %d arguments", GT_MAX_ARGS);
    if (TYPEOF(an) != STRSXP || XLENGTH(an) != XLENGTH(a)) gt_raise(GUATIAO_ERR_BAD_VALUE, "a name per argument");
    names_len += strlen(CHAR(STRING_ELT(slot_names, i))) + 1;
    for (R_xlen_t j = 0; j < XLENGTH(an); j++) names_len += strlen(CHAR(STRING_ELT(an, j))) + 1;
    for (R_xlen_t j = 0; j < XLENGTH(a); j++)
      if (!known_arg(INTEGER(a)[j])) gt_raise(GUATIAO_ERR_BAD_VALUE, "an unknown argument shape");
    if (!known_ret(INTEGER(rets)[i])) gt_raise(GUATIAO_ERR_BAD_VALUE, "an unknown answer shape");
    SEXP ak = VECTOR_ELT(arg_kinds, i);
    for (R_xlen_t j = 0; j < XLENGTH(a); j++) {
      int code = INTEGER(a)[j];
      if ((code == S_LENT_OBJECT || code == S_TAKEN_OBJECT) && TYPEOF(VECTOR_ELT(ak, j)) != EXTPTRSXP)
        gt_raise(GUATIAO_ERR_BAD_VALUE, "an object shape names its kind");
    }
  }
  gt_kind *k = (gt_kind *)calloc(1, sizeof(gt_kind));
  gt_slot *slots = (gt_slot *)calloc((size_t)n, sizeof(gt_slot));
  const char *nm = Rf_translateCharUTF8(STRING_ELT(name, 0));
  char *name_copy = (char *)malloc(strlen(nm) + 1);
  char *names = (char *)malloc(names_len + 1);
  if (!k || !slots || !name_copy || !names) {
    free(k);
    free(slots);
    free(name_copy);
    free(names);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  strcpy(name_copy, nm);
  k->name = name_copy;
  k->names = names;
  k->int64 = Rf_asInteger(int64);
  k->floor_hash = (uint32_t)Rf_asReal(floor_hash);
  k->object = obj;
  k->nslots = n;
  k->size = 8 + sizeof(void *) * (size_t)n;
  k->slots = slots;
  k->spec = R_NilValue;
  int c = 0;
  if (obj) {
    slots[0].destroy = 1;
    slots[0].required = 1;
    slots[0].kind = k;
    slots[0].index_r = 0;
    slots[0].name = "close";
    c = 1;
  }
  char *at = names;
  for (int i = 0; i < user; i++, c++) {
    gt_slot *s = &slots[c];
    SEXP a = VECTOR_ELT(args, i);
    SEXP an = VECTOR_ELT(arg_names, i);
    s->nargs = (int)XLENGTH(a);
    strcpy(at, CHAR(STRING_ELT(slot_names, i)));
    s->name = at;
    at += strlen(at) + 1;
    SEXP ak = VECTOR_ELT(arg_kinds, i);
    for (int j = 0; j < s->nargs; j++) {
      strcpy(at, CHAR(STRING_ELT(an, j)));
      s->arg_names[j] = at;
      at += strlen(at) + 1;
      s->args[j] = INTEGER(a)[j];
      SEXP kp = VECTOR_ELT(ak, j);
      s->arg_kind[j] = TYPEOF(kp) == EXTPTRSXP ? (gt_kind *)R_ExternalPtrAddr(kp) : NULL;
    }
    s->ret = INTEGER(rets)[i];
    s->fallible = LOGICAL(fallible)[i] == 1;
    s->required = LOGICAL(required)[i] == 1;
    s->kind = k;
    s->index_r = i + 1;
  }
  for (int i = 0; i < n; i++) prep_slot(&slots[i]);
  return R_MakeExternalPtr(k, R_NilValue, R_NilValue);
}

SEXP C_table_floor_hash(SEXP table) {
  const guatiao_kind_header *h = (const guatiao_kind_header *)R_ExternalPtrAddr(table);
  if (!h) gt_raise(GUATIAO_ERR_NULL, "no table");
  return Rf_ScalarReal((double)h->floor_hash);
}

/* ---- remotes ---------------------------------------------------------------------- */

static void destroy_object(const void *table, size_t size, void *ctx) {
  if (!table || size < 16) return;
  void (*destroy)(void *) = *(void (*const *)(void *))((const char *)table + 8);
  if (destroy) destroy(ctx);
}

static void remote_finalize(SEXP x) {
  gt_remote *r = (gt_remote *)R_ExternalPtrAddr(x);
  if (!r) return;
  if (r->owned && r->live) destroy_object(r->table, r->size, r->ctx);
  free(r);
  R_ClearExternalPtr(x);
}

static SEXP remote_make(gt_kind *k, const void *table, size_t size, void *ctx, int owned) {
  gt_remote *r = (gt_remote *)calloc(1, sizeof(gt_remote));
  if (!r) {
    if (owned && k->object) destroy_object(table, size, ctx);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  r->kind = k;
  r->table = table;
  r->size = size;
  r->ctx = ctx;
  r->owned = owned && k->object;
  r->live = 1;
  SEXP x = PROTECT(R_MakeExternalPtr(r, R_NilValue, R_NilValue));
  R_RegisterCFinalizerEx(x, remote_finalize, TRUE);
  UNPROTECT(1);
  return x;
}

R_CFinalizer_t gt_remote_finalizer(void);
void gt_warn_unfallible(const gt_slot *sl, const char *msg);

static void *addr_of(SEXP x) {
  if (x == R_NilValue) return NULL;
  if (TYPEOF(x) == EXTPTRSXP) return R_ExternalPtrAddr(x);
  if (TYPEOF(x) == REALSXP && XLENGTH(x) == 1) return (void *)(uintptr_t)REAL(x)[0];
  gt_raise(GUATIAO_ERR_WRONG_KIND, "an address is an external pointer");
}

SEXP C_remote_new(SEXP kind, SEXP table, SEXP size, SEXP ctx, SEXP owned) {
  gt_kind *k = kind_of(kind);
  const void *t = addr_of(table);
  if (!t) gt_raise(GUATIAO_ERR_NULL, "no table");
  return remote_make(k, t, (size_t)Rf_asReal(size), addr_of(ctx), Rf_asLogical(owned) == 1);
}

static gt_remote *remote_of(SEXP x) {
  if (TYPEOF(x) != EXTPTRSXP) gt_raise(GUATIAO_ERR_WRONG_KIND, "not a remote");
  gt_remote *r = (gt_remote *)R_ExternalPtrAddr(x);
  if (!r || !r->live) {
    gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    Rf_error("closed");
  }
  return r;
}

/* The slot's function in a table, or NULL when the provider left it out
 * or the table is smaller than its kind's, which fills no slot. */
static void *slot_fn(const gt_remote *r, int c) {
  if (r->size < r->kind->size) return NULL;
  return *(void *const *)((const char *)r->table + 8 + sizeof(void *) * (size_t)c);
}

static int c_index(const gt_kind *k, int i) { return k->object ? i : i - 1; }

SEXP C_remote_has(SEXP handle, SEXP index) {
  gt_remote *r = remote_of(handle);
  int c = c_index(r->kind, Rf_asInteger(index));
  if (c < 0 || c >= r->kind->nslots) gt_raise(GUATIAO_ERR_NOT_FOUND, "no such slot");
  return Rf_ScalarLogical(slot_fn(r, c) != NULL);
}

SEXP C_remote_close(SEXP handle) {
  if (TYPEOF(handle) != EXTPTRSXP) return R_NilValue;
  gt_remote *r = (gt_remote *)R_ExternalPtrAddr(handle);
  if (!r || !r->live) return R_NilValue;
  r->live = 0;
  if (r->owned) destroy_object(r->table, r->size, r->ctx);
  r->owned = 0;
  return R_NilValue;
}

/* ---- calling a slot ------------------------------------------------------------- */

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

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* A scalar from R into its C width; 0 or a message on refusal. An integer
 * shape takes an R integer or logical, a whole double, an integer64, a
 * `number` or a string spelling an integer, read exactly and
 * range-checked. */
static const char *scalar_in(int shape, SEXP x, arg_store *a) {
  if (shape == S_BOOL) {
    if (!Rf_isLogical(x) || XLENGTH(x) != 1 || LOGICAL(x)[0] == NA_LOGICAL) return "want TRUE or FALSE";
    a->u8 = LOGICAL(x)[0] ? 1 : 0;
    return NULL;
  }
  if (shape == S_F32 || shape == S_F64) {
    if (!(Rf_isReal(x) || Rf_isInteger(x) || Rf_isLogical(x)) || XLENGTH(x) != 1) return "want a number";
    double d = Rf_asReal(x);
    if (gt_is_integer64(x)) {
      gt_wide w;
      const char *why = gt_wide_in(x, &w);
      if (why) return why;
      d = w.neg ? -(double)w.mag : (double)w.mag;
    }
    if (ISNAN(d)) return "want a number, not NA";
    if (shape == S_F32) a->f32 = (float)d;
    else a->f64 = d;
    return NULL;
  }
  gt_wide w;
  const char *why = gt_wide_in(x, &w);
  if (why) return why;
  int64_t s = 0;
  uint64_t u = 0;
  switch (shape) {
  case S_I8: why = gt_wide_signed(w, 8, &s); a->i8 = (int8_t)s; return why;
  case S_U8: why = gt_wide_unsigned(w, 8, &u); a->u8 = (uint8_t)u; return why;
  case S_I16: why = gt_wide_signed(w, 16, &s); a->i16 = (int16_t)s; return why;
  case S_U16: why = gt_wide_unsigned(w, 16, &u); a->u16 = (uint16_t)u; return why;
  case S_I32: why = gt_wide_signed(w, 32, &s); a->i32 = (int32_t)s; return why;
  case S_U32: why = gt_wide_unsigned(w, 32, &u); a->u32 = (uint32_t)u; return why;
  case S_I64:
  case S_INTPTR: why = gt_wide_signed(w, 64, &s); a->i64 = s; return why;
  case S_U64:
  case S_SIZE: why = gt_wide_unsigned(w, 64, &u); a->u64 = u; return why;
  default: return "not a scalar";
  }
}

/* A scalar as R holds it; a 64-bit integer as the `int64 =` mode says. */
static SEXP scalar_out(int shape, const void *p, int mode) {
  switch (shape) {
  case S_BOOL: return Rf_ScalarLogical(*(const uint8_t *)p ? 1 : 0);
  case S_I8: return Rf_ScalarInteger(*(const int8_t *)p);
  case S_U8: return Rf_ScalarInteger(*(const uint8_t *)p);
  case S_I16: return Rf_ScalarInteger(*(const int16_t *)p);
  case S_U16: return Rf_ScalarInteger(*(const uint16_t *)p);
  case S_I32: return Rf_ScalarInteger(*(const int32_t *)p);
  case S_U32: return Rf_ScalarReal((double)*(const uint32_t *)p);
  case S_I64:
  case S_INTPTR: return gt_i64_sexp(*(const int64_t *)p, mode);
  case S_U64:
  case S_SIZE: return gt_u64_sexp(*(const uint64_t *)p, mode);
  case S_F32: return Rf_ScalarReal((double)*(const float *)p);
  case S_F64: return Rf_ScalarReal(*(const double *)p);
  default: return R_NilValue;
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

/* Reads a status a callee answered with its words, freeing them, and
 * signals marking that the call happened. */
static void raise_called(guatiao_status st, guatiao_provider_error *err) {
  char buf[2048];
  size_t n = 0;
  if (err->message.len && err->message.ptr) {
    n = err->message.len < sizeof buf - 1 ? err->message.len : sizeof buf - 1;
    memcpy(buf, err->message.ptr, n);
  }
  buf[n] = '\0';
  guatiao_value m;
  set_absent(&m);
  m.tag = GUATIAO_STRING;
  m.payload.text = err->message;
  if (err->message.cap) gt_free(&m);
  SEXP s = PROTECT(Rf_ScalarReal((double)st));
  SEXP msg = PROTECT(Rf_allocVector(STRSXP, 1));
  SET_STRING_ELT(msg, 0, Rf_mkCharCE(buf, CE_UTF8));
  gt_call_r(".gt_raise_called", 2, s, msg, R_NilValue);
  UNPROTECT(2);
  Rf_error("unreachable");
}

/* A tree answered by a slot, moved into the binding's allocator so it
 * outlives the library: a slot that ignored the carrier built it in its
 * own, wholly or in part. */
static guatiao_status into_binding(guatiao_value *v) {
  return gt_core.guatiao_value_make_local(v, &gt_binding_alloc);
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
  if (a && a->struct_size >= sizeof(guatiao_alloc) && a->alloc && a->free) return a;
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
    if (out->o.table) destroy_object(out->o.table, out->o.size, out->o.ctx);
    break;
  default: break;
  }
}

static SEXP remote_answer(const gt_slot *sl, out_store *outs, int mode) {
  out_store out = *outs;
  switch (sl->ret) {
  case R_NONE: return R_NilValue;
  case S_STRING: {
    guatiao_value v;
    set_absent(&v);
    v.tag = GUATIAO_STRING;
    v.payload.text = out.s;
    return gt_take_text(&v);
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
      gt_raise(moved, "moving the answer into this binding's allocator");
    }
    SEXP owner = gt_owner_new(&v, &gt_binding_alloc, R_NilValue);
    if (!owner) {
      gt_free(&v);
      gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
    }
    return owner;
  }
  case R_OBJECT: {
    SEXP res = PROTECT(Rf_allocVector(VECSXP, 3));
    SET_VECTOR_ELT(res, 0, R_MakeExternalPtr((void *)out.o.table, R_NilValue, R_NilValue));
    SET_VECTOR_ELT(res, 1, Rf_ScalarReal((double)out.o.size));
    SET_VECTOR_ELT(res, 2, R_MakeExternalPtr(out.o.ctx, R_NilValue, R_NilValue));
    UNPROTECT(1);
    if (!out.o.table) gt_raise(GUATIAO_ERR_NULL, "the slot answered no object");
    return res;
  }
  default: return scalar_out(sl->ret, &out, mode);
  }
}

/* Why an argument was refused, naming the kind, the slot and the argument. */
static void arg_refused(char *msg, size_t n, const gt_slot *sl, int j, const char *why) {
  snprintf(msg, n, "%s.%s(%s): %s", sl->kind->name, sl->name, sl->arg_names[j], why);
}

SEXP C_remote_call(SEXP handle, SEXP index, SEXP args, SEXP int64) {
  gt_need(GT_LIB_CORE);
  gt_remote *r = remote_of(handle);
  gt_kind *k = r->kind;
  int i = Rf_asInteger(index);
  int c = c_index(k, i);
  if (c < (k->object ? 1 : 0) || c >= k->nslots) gt_raise(GUATIAO_ERR_NOT_FOUND, "no such slot");
  gt_slot *sl = &k->slots[c];
  void *fn = slot_fn(r, c);
  if (!fn && r->size < k->size)
    gt_raise(GUATIAO_ERR_WRONG_KIND, "the %s table is %d bytes, smaller than the kind's %d", k->name,
             (int)r->size, (int)k->size);
  if (!fn) gt_raise(GUATIAO_ERR_UNSUPPORTED, "the table leaves out the %s slot %d", k->name, i);
  if (XLENGTH(args) != sl->nargs)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "%s.%s takes %d arguments", k->name, sl->name, sl->nargs);

  arg_store store[GT_MAX_ARGS];
  guatiao_value tmp[GT_MAX_ARGS];
  int built[GT_MAX_ARGS];
  guatiao_value *lent[GT_MAX_ARGS]; /* a lent tree of the caller's, settled once the call returns */
  const guatiao_alloc *lent_home[GT_MAX_ARGS];
  void *values[GT_MAX_ARGS + 3];
  memset(store, 0, sizeof store);
  memset(built, 0, sizeof built);
  memset(lent, 0, sizeof lent);
  for (int j = 0; j < sl->nargs; j++) set_absent(&tmp[j]);
  char msg[1024];
  msg[0] = '\0';
  guatiao_status fail = GUATIAO_OK;
  int has_buffers = 0;
  for (int j = 0; j < sl->nargs; j++) has_buffers |= sl->args[j] == S_LENT_BYTES;
  SEXP bufs = PROTECT(Rf_allocVector(VECSXP, sl->nargs));

  /* First what can fail or run R code: text, scalars, trees built from R data. */
  for (int j = 0; j < sl->nargs && !fail; j++) {
    SEXP x = VECTOR_ELT(args, j);
    int s = sl->args[j];
    if (s == S_STRING) {
      if (TYPEOF(x) != STRSXP || XLENGTH(x) != 1 || STRING_ELT(x, 0) == NA_STRING) {
        fail = GUATIAO_ERR_BAD_VALUE;
        arg_refused(msg, sizeof msg, sl, j, "want one string");
        break;
      }
      const char *p = Rf_translateCharUTF8(STRING_ELT(x, 0));
      store[j].s = guatiao_str_from(p, strlen(p));
    } else if (s == S_BYTES) {
      if (TYPEOF(x) != RAWSXP) {
        fail = GUATIAO_ERR_BAD_VALUE;
        arg_refused(msg, sizeof msg, sl, j, "want a raw vector");
        break;
      }
      store[j].b = guatiao_bytes_from(XLENGTH(x) ? RAW(x) : NULL, (size_t)XLENGTH(x));
    } else if (s == S_LENT_BYTES) {
      /* The buffer's bytes, copied: the copy is written and handed back. */
      if (TYPEOF(x) != RAWSXP) {
        fail = GUATIAO_ERR_BAD_VALUE;
        arg_refused(msg, sizeof msg, sl, j, "want a byte_buffer()");
        break;
      }
      SEXP data = Rf_duplicate(x);
      SET_VECTOR_ELT(bufs, j, data);
      store[j].bm.ptr = XLENGTH(data) ? RAW(data) : NULL;
      store[j].bm.len = (size_t)XLENGTH(data);
    } else if (s >= S_BOOL && s <= S_INTPTR) {
      const char *why = scalar_in(s, x, &store[j]);
      if (why) {
        fail = GUATIAO_ERR_BAD_VALUE;
        arg_refused(msg, sizeof msg, sl, j, why);
        break;
      }
    } else if (s == S_LENT_OBJECT || s == S_TAKEN_OBJECT) {
      /* list(table, size, ctx, id, handle), made on the R side */
      if (TYPEOF(x) != VECSXP || XLENGTH(x) != 5) {
        fail = GUATIAO_ERR_BAD_VALUE;
        arg_refused(msg, sizeof msg, sl, j, "want an object");
        break;
      }
      SEXP handle = VECTOR_ELT(x, 4);
      if (TYPEOF(handle) == EXTPTRSXP) {
        gt_remote *rr = (gt_remote *)R_ExternalPtrAddr(handle);
        if (!rr || !rr->live) {
          fail = GUATIAO_ERR_GONE;
          arg_refused(msg, sizeof msg, sl, j, "the remote is closed");
          break;
        }
        if (s == S_TAKEN_OBJECT && !rr->owned) {
          fail = GUATIAO_ERR_WRONG_KIND;
          arg_refused(msg, sizeof msg, sl, j, "only a remote this side owns is handed over");
          break;
        }
      }
    } else if (!Rf_inherits(x, "guatiao_node")) {
      char why[512];
      fail = gt_build(x, &gt_binding_alloc, &tmp[j], why, sizeof why);
      if (fail) {
        arg_refused(msg, sizeof msg, sl, j, why);
        break;
      }
      built[j] = 1;
    }
  }
  /* Then the nodes, found after any R code ran. */
  for (int j = 0; j < sl->nargs && !fail; j++) {
    SEXP x = VECTOR_ELT(args, j);
    int s = sl->args[j];
    if (!((s >= S_VALUE && s <= S_LIST) || (s >= S_LENT_VALUE && s <= S_LENT_LIST) || (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST)))
      continue;
    guatiao_value *node = &tmp[j];
    gt_owner *o = NULL;
    if (!built[j]) {
      int got = gt_node_try(x, &o, &node);
      if (got) {
        fail = got == 1 ? GUATIAO_ERR_GONE : GUATIAO_ERR_NOT_FOUND;
        arg_refused(msg, sizeof msg, sl, j, got == 1 ? "the value is closed" : "the view no longer addresses a node");
        break;
      }
      if (s >= S_LENT_VALUE && (o->flags & GT_READONLY)) {
        fail = GUATIAO_ERR_UNSUPPORTED;
        arg_refused(msg, sizeof msg, sl, j, "a read-only view is not lent");
        break;
      }
      if (s >= S_TAKEN_VALUE) {
        int moving = (o->flags & GT_OWNED) && XLENGTH(VECTOR_ELT(x, 1)) == 0;
        if (!moving) {
          /* A view is handed over as a copy; its value stays the caller's. */
          guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, node, &tmp[j]);
          if (st) {
            fail = st;
            arg_refused(msg, sizeof msg, sl, j, "the copy was refused");
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
      fail = GUATIAO_ERR_WRONG_KIND;
      arg_refused(msg, sizeof msg, sl, j, why);
      break;
    }
    store[j].p = p;
    if (s >= S_LENT_VALUE && s <= S_LENT_LIST && !built[j]) {
      lent[j] = node;
      lent_home[j] = recorded_alloc(node, o->alloc);
    }
  }
  if (fail) {
    for (int j = 0; j < sl->nargs; j++) gt_free(&tmp[j]);
    UNPROTECT(1);
    gt_raise(fail, "%s", msg);
  }
  /* Nothing below fails: what is handed over crosses now. */
  guatiao_object lent_objects[GT_MAX_ARGS];
  for (int j = 0; j < sl->nargs; j++) {
    SEXP x = VECTOR_ELT(args, j);
    int s = sl->args[j];
    if (s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST && !built[j]) {
      /* The value's own root moves: it holds absent afterwards. */
      gt_owner *o = gt_owner_of(VECTOR_ELT(x, 0));
      tmp[j] = *o->root;
      set_absent(o->root);
      store[j].p = tree_tag(s) < 0 ? (void *)&tmp[j] : tree_tag(s) == GUATIAO_MAP ? (void *)&tmp[j].payload.map : (void *)&tmp[j].payload.list;
    }
    if (s == S_LENT_OBJECT || s == S_TAKEN_OBJECT) {
      guatiao_object o = {NULL, 0, NULL};
      SEXP handle = VECTOR_ELT(x, 4);
      if (TYPEOF(handle) == EXTPTRSXP) {
        gt_remote *rr = (gt_remote *)R_ExternalPtrAddr(handle);
        if (rr && rr->live) {
          o.table = rr->table;
          o.size = rr->size;
          o.ctx = rr->ctx;
          if (s == S_TAKEN_OBJECT) {
            /* Ownership crosses with the call. */
            rr->live = 0;
            rr->owned = 0;
          }
        }
      } else {
        o.table = addr_of(VECTOR_ELT(x, 0));
        o.size = (size_t)Rf_asReal(VECTOR_ELT(x, 1));
        o.ctx = addr_of(VECTOR_ELT(x, 2));
      }
      if (s == S_TAKEN_OBJECT) {
        store[j].o = o;
      } else {
        lent_objects[j] = o;
        store[j].p = &lent_objects[j];
      }
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
  for (int j = 0; j < sl->nargs; j++)
    if (lent[j]) {
      /* A lent map also loses a key the callee wrote that is not UTF-8. */
      guatiao_status moved = sl->args[j] == S_LENT_MAP ? gt_core.guatiao_map_settle_lent(lent[j], lent_home[j])
                                                       : gt_core.guatiao_value_make_local(lent[j], lent_home[j]);
      if (moved != GUATIAO_OK && settled == GUATIAO_OK) settled = moved;
    }
  for (int j = 0; j < sl->nargs; j++) {
    int s = sl->args[j];
    if (s < S_TAKEN_VALUE && built[j]) gt_free(&tmp[j]); /* what was handed over is the callee's */
  }
  if (st) raise_called(st, &err);
  if (err.message.cap) {
    guatiao_value m;
    set_absent(&m);
    m.tag = GUATIAO_STRING;
    m.payload.text = err.message;
    gt_free(&m);
  }
  if (settled) {
    drop_answer(sl, &out);
    UNPROTECT(1);
    if (settled == GUATIAO_ERR_BAD_VALUE)
      gt_raise(settled, "%s.%s wrote a key that is not UTF-8 into the map it was lent; the entry was removed",
               sl->kind->name, sl->name);
    gt_raise(settled, "moving what %s.%s put into a lent tree into that tree's allocator", sl->kind->name, sl->name);
  }
  SEXP result = PROTECT(remote_answer(sl, &out, Rf_asInteger(int64)));
  if (has_buffers) {
    SEXP both = PROTECT(Rf_allocVector(VECSXP, 2));
    SET_VECTOR_ELT(both, 0, result);
    SET_VECTOR_ELT(both, 1, bufs);
    UNPROTECT(3);
    return both;
  }
  UNPROTECT(2);
  return result;
}

/* ---- serving R objects ------------------------------------------------------------- */

/* Ids a library's thread asked to destroy, released on R's thread later. */
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

SEXP C_pending_destroys(void) {
  gt_lock();
  size_t n = pending_len;
  double *copy = n ? (double *)malloc(n * sizeof(double)) : NULL;
  if (copy) memcpy(copy, pending, n * sizeof(double));
  if (copy || !n) pending_len = 0;
  gt_unlock();
  if (n && !copy) return Rf_allocVector(REALSXP, 0);
  SEXP out = Rf_allocVector(REALSXP, (R_xlen_t)n);
  if (n) memcpy(REAL(out), copy, n * sizeof(double));
  free(copy);
  return out;
}

double gt_id_of_ctx(const void *ctx) {
  uintptr_t c = (uintptr_t)ctx;
  if (c < 65536 || (c - 65536) % 16) return -1;
  return (double)((c - 65536) / 16);
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

const char *const GT_WRONG_THREAD = "R code runs only on R's main thread";

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
      destroy_object(o.table, o.size, o.ctx);
    }
  }
}

typedef struct served_job {
  gt_slot *slot;
  void **args;
  void *ctx;
  guatiao_status status;
  char msg[1024];
  int lent_tag[GT_MAX_ARGS];          /* a lent map or list copied into lent_tmp */
  guatiao_value lent_tmp[GT_MAX_ARGS];
  guatiao_value view_tmp[GT_MAX_ARGS]; /* a borrowed map or list, as a node */
  gt_answer answer;
  SEXP keep; /* preserved: what the call lent R, cleared once it returns */
} served_job;

/* An object as a remote R holds, destroyed when R collects it if `owned`. */
static SEXP served_remote(const gt_slot *sl, int k, guatiao_object o, int owned) {
  gt_remote *r = (gt_remote *)calloc(1, sizeof(gt_remote));
  if (!r) return NULL;
  r->kind = sl->arg_kind[k];
  r->table = o.table;
  r->size = o.size;
  r->ctx = o.ctx;
  r->owned = owned;
  r->live = 1;
  SEXP x = PROTECT(R_MakeExternalPtr(r, R_NilValue, R_NilValue));
  R_RegisterCFinalizerEx(x, gt_remote_finalizer(), TRUE);
  UNPROTECT(1);
  return x;
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

/* The arguments as the dispatcher reads them, or R_NilValue for a call
 * refused while receiving what was handed over (j->status and j->msg say
 * why: answered without an R error, which would end the call as INTERNAL). */
static SEXP served_args(served_job *j) {
  gt_slot *sl = j->slot;
  SEXP args = PROTECT(Rf_allocVector(VECSXP, sl->nargs));
  /* Every handed-over argument first, trees and objects, and every one of
   * them past a refusal too: the call owns all of it, and nothing below
   * can leave one without an owner. */
  guatiao_status refused = GUATIAO_OK;
  for (int k = 0; k < sl->nargs; k++) {
    int s = sl->args[k];
    if (s == S_TAKEN_OBJECT) {
      guatiao_object o = *(guatiao_object *)j->args[1 + k];
      SEXP x = refused ? NULL : served_remote(sl, k, o, 1);
      if (!x) {
        destroy_object(o.table, o.size, o.ctx);
        if (!refused) {
          refused = GUATIAO_ERR_ALLOC;
          snprintf(j->msg, sizeof j->msg, "out of memory");
        }
        continue;
      }
      SET_VECTOR_ELT(args, k, x);
      continue;
    }
    if (s < S_TAKEN_VALUE || s > S_TAKEN_LIST) continue;
    if (!*(void **)j->args[1 + k]) {
      if (!refused) {
        refused = GUATIAO_ERR_NULL;
        snprintf(j->msg, sizeof j->msg, "the handed-over %s is null",
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
      snprintf(j->msg, sizeof j->msg, "a handed-over tree could not be moved into this binding's allocator");
      continue;
    }
    SEXP owner = gt_owner_new(&v, &gt_binding_alloc, R_NilValue);
    if (!owner) {
      gt_free(&v);
      refused = GUATIAO_ERR_ALLOC;
      snprintf(j->msg, sizeof j->msg, "out of memory");
      continue;
    }
    SET_VECTOR_ELT(args, k, owner);
  }
  if (refused) {
    j->status = refused;
    UNPROTECT(1);
    return R_NilValue;
  }
  for (int k = 0; k < sl->nargs; k++) {
    int s = sl->args[k];
    void *a = j->args[1 + k];
    SEXP x = R_NilValue;
    if (s == S_STRING || s == S_BYTES) {
      guatiao_str t = *(guatiao_str *)a;
      if (s == S_STRING) {
        SEXP ch = gt_mkchar(t);
        if (!ch) {
          j->status = GUATIAO_ERR_BAD_VALUE;
          snprintf(j->msg, sizeof j->msg, "text holding a NUL cannot be an R string");
          UNPROTECT(1);
          return R_NilValue;
        }
        x = Rf_ScalarString(ch);
      } else {
        x = Rf_allocVector(RAWSXP, (R_xlen_t)t.len);
        if (t.len) memcpy(RAW(x), t.ptr, t.len);
      }
    } else if (s == S_LENT_BYTES) {
      guatiao_bytes_mut bm = *(guatiao_bytes_mut *)a;
      SEXP data = PROTECT(Rf_allocVector(RAWSXP, (R_xlen_t)bm.len));
      if (bm.len) memcpy(RAW(data), bm.ptr, bm.len);
      x = gt_call_r(".gt_buffer_of", 1, data, R_NilValue, R_NilValue);
      UNPROTECT(1);
    } else if (s >= S_BOOL && s <= S_INTPTR) {
      x = scalar_out(s, a, sl->kind->int64);
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
      x = gt_owner_borrow(node, &gt_binding_alloc, !lent, R_NilValue);
      SET_VECTOR_ELT(j->keep, k, x);
    } else if ((s >= S_TAKEN_VALUE && s <= S_TAKEN_LIST) || s == S_TAKEN_OBJECT) {
      continue;
    } else if (s == S_LENT_OBJECT) {
      x = served_remote(sl, k, **(guatiao_object **)a, 0);
      if (!x) Rf_error("out of memory");
      SET_VECTOR_ELT(j->keep, k, x);
    }
    SET_VECTOR_ELT(args, k, x);
  }
  UNPROTECT(1);
  return args;
}

static void served_body(void *p) {
  served_job *j = (served_job *)p;
  gt_slot *sl = j->slot;
  SEXP keep = PROTECT(Rf_allocVector(VECSXP, sl->nargs + 1));
  R_PreserveObject(keep);
  j->keep = keep;
  UNPROTECT(1);
  SEXP out = R_MakeExternalPtr(&j->answer, R_NilValue, R_NilValue);
  SET_VECTOR_ELT(keep, sl->nargs, out);
  SEXP args = served_args(j);
  if (args == R_NilValue) return;
  PROTECT(args);
  SEXP f = PROTECT(Rf_install(".gt_dispatch"));
  SEXP ctx = PROTECT(Rf_ScalarReal(gt_id_of_ctx(j->ctx) < 0 ? -1 : (double)(uintptr_t)j->ctx));
  SEXP idx = PROTECT(Rf_ScalarInteger(sl->index_r));
  SEXP call = PROTECT(Rf_lang6(f, sl->kind->spec, idx, ctx, args, out));
  SEXP res = PROTECT(Rf_eval(call, gt_namespace()));
  j->status = (guatiao_status)Rf_asInteger(VECTOR_ELT(res, 0));
  SEXP m = VECTOR_ELT(res, 1);
  if (TYPEOF(m) == STRSXP && XLENGTH(m) == 1) snprintf(j->msg, sizeof j->msg, "%s", Rf_translateCharUTF8(STRING_ELT(m, 0)));
  /* A lent buffer is written back with what the method left in it. */
  SEXP bufs = XLENGTH(res) > 2 ? VECTOR_ELT(res, 2) : R_NilValue;
  for (int k = 0; k < sl->nargs && TYPEOF(bufs) == VECSXP && XLENGTH(bufs) == sl->nargs; k++) {
    if (sl->args[k] != S_LENT_BYTES) continue;
    guatiao_bytes_mut bm = *(guatiao_bytes_mut *)j->args[1 + k];
    SEXP data = VECTOR_ELT(bufs, k);
    if (TYPEOF(data) == RAWSXP && bm.len) {
      size_t n = (size_t)XLENGTH(data) < bm.len ? (size_t)XLENGTH(data) : bm.len;
      memcpy(bm.ptr, RAW(data), n);
    }
  }
  UNPROTECT(6);
}

/* Clears what R was lent, and writes lent maps and lists back. No R
 * allocation happens here: it runs after the protected evaluation. */
static void served_settle(served_job *j) {
  gt_slot *sl = j->slot;
  if (j->keep) {
    for (int k = 0; k < sl->nargs; k++) {
      SEXP x = VECTOR_ELT(j->keep, k);
      if (x == R_NilValue) continue;
      int s = sl->args[k];
      if (s == S_LENT_OBJECT) {
        gt_remote *r = (gt_remote *)R_ExternalPtrAddr(x);
        if (r) r->live = 0;
      } else {
        gt_owner_clear(x);
      }
    }
    R_ClearExternalPtr(VECTOR_ELT(j->keep, sl->nargs));
    R_ReleaseObject(j->keep);
    j->keep = NULL;
  }
  for (int k = 0; k < sl->nargs; k++) {
    if (!j->lent_tag[k]) continue;
    void *p = *(void **)j->args[1 + k];
    if (j->lent_tag[k] == GUATIAO_MAP) *(guatiao_map *)p = j->lent_tmp[k].payload.map;
    else *(guatiao_list *)p = j->lent_tmp[k].payload.list;
  }
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
  if (!gt_on_main_thread()) {
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
  snprintf(j->msg, sizeof j->msg, "an R error ended the call");
  j->answer.live = 1;
  j->answer.ret = sl->ret;
  j->answer.out = out_arg(sl, args);
  j->answer.slot = sl;
  if (!R_ToplevelExec(served_body, j)) {
    j->status = GUATIAO_ERR_INTERNAL;
    snprintf(j->msg, sizeof j->msg, "an R error ended the call");
  }
  served_settle(j);
  guatiao_status st = j->status;
  if (st && err) gt_write_err(err, st, j->msg);
  else if (st) gt_warn_unfallible(sl, j->msg);
  free(j);
  *(ffi_arg *)ret = st;
}

typedef struct destroy_job {
  double id;
} destroy_job;

static void destroy_body(void *p) {
  destroy_job *d = (destroy_job *)p;
  SEXP id = PROTECT(Rf_ScalarReal(d->id));
  SEXP f = PROTECT(Rf_install(".gt_release_id"));
  SEXP call = PROTECT(Rf_lang2(f, id));
  Rf_eval(call, gt_namespace());
  UNPROTECT(3);
}

static void served_destroy(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)ret;
  (void)user;
  double id = gt_id_of_ctx(*(void **)args[0]);
  if (id < 0) return;
  if (!gt_on_main_thread()) {
    gt_queue_destroy(id);
    return;
  }
  destroy_job d = {id};
  R_ToplevelExec(destroy_body, &d);
}

/* A closure over the served handler for one slot, made once a session. */
static void *closure_code(gt_slot *sl) {
  if (sl->code) return sl->code;
  void *code = NULL;
  ffi_closure *cl = (ffi_closure *)ffi_closure_alloc(sizeof(ffi_closure), &code);
  if (!cl) gt_raise(GUATIAO_ERR_ALLOC, "libffi could not allocate a closure");
  ffi_status st = ffi_prep_closure_loc(cl, &sl->cif, sl->destroy ? served_destroy : served_slot, sl, code);
  if (st != FFI_OK) {
    ffi_closure_free(cl);
    gt_raise(GUATIAO_ERR_INTERNAL, "libffi refused a closure");
  }
  sl->closure = cl;
  sl->code = code;
  return code;
}

/* A table serving `kind` with a closure for each slot `has` marks; kept
 * for the session, as hosts may hold it. */
SEXP C_served_table(SEXP kind, SEXP spec, SEXP has) {
  gt_kind *k = kind_of(kind);
  if (k->spec == R_NilValue) {
    R_PreserveObject(spec);
    k->spec = spec;
  }
  int user = k->nslots - (k->object ? 1 : 0);
  if (XLENGTH(has) != user) gt_raise(GUATIAO_ERR_BAD_VALUE, "one flag per slot");
  void *table = calloc(1, k->size);
  if (!table) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  guatiao_kind_header *h = (guatiao_kind_header *)table;
  /* The slots follow the 8-byte header, a pointer each. */
  void **slots = (void **)((char *)table + sizeof(guatiao_kind_header));
  h->struct_size = (uint32_t)k->size;
  h->floor_hash = k->floor_hash;
  for (int c = 0; c < k->nslots; c++) {
    gt_slot *sl = &k->slots[c];
    int wanted = sl->destroy || LOGICAL(has)[sl->index_r - 1] == 1;
    if (wanted) slots[c] = closure_code(sl);
  }
  return R_MakeExternalPtr(table, R_NilValue, R_NilValue);
}

/* ---- answers -------------------------------------------------------------------------- */

static void write_scalar(const gt_slot *sl, int shape, SEXP x, void *out) {
  arg_store a;
  const char *why = scalar_in(shape, x, &a);
  if (why) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s.%s answered: %s", sl->kind->name, sl->name, why);
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
}

/* Writes a method's answer into the slot's out parameter, in the allocator
 * the carrier names. Called from .gt_dispatch, inside the protected call. */
SEXP C_answer(SEXP answer, SEXP result) {
  gt_answer *ans = (gt_answer *)R_ExternalPtrAddr(answer);
  if (!ans || !ans->live) gt_raise(GUATIAO_ERR_GONE, "the call has returned");
  void *out = ans->out;
  if (ans->ret == R_NONE || !out) return R_NilValue;
  if (ans->ret >= S_BOOL && ans->ret <= S_INTPTR) {
    write_scalar(ans->slot, ans->ret, result, out);
    return R_NilValue;
  }
  if (ans->ret == R_OBJECT) {
    guatiao_object o;
    if (TYPEOF(result) != VECSXP) gt_raise(GUATIAO_ERR_BAD_VALUE, "the answer is an object of the kind");
    if (XLENGTH(result) >= 5 && TYPEOF(VECTOR_ELT(result, 4)) == EXTPTRSXP) {
      gt_remote *r = (gt_remote *)R_ExternalPtrAddr(VECTOR_ELT(result, 4));
      if (!r || !r->live || !r->owned) gt_raise(GUATIAO_ERR_BAD_VALUE, "only a remote this side owns is handed over");
      o.table = r->table;
      o.size = r->size;
      o.ctx = r->ctx;
      r->live = 0;
      r->owned = 0;
    } else {
      o.table = addr_of(VECTOR_ELT(result, 0));
      o.size = (size_t)Rf_asReal(VECTOR_ELT(result, 1));
      o.ctx = addr_of(VECTOR_ELT(result, 2));
    }
    *(guatiao_object *)out = o;
    return R_NilValue;
  }
  const guatiao_alloc *a = &gt_binding_alloc;
  if (ans->ret == S_VALUE) {
    const guatiao_alloc *c = ((guatiao_value *)out)->payload.map.alloc;
    if (c) a = c;
  } else if (ans->ret == S_MAP || ans->ret == S_LIST) {
    const guatiao_alloc *c = ((guatiao_map *)out)->alloc;
    if (c) a = c;
  }
  guatiao_value tmp;
  gt_build_or_raise(result, a, &tmp);
  switch (ans->ret) {
  case S_STRING:
    if (tmp.tag != GUATIAO_STRING) {
      gt_free(&tmp);
      gt_raise(GUATIAO_ERR_BAD_VALUE, "the answer is one string");
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
    if (empty) {
      gt_free(&tmp);
      guatiao_status st = want == GUATIAO_MAP ? gt_core.guatiao_value_map(a, &tmp) : gt_core.guatiao_value_list(a, &tmp);
      if (st) gt_raise(st, "the allocator refused");
    }
    if (tmp.tag != (uint32_t)want) {
      gt_free(&tmp);
      gt_raise(GUATIAO_ERR_BAD_VALUE, want == GUATIAO_MAP ? "the answer is a map (a named list)" : "the answer is a list");
    }
    if (want == GUATIAO_MAP) *(guatiao_map *)out = tmp.payload.map;
    else *(guatiao_list *)out = tmp.payload.list;
    break;
  }
  default: gt_free(&tmp); gt_raise(GUATIAO_ERR_INTERNAL, "an unknown answer shape");
  }
  ans->live = 0; /* one answer per call */
  return R_NilValue;
}

R_CFinalizer_t gt_remote_finalizer(void) { return remote_finalize; }

void gt_warn_unfallible(const gt_slot *sl, const char *msg) {
  (void)sl;
  REprintf("guatiao: a slot that cannot fail failed: %s\n", msg);
}

SEXP C_remote_live(SEXP handle) {
  gt_remote *r = TYPEOF(handle) == EXTPTRSXP ? (gt_remote *)R_ExternalPtrAddr(handle) : NULL;
  return Rf_ScalarLogical(r && r->live);
}
