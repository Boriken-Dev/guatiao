/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A library written in R: its descriptor is built in C memory once and
 * kept for the session, since a host may hold it as long as it lives. The
 * descriptor's own slots (the entry, unload, and each provider's
 * available, create and destroy) are libffi closures over handlers here.
 * The entry and unload answer from C alone, on any thread; the others
 * reach R only on R's main thread, inside R_ToplevelExec.
 */

#include <ffi.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"
#include "kinds.h"

struct gt_library;

typedef struct gt_provider {
  struct gt_library *lib;
  int index; /* the R-side provider record */
} gt_provider;

typedef struct gt_library {
  guatiao_library_info info;
  guatiao_provider_info *providers;
  gt_provider *recs;
  int nproviders;
  int closed;
  long live; /* instances built and not yet destroyed; guarded by gt_lock */
  void *entry_code;
} gt_library;

/* ---- closures over the descriptor's slots -------------------------------------- */

static ffi_cif entry_cif, unload_cif, available_cif, create_cif, destroy_cif;
static ffi_type *entry_args[1], *available_args[2], *create_args[4], *destroy_args[2];
static int cifs_ready = 0;

static void prep_cifs(void) {
  if (cifs_ready) return;
  entry_args[0] = &ffi_type_pointer;
  available_args[0] = available_args[1] = &ffi_type_pointer;
  for (int i = 0; i < 4; i++) create_args[i] = &ffi_type_pointer;
  destroy_args[0] = destroy_args[1] = &ffi_type_pointer;
  if (ffi_prep_cif(&entry_cif, FFI_DEFAULT_ABI, 1, &ffi_type_pointer, entry_args) != FFI_OK ||
      ffi_prep_cif(&unload_cif, FFI_DEFAULT_ABI, 0, &ffi_type_uint32, NULL) != FFI_OK ||
      ffi_prep_cif(&available_cif, FFI_DEFAULT_ABI, 2, &ffi_type_uint8, available_args) != FFI_OK ||
      ffi_prep_cif(&create_cif, FFI_DEFAULT_ABI, 4, &ffi_type_uint32, create_args) != FFI_OK ||
      ffi_prep_cif(&destroy_cif, FFI_DEFAULT_ABI, 2, &ffi_type_void, destroy_args) != FFI_OK)
    gt_raise(GUATIAO_ERR_INTERNAL, "libffi refused a descriptor's call interface");
  cifs_ready = 1;
}

static void *closure(ffi_cif *cif, void (*fn)(ffi_cif *, void *, void **, void *), void *user) {
  void *code = NULL;
  ffi_closure *cl = (ffi_closure *)ffi_closure_alloc(sizeof(ffi_closure), &code);
  if (!cl) gt_raise(GUATIAO_ERR_ALLOC, "libffi could not allocate a closure");
  if (ffi_prep_closure_loc(cl, cif, fn, user, code) != FFI_OK) {
    ffi_closure_free(cl);
    gt_raise(GUATIAO_ERR_INTERNAL, "libffi refused a closure");
  }
  return code;
}

static void on_entry(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)args;
  gt_library *lib = (gt_library *)user;
  *(void **)ret = lib->closed ? NULL : (void *)&lib->info;
}

static void on_unload(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)args;
  gt_library *lib = (gt_library *)user;
  gt_lock();
  long live = lib->live;
  gt_unlock();
  *(ffi_arg *)ret = live > 0 ? GUATIAO_ERR_BUSY : GUATIAO_OK;
}

/* A reason a provider gave, kept for every reader: a host may read it any
 * time after. One copy per distinct text. */
typedef struct reason {
  struct reason *next;
  size_t len;
  char text[1];
} reason;
static reason *reasons = NULL;

static const char *keep_reason(const char *text, size_t *len) {
  size_t n = strlen(text);
  gt_lock();
  for (reason *r = reasons; r; r = r->next) {
    if (r->len == n && memcmp(r->text, text, n) == 0) {
      gt_unlock();
      *len = n;
      return r->text;
    }
  }
  reason *r = (reason *)malloc(sizeof(reason) + n);
  if (r) {
    r->len = n;
    memcpy(r->text, text, n + 1);
    r->next = reasons;
    reasons = r;
  }
  gt_unlock();
  *len = r ? n : 0;
  return r ? r->text : "";
}

