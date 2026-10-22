/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Forms, paths and the flat store through the guatiao_intake library, over
 * the C table the guatiao package registers. */

#include <string.h>

#define R_NO_REMAP
#define STRICT_R_HEADERS
#include <R.h>
#include <R_ext/Rdynload.h>
#include <Rinternals.h>

#include "guatiao.h"
#include "guatiao_intake.h"
#include <guatiao_r_api.h>

static const guatiao_r_api *api;

/* An export of guatiao_intake, or the missing-symbol condition. */
static void *intake_fn(const char *name) {
  void *f = api->export_of("guatiao_intake", name);
  if (!f) api->missing("guatiao_intake", name, "");
  return f;
}

#define INTAKE(name) ((__typeof__(&name))intake_fn(#name))

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* Answers a tree an intake call built: plain data, freed. */
static SEXP answer_plain(guatiao_status st, guatiao_value *out, const char *what) {
  if (st) {
    api->free(out);
    api->raise(st, what);
  }
  return api->read_and_free(out, 0);
}

/* Answers a tree an intake call built as a value; NULL for null or absent. */
static SEXP answer_value(guatiao_status st, guatiao_value *out, const char *what) {
  if (st) {
    api->free(out);
    api->raise(st, what);
  }
  if (out->tag == GUATIAO_NULL || out->tag == GUATIAO_ABSENT) return R_NilValue;
  return api->value_new(out, api->binding_alloc, R_NilValue);
}

SEXP C_intake_check(SEXP schema, SEXP form) {
  guatiao_value ts, tf, out;
  set_absent(&out);
  const guatiao_value *s, *f;
  api->tree_args(schema, &ts, &s, form, &tf, &f, R_NilValue, NULL, NULL);
  guatiao_status st = INTAKE(guatiao_intake_check)(s, f, api->binding_alloc, &out);
  api->free(&ts);
  api->free(&tf);
  if (!st) {
    api->free(&out);
    return R_NilValue;
  }
  if (st == GUATIAO_ERR_BAD_VALUE && out.tag == GUATIAO_MAP) return api->read_and_free(&out, 0);
  return answer_plain(st, &out, "the form could not be checked");
}

SEXP C_intake_layout(SEXP schema, SEXP form) {
  guatiao_value ts, tf, out;
  set_absent(&out);
  const guatiao_value *s, *f;
  api->tree_args(schema, &ts, &s, form, &tf, &f, R_NilValue, NULL, NULL);
  guatiao_status st = INTAKE(guatiao_intake_layout)(s, f, api->binding_alloc, &out);
  api->free(&ts);
  api->free(&tf);
  return answer_plain(st, &out, "the form was refused");
}

SEXP C_intake_is_visible(SEXP schema, SEXP form, SEXP key, SEXP values) {
  guatiao_value ts, tf, tv;
  bool out = false;
  guatiao_str k = api->str_arg(key, "the key");
  const guatiao_value *s, *f, *v;
  api->tree_args(schema, &ts, &s, form, &tf, &f, values, &tv, &v);
  guatiao_status st = INTAKE(guatiao_intake_is_visible)(s, f, k, v, &out);
  api->free(&ts);
  api->free(&tf);
  api->free(&tv);
  if (st) api->raise(st, "visibility could not be judged");
  return Rf_ScalarLogical(out);
}

SEXP C_intake_for_schema(SEXP schema) {
  guatiao_value ts, out;
  set_absent(&out);
  const guatiao_value *s = api->tree_arg(schema, &ts);
  guatiao_status st = INTAKE(guatiao_intake_for_schema)(s, api->binding_alloc, &out);
  api->free(&ts);
  if (st) {
    api->free(&out);
    api->raise(st, "no form for that schema");
  }
  return api->value_new(&out, api->binding_alloc, R_NilValue);
}

SEXP C_intake_form_for(SEXP form, SEXP schema, SEXP path) {
  guatiao_value tf, ts, out;
  set_absent(&out);
  guatiao_str p = api->str_arg(path, "the path");
  const guatiao_value *f, *s;
  api->tree_args(form, &tf, &f, schema, &ts, &s, R_NilValue, NULL, NULL);
  guatiao_status st = INTAKE(guatiao_intake_form_for)(f, s, p, api->binding_alloc, &out);
  api->free(&tf);
  api->free(&ts);
  return answer_value(st, &out, "no form at that path");
}

