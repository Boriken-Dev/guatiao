/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The registry: every answer that is data is read into R and freed here. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

static void registry_finalize(SEXP x) {
  guatiao_registry *reg = (guatiao_registry *)R_ExternalPtrAddr(x);
  if (!reg) return;
  if (gt_core.guatiao_registry_free) gt_core.guatiao_registry_free(reg);
  R_ClearExternalPtr(x);
}

static guatiao_registry *reg_of(SEXP x) {
  guatiao_registry *reg = TYPEOF(x) == EXTPTRSXP ? (guatiao_registry *)R_ExternalPtrAddr(x) : NULL;
  if (!reg) {
    gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    Rf_error("closed");
  }
  return reg;
}

SEXP C_registry_new(SEXP id, SEXP version, SEXP alloc) {
  guatiao_str i = gt_str_arg(id, "the id");
  guatiao_str v = gt_str_arg(version, "the version");
  const guatiao_alloc *a = alloc == R_NilValue ? &gt_binding_alloc : gt_alloc_arg(alloc);
  guatiao_registry *reg = GT_CORE_FN(guatiao_registry_new)(i, v, a);
  if (!reg) gt_raise(GUATIAO_ERR_BAD_VALUE, "the registry was refused");
  SEXP x = PROTECT(R_MakeExternalPtr(reg, R_NilValue, alloc));
  R_RegisterCFinalizerEx(x, registry_finalize, TRUE);
  UNPROTECT(1);
  return x;
}

SEXP C_registry_free(SEXP x) {
  registry_finalize(x);
  return R_NilValue;
}

/* An answer built in the binding's allocator: plain R data, freed. */
static SEXP answer(guatiao_status st, guatiao_value *out, const char *what) {
  if (st) {
    gt_free(out);
    gt_raise(st, "%s", what);
  }
  return gt_read_and_free(out, 0);
}

SEXP C_registry_load_file(SEXP x, SEXP path) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str p = gt_str_arg(path, "the path");
  guatiao_value out;
  set_absent(&out);
  return answer(GT_CORE_FN(guatiao_registry_load_file)(reg, p, &gt_binding_alloc, &out), &out, "the file was refused");
}

typedef const guatiao_library_info *(*entry_fn)(const guatiao_host_info *);

SEXP C_registry_register_entry(SEXP x, SEXP name, SEXP entry) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str n = gt_str_arg(name, "the name");
  if (TYPEOF(entry) != EXTPTRSXP || !R_ExternalPtrAddr(entry)) gt_raise(GUATIAO_ERR_NULL, "no entry function");
  entry_fn fn;
  void *p = R_ExternalPtrAddr(entry);
  memcpy(&fn, &p, sizeof fn);
  guatiao_value out;
  set_absent(&out);
  return answer(GT_CORE_FN(guatiao_registry_register_entry)(reg, n, fn, &gt_binding_alloc, &out), &out,
                "the library was refused");
}

SEXP C_registry_scan(SEXP x, SEXP where, SEXP rules, SEXP descending, SEXP is_path) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str w = gt_str_arg(where, "the directory");
  guatiao_str r = gt_str_arg(rules, "the rules");
  bool desc = Rf_asLogical(descending) == 1;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = Rf_asLogical(is_path) == 1
                          ? GT_CORE_FN(guatiao_registry_scan_path)(reg, w, desc, r, &gt_binding_alloc, &out)
                          : GT_CORE_FN(guatiao_registry_scan_dir_rules)(reg, w, desc, r, &gt_binding_alloc, &out);
  return answer(st, &out, "the scan was refused (a rule that is not one?)");
}

SEXP C_registry_query(SEXP x, SEXP what, SEXP arg) {
  guatiao_registry *reg = reg_of(x);
  int w = Rf_asInteger(what);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  if (w == 0) {
    st = GT_CORE_FN(guatiao_registry_libraries)(reg, &gt_binding_alloc, &out);
  } else {
    guatiao_str a = gt_str_arg(arg, "the kind or key");
    switch (w) {
    case 1: st = GT_CORE_FN(guatiao_registry_providers)(reg, a, &gt_binding_alloc, &out); break;
    case 2: st = GT_CORE_FN(guatiao_registry_available)(reg, a, &gt_binding_alloc, &out); break;
    case 3: st = GT_CORE_FN(guatiao_registry_why_not)(reg, a, &gt_binding_alloc, &out); break;
    case 4: st = GT_CORE_FN(guatiao_registry_provider)(reg, a, &gt_binding_alloc, &out); break;
    default: st = GT_CORE_FN(guatiao_registry_best)(reg, a, &gt_binding_alloc, &out); break;
    }
  }
  return answer(st, &out, w == 4 ? "no provider answers to that key" : w == 5 ? "nothing can serve that kind" : "refused");
}

