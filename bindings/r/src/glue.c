/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"

/* ---- the package's namespace and its R helpers ----------------------------- */

static SEXP gt_ns = NULL;

SEXP gt_namespace(void) {
  if (!gt_ns) {
    SEXP name = PROTECT(Rf_mkString("guatiao"));
    gt_ns = R_FindNamespace(name);
    R_PreserveObject(gt_ns);
    UNPROTECT(1);
  }
  return gt_ns;
}

SEXP gt_call_r(const char *fn, int n, SEXP a, SEXP b, SEXP c) {
  SEXP f = PROTECT(Rf_install(fn));
  SEXP call;
  switch (n) {
  case 0: call = PROTECT(Rf_lang1(f)); break;
  case 1: call = PROTECT(Rf_lang2(f, a)); break;
  case 2: call = PROTECT(Rf_lang3(f, a, b)); break;
  default: call = PROTECT(Rf_lang4(f, a, b, c)); break;
  }
  SEXP out = Rf_eval(call, gt_namespace());
  UNPROTECT(2);
  return out;
}

/* ---- raising ---------------------------------------------------------------- */

void gt_raise(guatiao_status status, const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  SEXP st = PROTECT(Rf_ScalarReal((double)status));
  SEXP msg = PROTECT(Rf_allocVector(STRSXP, 1));
  SET_STRING_ELT(msg, 0, Rf_mkCharCE(buf, CE_UTF8));
  gt_call_r(".gt_raise", 2, st, msg, R_NilValue);
  UNPROTECT(2);
  Rf_error("unreachable: a guatiao error was not signalled");
}

void gt_raise_err(guatiao_status status, guatiao_provider_error *err, const char *fallback) {
  char buf[2048];
  size_t n = 0;
  if (err && err->message.len && err->message.ptr) {
    n = err->message.len < sizeof buf - 1 ? err->message.len : sizeof buf - 1;
    memcpy(buf, err->message.ptr, n);
  }
  buf[n] = '\0';
  if (err) {
    guatiao_value m;
    memset(&m, 0, sizeof m);
    m.tag = GUATIAO_STRING;
    m.payload.text = err->message;
    gt_free(&m);
    memset(&err->message, 0, sizeof err->message);
  }
  if (n == 0 && fallback) gt_raise(status, "%s", fallback);
  gt_raise(status, "%s", buf);
}

void gt_need(int lib) {
  if (gt_is_open(lib)) return;
  SEXP l = PROTECT(Rf_ScalarInteger(lib));
  gt_call_r(".gt_ensure", 1, l, R_NilValue, R_NilValue);
  UNPROTECT(1);
  if (!gt_is_open(lib)) Rf_error("the library did not open");
}

void gt_missing_symbol(int lib, const char *name) {
  const char *feature = gt_missing_feature(lib, name);
  SEXP l = PROTECT(Rf_ScalarInteger(lib));
  SEXP s = PROTECT(Rf_mkString(name));
  SEXP f = PROTECT(Rf_mkString(feature ? feature : ""));
  gt_call_r(".gt_missing_symbol", 3, l, s, f);
  UNPROTECT(3);
  Rf_error("unreachable: MissingSymbol was not signalled");
}

/* ---- text -------------------------------------------------------------------- */

guatiao_str gt_str_arg(SEXP s, const char *what) {
  if (TYPEOF(s) != STRSXP || XLENGTH(s) != 1 || STRING_ELT(s, 0) == NA_STRING)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "%s is one string", what);
  const char *p = Rf_translateCharUTF8(STRING_ELT(s, 0));
  return guatiao_str_from(p, strlen(p));
}

SEXP gt_mkchar(guatiao_str t) {
  if (t.len && memchr(t.ptr, 0, t.len)) return NULL;
  if (t.len > (size_t)INT32_MAX) return NULL;
  return Rf_mkCharLenCE(t.len ? (const char *)t.ptr : "", (int)t.len, CE_UTF8);
}

SEXP gt_text_sexp(guatiao_str t) {
  SEXP c = gt_mkchar(t);
  if (!c) gt_raise(GUATIAO_ERR_BAD_VALUE, "text holding a NUL cannot be an R string");
  PROTECT(c);
  SEXP out = Rf_ScalarString(c);
  UNPROTECT(1);
  return out;
}

SEXP gt_take_text(guatiao_value *v) {
  guatiao_str t = guatiao_string_text(v);
  SEXP c = gt_mkchar(t);
  if (!c) {
    gt_free(v);
    gt_raise(GUATIAO_ERR_BAD_VALUE, "text holding a NUL cannot be an R string");
  }
  PROTECT(c);
  SEXP out = PROTECT(Rf_ScalarString(c));
  gt_free(v);
  UNPROTECT(2);
  return out;
}

/* ---- owners --------------------------------------------------------------------- */

void gt_free(guatiao_value *v) {
  if (!v) return;
  switch (v->tag) {
  case GUATIAO_ABSENT:
  case GUATIAO_NULL:
  case GUATIAO_BOOL: break;
  default:
    if (gt_core.guatiao_value_free) gt_core.guatiao_value_free(v);
    return;
  }
  memset(v, 0, sizeof *v);
  v->tag = GUATIAO_NULL;
}