SEXP C_intake_at(SEXP value, SEXP path) {
  guatiao_value tmp;
  guatiao_str p = api->str_arg(path, "the path");
  const guatiao_value *v = api->tree_arg(value, &tmp);
  const guatiao_value *found = INTAKE(guatiao_intake_path_get)(v, p);
  return api->view_at(value, v, found, &tmp);
}

SEXP C_intake_resolve(SEXP schema, SEXP key) {
  guatiao_value tmp;
  guatiao_str k = api->str_arg(key, "the key");
  const guatiao_value *s = api->tree_arg(schema, &tmp);
  const guatiao_value *found = INTAKE(guatiao_intake_resolve)(s, k);
  return api->view_at(schema, s, found, &tmp);
}

SEXP C_intake_flat_keys(SEXP schema, SEXP key) {
  guatiao_value ts, out;
  set_absent(&out);
  guatiao_str k = api->str_arg(key, "the key");
  const guatiao_value *s = api->tree_arg(schema, &ts);
  guatiao_status st = INTAKE(guatiao_intake_flat_keys)(s, k, api->binding_alloc, &out);
  api->free(&ts);
  return answer_plain(st, &out, "no such key in the schema");
}

SEXP C_intake_flatten(SEXP schema, SEXP key, SEXP value) {
  guatiao_value ts, tv, out;
  set_absent(&out);
  guatiao_str k = api->str_arg(key, "the key");
  const guatiao_value *s, *v;
  api->tree_args(schema, &ts, &s, value, &tv, &v, R_NilValue, NULL, NULL);
  guatiao_status st = INTAKE(guatiao_intake_flatten)(s, k, v, api->binding_alloc, &out);
  api->free(&ts);
  api->free(&tv);
  return answer_plain(st, &out, "the value does not fit the schema");
}

SEXP C_intake_unflatten(SEXP schema, SEXP key, SEXP store) {
  guatiao_value ts, tv, out;
  set_absent(&out);
  guatiao_str k = api->str_arg(key, "the key");
  const guatiao_value *s, *v;
  api->tree_args(schema, &ts, &s, store, &tv, &v, R_NilValue, NULL, NULL);
  guatiao_status st = INTAKE(guatiao_intake_unflatten)(s, k, v, api->binding_alloc, &out);
  api->free(&ts);
  api->free(&tv);
  if (st == GUATIAO_ERR_WRONG_KIND) {
    api->free(&out);
    return R_NilValue; /* a key that does not apply */
  }
  return answer_value(st, &out, "the store does not fit the schema");
}

static const R_CallMethodDef calls[] = {
    {"C_intake_at", (DL_FUNC)(void (*)(void)) & C_intake_at, 2},
    {"C_intake_check", (DL_FUNC)(void (*)(void)) & C_intake_check, 2},
    {"C_intake_flat_keys", (DL_FUNC)(void (*)(void)) & C_intake_flat_keys, 2},
    {"C_intake_flatten", (DL_FUNC)(void (*)(void)) & C_intake_flatten, 3},
    {"C_intake_for_schema", (DL_FUNC)(void (*)(void)) & C_intake_for_schema, 1},
    {"C_intake_form_for", (DL_FUNC)(void (*)(void)) & C_intake_form_for, 3},
    {"C_intake_is_visible", (DL_FUNC)(void (*)(void)) & C_intake_is_visible, 4},
    {"C_intake_layout", (DL_FUNC)(void (*)(void)) & C_intake_layout, 2},
    {"C_intake_resolve", (DL_FUNC)(void (*)(void)) & C_intake_resolve, 2},
    {"C_intake_unflatten", (DL_FUNC)(void (*)(void)) & C_intake_unflatten, 3},
    {NULL, NULL, 0}};

void R_init_guatiao_intake(DllInfo *dll) {
  api = ((const guatiao_r_api *(*)(void))R_GetCCallable("guatiao", "guatiao_r_api"))();
  if (api->size < sizeof(guatiao_r_api)) Rf_error("the guatiao package is older than guatiao.intake needs");
  R_registerRoutines(dll, NULL, calls, NULL, NULL);
  R_useDynamicSymbols(dll, FALSE);
  R_forceSymbols(dll, TRUE);
}