SEXP C_registry_provider_available(SEXP x, SEXP key) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str k = gt_str_arg(key, "the key");
  guatiao_str why = {NULL, 0};
  bool ok = GT_CORE_FN(guatiao_registry_provider_available)(reg, k, &why);
  SEXP out = PROTECT(Rf_allocVector(VECSXP, 2));
  SEXP names = PROTECT(Rf_allocVector(STRSXP, 2));
  SET_STRING_ELT(names, 0, Rf_mkChar("available"));
  SET_STRING_ELT(names, 1, Rf_mkChar("reason"));
  SET_VECTOR_ELT(out, 0, Rf_ScalarLogical(ok));
  if (!ok && why.len) SET_VECTOR_ELT(out, 1, gt_text_sexp(why));
  Rf_setAttrib(out, R_NamesSymbol, names);
  UNPROTECT(2);
  return out;
}

SEXP C_registry_keyed_by(SEXP x, SEXP template_, SEXP libraries) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str t = gt_str_arg(template_, "the template");
  guatiao_status st = Rf_asLogical(libraries) == 1 ? GT_CORE_FN(guatiao_registry_libraries_keyed_by)(reg, t)
                                                   : GT_CORE_FN(guatiao_registry_keyed_by)(reg, t);
  if (st) gt_raise(st, "the template was refused");
  return R_NilValue;
}

SEXP C_registry_set_priority(SEXP x, SEXP id, SEXP priority) {
  guatiao_registry *reg = reg_of(x);
  guatiao_status st = GT_CORE_FN(guatiao_registry_set_priority)(reg, gt_str_arg(id, "the id"), Rf_asInteger(priority));
  if (st) gt_raise(st, "the priority was refused");
  return R_NilValue;
}

SEXP C_registry_priority(SEXP x, SEXP id) {
  guatiao_registry *reg = reg_of(x);
  return Rf_ScalarInteger(GT_CORE_FN(guatiao_registry_priority)(reg, gt_str_arg(id, "the id")));
}

SEXP C_registry_provider_table(SEXP x, SEXP key, SEXP kind) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str k = gt_str_arg(key, "the key");
  size_t size = 0;
  const void *table = kind == R_NilValue ? GT_CORE_FN(guatiao_registry_provider_vtable)(reg, k, &size)
                                         : GT_CORE_FN(guatiao_registry_provider_table)(reg, k, gt_str_arg(kind, "the kind"), &size);
  void *ctx = GT_CORE_FN(guatiao_registry_provider_ctx)(reg, k);
  SEXP out = PROTECT(Rf_allocVector(VECSXP, 3));
  SEXP names = PROTECT(Rf_allocVector(STRSXP, 3));
  SET_STRING_ELT(names, 0, Rf_mkChar("table"));
  SET_STRING_ELT(names, 1, Rf_mkChar("size"));
  SET_STRING_ELT(names, 2, Rf_mkChar("ctx"));
  if (table) SET_VECTOR_ELT(out, 0, R_MakeExternalPtr((void *)table, R_NilValue, x));
  SET_VECTOR_ELT(out, 1, Rf_ScalarReal(table ? (double)size : 0));
  SET_VECTOR_ELT(out, 2, R_MakeExternalPtr(ctx, R_NilValue, x));
  Rf_setAttrib(out, R_NamesSymbol, names);
  UNPROTECT(2);
  return out;
}

/* A copy of a provider's configuration schema, so nothing R holds reads
 * the registry's after a retire. */