typedef struct available_job {
  int index;
  char why[1024];
  int has_why;
} available_job;

static void available_body(void *p) {
  available_job *j = (available_job *)p;
  SEXP idx = PROTECT(Rf_ScalarInteger(j->index));
  SEXP r = PROTECT(gt_call_r(".gt_available", 1, idx, R_NilValue, R_NilValue));
  if (TYPEOF(r) == STRSXP && XLENGTH(r) == 1) {
    snprintf(j->why, sizeof j->why, "%s", Rf_translateCharUTF8(STRING_ELT(r, 0)));
    j->has_why = 1;
  }
  UNPROTECT(2);
}

static void on_available(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  gt_provider *p = (gt_provider *)user;
  guatiao_str *reason_out = *(guatiao_str **)args[1];
  const char *why = NULL;
  available_job j;
  memset(&j, 0, sizeof j);
  j.index = p->index;
  if (!gt_on_main_thread()) {
    why = GT_WRONG_THREAD;
  } else if (!R_ToplevelExec(available_body, &j)) {
    why = "asking whether the provider can run failed in R";
  } else if (j.has_why) {
    why = j.why;
  }
  if (!why) {
    *(ffi_arg *)ret = 1;
    return;
  }
  size_t len = 0;
  const char *kept = keep_reason(why, &len);
  if (reason_out) *reason_out = guatiao_str_from(kept, len);
  *(ffi_arg *)ret = 0;
}

typedef struct create_job {
  int index;
  guatiao_value *config;
  guatiao_status status;
  char msg[1024];
  double ctx;
  SEXP owner;
} create_job;

static void create_body(void *p) {
  create_job *j = (create_job *)p;
  SEXP owner = gt_owner_borrow(j->config, &gt_binding_alloc, 1, R_NilValue);
  R_PreserveObject(owner);
  j->owner = owner;
  SEXP node = PROTECT(Rf_allocVector(VECSXP, 2));
  SET_VECTOR_ELT(node, 0, owner);
  SET_VECTOR_ELT(node, 1, Rf_allocVector(VECSXP, 0));
  SEXP cls = PROTECT(Rf_mkString("guatiao_node"));
  Rf_setAttrib(node, R_ClassSymbol, cls);
  SEXP idx = PROTECT(Rf_ScalarInteger(j->index));
  SEXP r = PROTECT(gt_call_r(".gt_create", 2, idx, node, R_NilValue));
  j->status = (guatiao_status)Rf_asInteger(VECTOR_ELT(r, 0));
  SEXP m = VECTOR_ELT(r, 1);
  if (TYPEOF(m) == STRSXP && XLENGTH(m) == 1) snprintf(j->msg, sizeof j->msg, "%s", Rf_translateCharUTF8(STRING_ELT(m, 0)));
  j->ctx = Rf_asReal(VECTOR_ELT(r, 2));
  UNPROTECT(4);
}

static void on_create(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  gt_provider *p = (gt_provider *)user;
  const guatiao_value *config = *(const guatiao_value **)args[1];
  void **out = *(void ***)args[2];
  guatiao_provider_error *err = *(guatiao_provider_error **)args[3];
  if (out) *out = NULL;
  if (!out) {
    gt_write_err(err, GUATIAO_ERR_NULL, "no slot for the instance");
    *(ffi_arg *)ret = GUATIAO_ERR_NULL;
    return;
  }
  if (!gt_on_main_thread()) {
    gt_write_err(err, GUATIAO_ERR_UNSUPPORTED, GT_WRONG_THREAD);
    *(ffi_arg *)ret = GUATIAO_ERR_UNSUPPORTED;
    return;
  }
  guatiao_value null_config;
  memset(&null_config, 0, sizeof null_config);
  null_config.tag = GUATIAO_NULL;
  create_job *j = (create_job *)calloc(1, sizeof(create_job));
  if (!j) {
    *(ffi_arg *)ret = GUATIAO_ERR_ALLOC;
    return;
  }
  j->index = p->index;
  j->config = config ? (guatiao_value *)config : &null_config;
  j->status = GUATIAO_ERR_INTERNAL;
  snprintf(j->msg, sizeof j->msg, "an R error ended the call");
  if (!R_ToplevelExec(create_body, j)) j->status = GUATIAO_ERR_INTERNAL;
  if (j->owner) {
    gt_owner_clear(j->owner);
    R_ReleaseObject(j->owner);
  }
  guatiao_status st = j->status;
  if (st == GUATIAO_OK) {
    *out = (void *)(uintptr_t)j->ctx;
    gt_lock();
    p->lib->live++;
    gt_unlock();
  } else {
    gt_write_err(err, st, j->msg);
  }
  free(j);
  *(ffi_arg *)ret = st;
}

