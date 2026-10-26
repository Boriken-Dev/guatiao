/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The C table packages over another guatiao library build on, registered
 * as the C callable "guatiao_r_api" (inst/include/guatiao_r_api.h). */

#include <string.h>

#include "glue.h"

#include <R_ext/Rdynload.h>

#include "../inst/include/guatiao_r_api.h"

static int slot_of(const char *surface) {
  static const char *const names[3] = {"guatiao", "guatiao_serde", "guatiao_intake"};
  for (int i = 0; i < 3; i++)
    if (strcmp(surface, names[i]) == 0) return i;
  Rf_error("guatiao opens no library for the surface %s", surface);
  return -1;
}

static void *api_export_of(const char *surface, const char *name) {
  int lib = slot_of(surface);
  gt_need(lib);
  return gt_symbol_of(lib, name);
}

static void api_missing(const char *surface, const char *name, const char *feature) {
  SEXP l = PROTECT(Rf_ScalarInteger(slot_of(surface)));
  SEXP s = PROTECT(Rf_mkString(name));
  SEXP f = PROTECT(Rf_mkString(feature ? feature : ""));
  gt_call_r(".gt_missing_symbol", 3, l, s, f);
  UNPROTECT(3);
  Rf_error("unreachable: MissingSymbol was not signalled");
}

static void api_raise(guatiao_status status, const char *message) { gt_raise(status, "%s", message); }

static SEXP api_value_new(guatiao_value *root, const guatiao_alloc *alloc, SEXP keep) {
  SEXP owner = gt_owner_new(root, alloc, keep);
  if (!owner) {
    gt_free(root);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  memset(root, 0, sizeof *root);
  PROTECT(owner);
  SEXP v = gt_call_r(".gt_root", 1, owner, R_NilValue, R_NilValue);
  UNPROTECT(1);
  return v;
}

/* Fills `steps` from position `depth` with the path from `root` to
 * `target`; answers 1 when it is there. */
static int path_to(const guatiao_value *root, const guatiao_value *target, SEXP steps, int depth) {
  if (root == target) return 1;
  if (depth >= 128) return 0;
  if (root->tag == GUATIAO_MAP) {
    guatiao_entries es = guatiao_map_entries(root);
    for (size_t i = 0; i < es.len; i++) {
      if (path_to(&es.ptr[i].value, target, steps, depth + 1)) {
        SEXP k = gt_mkchar(guatiao_entry_key(&es.ptr[i]));
        if (!k) return 0;
        SET_VECTOR_ELT(steps, depth, Rf_ScalarString(k));
        return 1;
      }
    }
  } else if (root->tag == GUATIAO_LIST) {
    guatiao_values items = guatiao_list_items(root);
    for (size_t i = 0; i < items.len; i++) {
      if (path_to(&items.ptr[i], target, steps, depth + 1)) {
        SET_VECTOR_ELT(steps, depth, Rf_ScalarInteger((int)i));
        return 1;
      }
    }
  }
  return 0;
}

static SEXP api_view_at(SEXP arg, const guatiao_value *root, const guatiao_value *found, guatiao_value *tmp) {
  if (!found) {
    gt_free(tmp);
    return R_NilValue;
  }
  if (Rf_inherits(arg, "guatiao_node")) {
    SEXP steps = PROTECT(Rf_allocVector(VECSXP, 130));
    if (!path_to(root, found, steps, 0)) {
      UNPROTECT(1);
      return R_NilValue;
    }
    SEXP base = VECTOR_ELT(arg, 1);
    R_xlen_t nb = XLENGTH(base), extra = 0;
    while (extra < 130 && VECTOR_ELT(steps, extra) != R_NilValue) extra++;
    SEXP path = PROTECT(Rf_allocVector(VECSXP, nb + extra));
    for (R_xlen_t i = 0; i < nb; i++) SET_VECTOR_ELT(path, i, VECTOR_ELT(base, i));
    for (R_xlen_t i = 0; i < extra; i++) SET_VECTOR_ELT(path, nb + i, VECTOR_ELT(steps, i));
    SEXP view = gt_call_r(".gt_node", 2, VECTOR_ELT(arg, 0), path, R_NilValue);
    UNPROTECT(2);
    return view;
  }
  guatiao_value copy;
  memset(&copy, 0, sizeof copy);
  guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, found, &copy);
  gt_free(tmp);
  if (st) gt_raise(st, "the copy was refused");
  return api_value_new(&copy, &gt_binding_alloc, R_NilValue);
}

static const guatiao_r_api gt_r_api = {
    sizeof(guatiao_r_api),
    api_export_of,
    api_missing,
    api_raise,
    &gt_binding_alloc,
    gt_alloc_arg,
    gt_str_arg,
    gt_mkchar,
    gt_tree_arg,
    gt_tree_args,
    gt_free,
    api_value_new,
    gt_read_and_free,
    gt_take_text,
    api_view_at,
    gt_wire_of,
    gt_read_wire_and_free,
};

static const guatiao_r_api *gt_r_api_table(void) { return &gt_r_api; }

void gt_api_register(void) {
  R_RegisterCCallable("guatiao", "guatiao_r_api", (DL_FUNC)(void (*)(void)) & gt_r_api_table);
}
