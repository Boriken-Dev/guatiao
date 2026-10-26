/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* JSON, TOML and YAML through the guatiao_serde library, over the C table
 * the guatiao package registers. */

#include <string.h>

#define R_NO_REMAP
#define STRICT_R_HEADERS
#include <R.h>
#include <R_ext/Rdynload.h>
#include <Rinternals.h>

#include "guatiao.h"
#include "guatiao_serde.h"
#include <guatiao_r_api.h>

static const guatiao_r_api *api;

typedef guatiao_status (*parse_fn)(guatiao_str, uint32_t, const guatiao_alloc *, guatiao_value *,
                                   guatiao_parse_error *);
typedef guatiao_status (*emit_fn)(const guatiao_value *, uint32_t, const guatiao_alloc *, guatiao_value *);
typedef guatiao_status (*from_wire_fn)(guatiao_bytes, uint32_t, const guatiao_alloc *, guatiao_value *);

/* An export of guatiao_serde, or the missing-symbol condition naming its
 * feature. */
static void *serde_fn(const char *name, const char *feature) {
  void *f = api->export_of("guatiao_serde", name);
  if (!f) api->missing("guatiao_serde", name, feature);
  return f;
}

/* A refused document answers list(c(format, line, column), message) of
 * this class; the R side signals the condition. `fn` writes a tree, or
 * the document's wire bytes (`*_to_wire`), which `numbers` >= 0 reads as
 * R data. */
static SEXP parse_with(parse_fn fn, SEXP text, SEXP how, const guatiao_alloc *a, SEXP alloc, int numbers) {
  guatiao_str t = api->str_arg(text, "the document");
  guatiao_value out;
  memset(&out, 0, sizeof out);
  guatiao_parse_error e;
  memset(&e, 0, sizeof e);
  guatiao_status st = fn(t, (uint32_t)Rf_asInteger(how), a, &out, &e);
  if (st == GUATIAO_ERR_BAD_VALUE) {
    SEXP info = PROTECT(Rf_allocVector(REALSXP, 3));
    REAL(info)[0] = e.format;
    REAL(info)[1] = e.line;
    REAL(info)[2] = e.column;
    SEXP mc = api->mkchar(guatiao_string_text(&e.message));
    SEXP msg = PROTECT(mc ? Rf_ScalarString(mc) : Rf_mkString(""));
    api->free(&e.message);
    api->free(&out);
    SEXP refusal = PROTECT(Rf_allocVector(VECSXP, 2));
    SET_VECTOR_ELT(refusal, 0, info);
    SET_VECTOR_ELT(refusal, 1, msg);
    Rf_setAttrib(refusal, R_ClassSymbol, Rf_mkString("guatiao_serde_refusal"));
    UNPROTECT(3);
    return refusal;
  }
  api->free(&e.message);
  if (st) {
    api->free(&out);
    api->raise(st, "the document was refused");
  }
  if (numbers >= 0) return api->read_wire_and_free(&out, numbers);
  return api->value_new(&out, a, alloc);
}

SEXP C_serde_parse(SEXP format, SEXP text, SEXP how, SEXP alloc) {
  int f = Rf_asInteger(format);
  parse_fn fn = (parse_fn)(f == 0   ? serde_fn("guatiao_json_parse_ex", "")
                           : f == 1 ? serde_fn("guatiao_toml_parse_ex", "toml")
                                    : serde_fn("guatiao_yaml_parse_ex", "yaml"));
  return parse_with(fn, text, how, api->alloc_arg(alloc), alloc, -1);
}

/* The document straight to R data: its wire bytes, written by the library
 * in one call, read here; no tree is built. */
SEXP C_serde_data(SEXP format, SEXP text, SEXP how, SEXP numbers) {
  int f = Rf_asInteger(format);
  parse_fn fn = (parse_fn)(f == 0   ? serde_fn("guatiao_json_to_wire", "")
                           : f == 1 ? serde_fn("guatiao_toml_to_wire", "toml")
                                    : serde_fn("guatiao_yaml_to_wire", "yaml"));
  return parse_with(fn, text, how, api->binding_alloc, R_NilValue, Rf_asInteger(numbers));
}

/* R data as wire bytes written here, and the document from them in one
 * call: no tree is built. */
static SEXP emit_data(int f, SEXP x, SEXP how, SEXP pretty) {
  from_wire_fn fn = (from_wire_fn)(f == 0   ? serde_fn("guatiao_json_from_wire", "")
                                   : f == 1 ? serde_fn("guatiao_toml_from_wire", "toml")
                                            : serde_fn("guatiao_yaml_from_wire", "yaml"));
  char msg[1024];
  guatiao_bytes b;
  guatiao_status st = api->wire_of(x, &b, msg, sizeof msg);
  if (st) api->raise(st, msg);
  uint32_t h = (uint32_t)Rf_asInteger(how) | (Rf_asLogical(pretty) == 1 ? GUATIAO_PRETTY : 0);
  guatiao_value out;
  memset(&out, 0, sizeof out);
  st = fn(b, h, api->binding_alloc, &out);
  if (st) {
    api->free(&out);
    api->raise(st, f == 1 ? "TOML writes only a map, and no null" : "the value cannot be written");
  }
  return api->take_text(&out);
}

SEXP C_serde_emit(SEXP format, SEXP x, SEXP how, SEXP pretty) {
  int f = Rf_asInteger(format);
  if (!Rf_inherits(x, "guatiao_node")) return emit_data(f, x, how, pretty);
  emit_fn fn = (emit_fn)(f == 0 ? (Rf_asLogical(pretty) ? serde_fn("guatiao_json_emit_pretty", "")
                                                        : serde_fn("guatiao_json_emit", ""))
                         : f == 1 ? serde_fn("guatiao_toml_emit", "toml")
                                  : serde_fn("guatiao_yaml_emit", "yaml"));
  guatiao_value tmp;
  const guatiao_value *v = api->tree_arg(x, &tmp);
  guatiao_value out;
  memset(&out, 0, sizeof out);
  guatiao_status st = fn(v, (uint32_t)Rf_asInteger(how), api->binding_alloc, &out);
  api->free(&tmp);
  if (st) {
    api->free(&out);
    api->raise(st, f == 1 ? "TOML writes only a map, and no null" : "the value cannot be written");
  }
  return api->take_text(&out);
}

static const R_CallMethodDef calls[] = {
    {"C_serde_data", (DL_FUNC)(void (*)(void)) & C_serde_data, 4},
    {"C_serde_emit", (DL_FUNC)(void (*)(void)) & C_serde_emit, 4},
    {"C_serde_parse", (DL_FUNC)(void (*)(void)) & C_serde_parse, 4},
    {NULL, NULL, 0}};

void R_init_guatiao_serde(DllInfo *dll) {
  api = ((const guatiao_r_api *(*)(void))R_GetCCallable("guatiao", "guatiao_r_api"))();
  if (api->size != sizeof(guatiao_r_api))
    Rf_error("the guatiao package's table is %d bytes, and guatiao.serde was built against one of %d: install the two "
             "at one version",
             (int)api->size, (int)sizeof(guatiao_r_api));
  R_registerRoutines(dll, NULL, calls, NULL, NULL);
  R_useDynamicSymbols(dll, FALSE);
  R_forceSymbols(dll, TRUE);
}