typedef struct release_job {
  double id;
} release_job;

static void release_body(void *p) {
  release_job *r = (release_job *)p;
  SEXP id = PROTECT(Rf_ScalarReal(r->id));
  gt_call_r(".gt_release_id", 1, id, R_NilValue, R_NilValue);
  UNPROTECT(1);
}

static void on_destroy(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)ret;
  gt_provider *p = (gt_provider *)user;
  double id = gt_id_of_ctx(*(void **)args[1]);
  if (id < 0) return;
  gt_lock();
  if (p->lib->live > 0) p->lib->live--;
  gt_unlock();
  if (!gt_on_main_thread()) {
    gt_queue_destroy(id);
    return;
  }
  release_job r = {id};
  R_ToplevelExec(release_body, &r);
}

/* ---- building the descriptor --------------------------------------------------- */

static guatiao_str copy_str(SEXP s) {
  const char *p = (TYPEOF(s) == STRSXP && XLENGTH(s) == 1) ? Rf_translateCharUTF8(STRING_ELT(s, 0)) : "";
  size_t n = strlen(p);
  char *c = (char *)malloc(n + 1);
  if (!c) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  memcpy(c, p, n + 1);
  return guatiao_str_from(c, n);
}

/* A tree the descriptor keeps for the session, or NULL for R's NULL. */
static guatiao_value *keep_tree(SEXP x, int want_map) {
  if (x == R_NilValue) return NULL;
  guatiao_value *v = (guatiao_value *)calloc(1, sizeof(guatiao_value));
  if (!v) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  char msg[1024];
  guatiao_status st = gt_build(x, &gt_binding_alloc, v, msg, sizeof msg);
  if (st) {
    free(v);
    gt_raise(st, "%s", msg);
  }
  if (want_map && v->tag != GUATIAO_MAP) {
    gt_free(v);
    free(v);
    gt_raise(GUATIAO_ERR_BAD_VALUE, "meta is a map (a named list)");
  }
  return v;
}

static SEXP field(SEXP list, const char *name) {
  SEXP names = Rf_getAttrib(list, R_NamesSymbol);
  for (R_xlen_t i = 0; i < XLENGTH(list); i++)
    if (strcmp(CHAR(STRING_ELT(names, i)), name) == 0) return VECTOR_ELT(list, i);
  return R_NilValue;
}