static void owner_finalize(SEXP x) {
  gt_owner *o = (gt_owner *)R_ExternalPtrAddr(x);
  if (!o) return;
  if (o->flags & GT_OWNED) {
    gt_free(o->root);
    free(o->root);
  }
  free(o);
  R_ClearExternalPtr(x);
}

SEXP gt_owner_new(const guatiao_value *v, const guatiao_alloc *a, SEXP keep) {
  gt_owner *o = (gt_owner *)calloc(1, sizeof(gt_owner));
  guatiao_value *root = (guatiao_value *)calloc(1, sizeof(guatiao_value));
  if (!o || !root) {
    free(o);
    free(root);
    return NULL;
  }
  *root = *v;
  o->root = root;
  o->alloc = a ? a : &gt_binding_alloc;
  o->flags = GT_OWNED;
  SEXP x = PROTECT(R_MakeExternalPtr(o, R_NilValue, keep));
  R_RegisterCFinalizerEx(x, owner_finalize, TRUE);
  UNPROTECT(1);
  return x;
}

SEXP gt_owner_borrow(guatiao_value *node, const guatiao_alloc *a, int readonly, SEXP keep) {
  gt_owner *o = (gt_owner *)calloc(1, sizeof(gt_owner));
  if (!o) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  o->root = node;
  o->alloc = a ? a : &gt_binding_alloc;
  o->flags = readonly ? GT_READONLY : 0;
  SEXP x = PROTECT(R_MakeExternalPtr(o, R_NilValue, keep));
  R_RegisterCFinalizerEx(x, owner_finalize, TRUE);
  UNPROTECT(1);
  return x;
}

void gt_owner_clear(SEXP owner) {
  if (TYPEOF(owner) != EXTPTRSXP) return;
  gt_owner *o = (gt_owner *)R_ExternalPtrAddr(owner);
  if (!o) return;
  if (o->flags & GT_OWNED) {
    gt_free(o->root);
    free(o->root);
  }
  free(o);
  R_ClearExternalPtr(owner);
}

gt_owner *gt_owner_of(SEXP owner) {
  if (TYPEOF(owner) != EXTPTRSXP) gt_raise(GUATIAO_ERR_WRONG_KIND, "not a guatiao value");
  gt_owner *o = (gt_owner *)R_ExternalPtrAddr(owner);
  if (!o) {
    gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    Rf_error("closed");
  }
  return o;
}

/* 0: found; 1: the owner is closed; 2: the path no longer leads anywhere. */
static int node_lookup(SEXP node, gt_owner **owner_out, guatiao_value **out) {
  SEXP owner = VECTOR_ELT(node, 0);
  SEXP path = VECTOR_ELT(node, 1);
  if (TYPEOF(owner) != EXTPTRSXP) return 2;
  gt_owner *o = (gt_owner *)R_ExternalPtrAddr(owner);
  if (!o) return 1;
  guatiao_value *v = o->root;
  R_xlen_t n = XLENGTH(path);
  for (R_xlen_t i = 0; i < n; i++) {
    SEXP step = VECTOR_ELT(path, i);
    if (TYPEOF(step) == STRSXP) {
      const char *k = Rf_translateCharUTF8(STRING_ELT(step, 0));
      const guatiao_value *c = guatiao_map_find(v, guatiao_str_from(k, strlen(k)));
      if (!c) return 2;
      v = (guatiao_value *)c;
    } else {
      int at = INTEGER(step)[0];
      const guatiao_value *c = at < 0 ? NULL : guatiao_list_at(v, (size_t)at);
      if (!c) return 2;
      v = (guatiao_value *)c;
    }
  }
  if (owner_out) *owner_out = o;
  *out = v;
  return 0;
}

guatiao_value *gt_node_ptr(SEXP node, gt_owner **owner_out) {
  if (TYPEOF(node) != VECSXP || XLENGTH(node) < 2) gt_raise(GUATIAO_ERR_WRONG_KIND, "not a guatiao value");
  guatiao_value *v = NULL;
  switch (node_lookup(node, owner_out, &v)) {
  case 0: return v;
  case 1:
    gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    Rf_error("closed");
  default: gt_raise(GUATIAO_ERR_NOT_FOUND, "the node this view addressed is no longer there");
  }
}

int gt_node_try(SEXP node, gt_owner **owner_out, guatiao_value **out) {
  if (TYPEOF(node) != VECSXP || XLENGTH(node) < 2) return 2;
  return node_lookup(node, owner_out, out);
}

const guatiao_alloc *gt_alloc_for(const guatiao_value *node, const gt_owner *o) {
  const guatiao_alloc *rec = NULL;
  if (node->tag == GUATIAO_MAP) rec = node->payload.map.alloc;
  else if (node->tag == GUATIAO_LIST) rec = node->payload.list.alloc;
  if (rec) return rec;
  return o && o->alloc ? o->alloc : &gt_binding_alloc;
}

const guatiao_alloc *gt_alloc_arg(SEXP a) {
  if (a == R_NilValue) return &gt_binding_alloc;
  if (TYPEOF(a) != EXTPTRSXP || !R_ExternalPtrAddr(a)) gt_raise(GUATIAO_ERR_WRONG_KIND, "alloc is an allocator");
  return (const guatiao_alloc *)R_ExternalPtrAddr(a);
}
