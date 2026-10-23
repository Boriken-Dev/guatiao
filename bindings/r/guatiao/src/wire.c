/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Values as bytes (the wire encoding) and channels: the frame builders
 * and a receiver. A receiver answers R the facts of one frame; R/wire.R
 * makes the event or signals the refusal. */

#include <stdlib.h>
#include <string.h>

#include "glue.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* A raw vector of a BYTES value's bytes; `v` is freed whether or not the
 * copy is made. */
static SEXP take_bytes(guatiao_value *v) {
  guatiao_bytes b = guatiao_bytes_data(v);
  if (b.len > (size_t)R_XLEN_T_MAX) {
    gt_free(v);
    gt_raise(GUATIAO_ERR_BAD_VALUE, "bytes too long for an R raw vector");
  }
  SEXP x = Rf_allocVector(RAWSXP, (R_xlen_t)b.len);
  if (b.len) memcpy(RAW(x), b.ptr, b.len);
  gt_free(v);
  return x;
}

static guatiao_bytes raw_arg(SEXP x, const char *what) {
  if (TYPEOF(x) != RAWSXP) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s is a raw vector", what);
  return guatiao_bytes_from(XLENGTH(x) ? RAW(x) : NULL, (size_t)XLENGTH(x));
}

static uint64_t channel_arg(SEXP x) {
  gt_wide w = {0, 0, 0};
  uint64_t c = 0;
  const char *why = gt_wide_in(x, &w);
  if (!why) why = gt_wide_unsigned(w, 64, &c);
  if (why) gt_raise(GUATIAO_ERR_BAD_VALUE, "the channel: %s", why);
  return c;
}

SEXP C_wire_encode(SEXP x) {
  guatiao_value tmp, out;
  set_absent(&out);
  const guatiao_value *v = gt_tree_arg(x, &tmp);
  guatiao_status st = GT_CORE_FN(guatiao_wire_encode)(&gt_binding_alloc, v, &out);
  gt_free(&tmp);
  if (st) {
    gt_free(&out);
    gt_raise(st, "the value has a node the wire encoding cannot carry");
  }
  return take_bytes(&out);
}

SEXP C_wire_decode(SEXP bytes, SEXP alloc) {
  const guatiao_alloc *a = gt_alloc_arg(alloc);
  guatiao_bytes b = raw_arg(bytes, "the bytes");
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = GT_CORE_FN(guatiao_wire_decode)(a, b, &out);
  if (st) gt_raise(st, "bytes that are not a value in the wire encoding");
  SEXP owner = gt_owner_new(&out, a, alloc);
  if (!owner) {
    gt_free(&out);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  return owner;
}

/* `kind`: 0 announce (`x` the schema), 1 send (`x` the value), 2 close. */
SEXP C_wire_frame(SEXP kind, SEXP channel, SEXP x) {
  int k = Rf_asInteger(kind);
  uint64_t c = channel_arg(channel);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  if (k == 2) {
    st = GT_CORE_FN(guatiao_wire_close)(&gt_binding_alloc, c, &out);
  } else {
    guatiao_value tmp;
    const guatiao_value *v = gt_tree_arg(x, &tmp);
    st = k == 0 ? GT_CORE_FN(guatiao_wire_announce)(&gt_binding_alloc, c, v, &out)
                : GT_CORE_FN(guatiao_wire_send)(&gt_binding_alloc, c, v, &out);
    gt_free(&tmp);
  }
  if (st) {
    gt_free(&out);
    gt_raise(st, "the frame could not be written");
  }
  return take_bytes(&out);
}

/* ---- the receiver ------------------------------------------------------------- */

static void receiver_finalize(SEXP x) {
  guatiao_wire_receiver *r = (guatiao_wire_receiver *)R_ExternalPtrAddr(x);
  if (!r) return;
  if (gt_core.guatiao_wire_receiver_free) gt_core.guatiao_wire_receiver_free(r);
  R_ClearExternalPtr(x);
}

static guatiao_wire_receiver *receiver_of(SEXP x) {
  guatiao_wire_receiver *r = TYPEOF(x) == EXTPTRSXP ? (guatiao_wire_receiver *)R_ExternalPtrAddr(x) : NULL;
  if (!r) {
    gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    Rf_error("closed");
  }
  return r;
}

/* The receiver builds in the binding's allocator, which lives in this
 * image for the process: it outlives every receiver. */
SEXP C_wire_receiver_new(void) {
  guatiao_wire_receiver *r = GT_CORE_FN(guatiao_wire_receiver_new)(&gt_binding_alloc);
  if (!r) gt_raise(GUATIAO_ERR_ALLOC, "the receiver was refused");
  SEXP x = PROTECT(R_MakeExternalPtr(r, R_NilValue, R_NilValue));
  R_RegisterCFinalizerEx(x, receiver_finalize, TRUE);
  UNPROTECT(1);
  return x;
}

SEXP C_wire_receiver_free(SEXP x) {
  receiver_finalize(x);
  return R_NilValue;
}

/* One frame: list(status, refusal, kind, channel, at, message, owner,
 * detail). `owner` holds the event's value (the schema's copy or the
 * checked value) or is NULL; `detail` is the refused field's map as R
 * data, or NULL. */
SEXP C_wire_accept(SEXP x, SEXP frame) {
  guatiao_wire_receiver *r = receiver_of(x);
  guatiao_bytes f = raw_arg(frame, "a frame");
  guatiao_wire_event e;
  guatiao_status st = GT_CORE_FN(guatiao_wire_receiver_accept)(r, f, &e);

  /* The words, copied out and freed before anything can signal. */
  char msg[2048];
  guatiao_str t = guatiao_string_text(&e.message);
  size_t n = t.len < sizeof msg - 1 ? t.len : sizeof msg - 1;
  if (n) memcpy(msg, t.ptr, n);
  msg[n] = '\0';
  for (size_t i = 0; i < n; i++)
    if (!msg[i]) msg[i] = '?';
  gt_free(&e.message);

  SEXP owner = R_NilValue;
  if (st == GUATIAO_OK && guatiao_tag_of(&e.value) != GUATIAO_ABSENT) {
    owner = gt_owner_new(&e.value, &gt_binding_alloc, R_NilValue);
    if (!owner) {
      gt_free(&e.value);
      gt_free(&e.detail);
      gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
    }
    set_absent(&e.value); /* moved into the owner */
  } else {
    gt_free(&e.value);
  }
  PROTECT(owner);
  /* Frees the detail whether reading it finishes or signals; the owner,
   * protected, is its finalizer's to free if it does. */
  SEXP detail = PROTECT(gt_read_and_free(&e.detail, 1));

  SEXP out = PROTECT(Rf_allocVector(VECSXP, 8));
  SET_VECTOR_ELT(out, 0, Rf_ScalarInteger((int)st));
  SET_VECTOR_ELT(out, 1, Rf_ScalarInteger((int)e.refusal));
  SET_VECTOR_ELT(out, 2, Rf_ScalarInteger((int)e.kind));
  SET_VECTOR_ELT(out, 3, gt_u64_sexp(e.channel, GT_INT64_AUTO));
  SET_VECTOR_ELT(out, 4, Rf_ScalarReal((double)e.at));
  SEXP words = PROTECT(Rf_mkCharLenCE(msg, (int)n, CE_UTF8));
  SET_VECTOR_ELT(out, 5, Rf_ScalarString(words));
  SET_VECTOR_ELT(out, 6, owner);
  SET_VECTOR_ELT(out, 7, detail);
  UNPROTECT(4);
  return out;
}