SEXP C_library_new(SEXP id, SEXP version, SEXP meta, SEXP descs) {
  gt_need(GT_LIB_CORE);
  prep_cifs();
  int n = (int)XLENGTH(descs);
  gt_library *lib = (gt_library *)calloc(1, sizeof(gt_library));
  guatiao_provider_info *infos = (guatiao_provider_info *)calloc((size_t)(n ? n : 1), sizeof(guatiao_provider_info));
  gt_provider *recs = (gt_provider *)calloc((size_t)(n ? n : 1), sizeof(gt_provider));
  if (!lib || !infos || !recs) {
    free(lib);
    free(infos);
    free(recs);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  /* What is built from here is kept for the session: a host may hold it. */
  lib->providers = infos;
  lib->recs = recs;
  lib->nproviders = n;
  for (int i = 0; i < n; i++) {
    SEXP d = VECTOR_ELT(descs, i);
    guatiao_provider_info *p = &infos[i];
    gt_provider *rec = &recs[i];
    rec->lib = lib;
    rec->index = Rf_asInteger(field(d, "index"));
    SEXP kinds = field(d, "kinds");
    SEXP tables = field(d, "tables");
    SEXP sizes = field(d, "sizes");
    int nk = (int)XLENGTH(kinds);
    guatiao_str *names = (guatiao_str *)calloc((size_t)(nk ? nk : 1), sizeof(guatiao_str));
    guatiao_kind_table *kt = (guatiao_kind_table *)calloc((size_t)(nk ? nk : 1), sizeof(guatiao_kind_table));
    if (!names || !kt) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
    for (int k = 0; k < nk; k++) {
      SEXP one = PROTECT(Rf_ScalarString(STRING_ELT(kinds, k)));
      names[k] = copy_str(one);
      UNPROTECT(1);
      kt[k].struct_size = (uint32_t)sizeof(guatiao_kind_table);
      kt[k].vtable_size = (uint32_t)REAL(sizes)[k];
      kt[k].kind = names[k];
      kt[k].vtable = R_ExternalPtrAddr(VECTOR_ELT(tables, k));
    }
    p->struct_size = (uint32_t)sizeof(guatiao_provider_info);
    p->vtable_size = 0;
    p->kinds.ptr = names;
    p->kinds.len = (size_t)nk;
    p->id = copy_str(field(d, "id"));
    p->display_name = copy_str(field(d, "display_name"));
    p->config = keep_tree(field(d, "schema"), 0);
    p->vtable = NULL;
    double ctx = Rf_asReal(field(d, "ctx"));
    p->ctx = ctx > 0 ? (void *)(uintptr_t)ctx : NULL;
    guatiao_value *m = keep_tree(field(d, "meta"), 1);
    p->meta = m ? &m->payload.map : NULL;
    p->version = copy_str(field(d, "version"));
    p->available = Rf_asLogical(field(d, "available")) == 1
                       ? (bool (*)(void *, guatiao_str *))closure(&available_cif, on_available, rec)
                       : NULL;
    p->tables.ptr = kt;
    p->tables.len = (size_t)nk;
    p->tables.stride = sizeof(guatiao_kind_table);
    if (Rf_asLogical(field(d, "configured")) == 1) {
      p->create = (guatiao_status(*)(void *, const guatiao_value *, void **, guatiao_provider_error *))closure(
          &create_cif, on_create, rec);
      p->destroy = (void (*)(void *, void *))closure(&destroy_cif, on_destroy, rec);
    }
  }
  lib->info.struct_size = (uint32_t)sizeof(guatiao_library_info);
  lib->info.abi_version = GUATIAO_ABI_VERSION;
  lib->info.id = copy_str(id);
  lib->info.version = copy_str(version);
  lib->info.providers.ptr = infos;
  lib->info.providers.len = (size_t)n;
  lib->info.providers.stride = sizeof(guatiao_provider_info);
  guatiao_value *lm = keep_tree(meta, 1);
  lib->info.meta = lm ? &lm->payload.map : NULL;
  lib->info.unload = (guatiao_status(*)(void))closure(&unload_cif, on_unload, lib);
  lib->entry_code = closure(&entry_cif, on_entry, lib);
  return R_MakeExternalPtr(lib, R_NilValue, R_NilValue);
}

static gt_library *library_of(SEXP x) {
  gt_library *lib = TYPEOF(x) == EXTPTRSXP ? (gt_library *)R_ExternalPtrAddr(x) : NULL;
  if (!lib) gt_raise(GUATIAO_ERR_GONE, "no such library");
  return lib;
}

SEXP C_library_entry(SEXP x) { return R_MakeExternalPtr(library_of(x)->entry_code, R_NilValue, x); }

SEXP C_library_close(SEXP x) {
  library_of(x)->closed = 1;
  return R_NilValue;
}

SEXP C_library_live(SEXP x) {
  gt_library *lib = library_of(x);
  gt_lock();
  long live = lib->live;
  gt_unlock();
  return Rf_ScalarReal((double)live);
}

/* The descriptor a library's entry answers, for a test reading it. */
SEXP C_library_info(SEXP x) { return R_MakeExternalPtr(&library_of(x)->info, R_NilValue, x); }