SEXP C_registry_provider_config(SEXP x, SEXP key) {
  guatiao_registry *reg = reg_of(x);
  const guatiao_value *s = GT_CORE_FN(guatiao_registry_provider_config)(reg, gt_str_arg(key, "the key"));
  if (!s) return R_NilValue;
  guatiao_value copy;
  set_absent(&copy);
  guatiao_status st = GT_CORE_FN(guatiao_value_clone)(&gt_binding_alloc, s, &copy);
  if (st) gt_raise(st, "the copy was refused");
  SEXP owner = gt_owner_new(&copy, &gt_binding_alloc, R_NilValue);
  if (!owner) {
    gt_free(&copy);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  return owner;
}

SEXP C_registry_host(SEXP x) {
  guatiao_registry *reg = reg_of(x);
  const guatiao_host_info *h = GT_CORE_FN(guatiao_registry_host)(reg);
  return R_MakeExternalPtr((void *)h, R_NilValue, R_NilValue);
}

SEXP C_registry_create(SEXP x, SEXP key, SEXP config) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str k = gt_str_arg(key, "the key");
  guatiao_value tmp;
  const guatiao_value *c = gt_tree_arg(config, &tmp);
  void *instance = NULL;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = GT_CORE_FN(guatiao_registry_provider_create)(reg, k, c, &instance, &err);
  gt_free(&tmp);
  if (st) gt_raise_err(st, &err, st == GUATIAO_ERR_NOT_FOUND ? "no provider answers to that key"
                                 : st == GUATIAO_ERR_NULL     ? "the provider builds no instances"
                                                              : NULL);
  gt_raise_err_free(&err);
  return R_MakeExternalPtr(instance, R_NilValue, R_NilValue);
}

SEXP C_registry_destroy(SEXP x, SEXP key, SEXP instance) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str k = gt_str_arg(key, "the key");
  void *ctx = TYPEOF(instance) == EXTPTRSXP ? R_ExternalPtrAddr(instance) : NULL;
  if (ctx) GT_CORE_FN(guatiao_registry_provider_destroy)(reg, k, ctx);
  if (TYPEOF(instance) == EXTPTRSXP) R_ClearExternalPtr(instance);
  return R_NilValue;
}

SEXP C_registry_retire(SEXP x, SEXP key) {
  guatiao_registry *reg = reg_of(x);
  guatiao_status st = GT_CORE_FN(guatiao_registry_retire)(reg, gt_str_arg(key, "the key"));
  if (st) gt_raise(st, st == GUATIAO_ERR_NOT_FOUND ? "no library answers to that key" : "the retire was refused");
  return R_NilValue;
}

SEXP C_registry_collect(SEXP x) {
  guatiao_registry *reg = reg_of(x);
  guatiao_value out;
  set_absent(&out);
  return answer(GT_CORE_FN(guatiao_registry_collect)(reg, &gt_binding_alloc, &out), &out, "the collect was refused");
}

SEXP C_registry_unload(SEXP x, SEXP key, SEXP unchecked) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str k = gt_str_arg(key, "the key");
  guatiao_status st = Rf_asLogical(unchecked) == 1 ? GT_CORE_FN(guatiao_registry_unload_unchecked)(reg, k)
                                                   : GT_CORE_FN(guatiao_registry_unload)(reg, k);
  if (st) {
    const char *why = st == GUATIAO_ERR_BUSY          ? "something it handed out is still alive"
                      : st == GUATIAO_ERR_UNSUPPORTED ? "the library has no unload function"
                      : st == GUATIAO_ERR_WRONG_KIND  ? "the library is linked, not mapped"
                      : st == GUATIAO_ERR_NOT_FOUND   ? "no library answers to that key"
                                                      : "the unload was refused";
    gt_raise(st, "%s", why);
  }
  return R_NilValue;
}

SEXP C_registry_add_source_file(SEXP x, SEXP path, SEXP config) {
  guatiao_registry *reg = reg_of(x);
  guatiao_str p = gt_str_arg(path, "the path");
  guatiao_value tmp;
  const guatiao_value *c = gt_tree_arg(config, &tmp);
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = GT_CORE_FN(guatiao_registry_add_source_file)(reg, p, c, &err);
  gt_free(&tmp);
  if (st) gt_raise_err(st, &err, "the source was refused");
  gt_raise_err_free(&err);
  return R_NilValue;
}

SEXP C_registry_watch_dir(SEXP x, SEXP dir, SEXP rules) {
  guatiao_registry *reg = reg_of(x);
  guatiao_status st = GT_CORE_FN(guatiao_registry_watch_dir)(reg, gt_str_arg(dir, "the directory"), gt_str_arg(rules, "the rules"));
  if (st) gt_raise(st, "the directory was refused (watched already, or a rule that is not one)");
  return R_NilValue;
}

SEXP C_registry_refresh(SEXP x) {
  guatiao_registry *reg = reg_of(x);
  guatiao_value out;
  set_absent(&out);
  return answer(GT_CORE_FN(guatiao_registry_refresh)(reg, &gt_binding_alloc, &out), &out, "the refresh was refused");
}
