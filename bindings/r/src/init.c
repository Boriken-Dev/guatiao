/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The .Call entry points, registered by name; written from the
 * definitions in the other files. */

#include "glue.h"

#include <R_ext/Rdynload.h>

SEXP C_alloc_address(SEXP);
SEXP C_alloc_binding(void);
SEXP C_alloc_counting(void);
SEXP C_alloc_live(SEXP);
SEXP C_date_add_span(SEXP, SEXP);
SEXP C_datetime_add_span(SEXP, SEXP);
SEXP C_duration_read(SEXP);
SEXP C_has_export(SEXP, SEXP);
SEXP C_intake_at(SEXP, SEXP);
SEXP C_intake_check(SEXP, SEXP);
SEXP C_intake_flat_keys(SEXP, SEXP);
SEXP C_intake_flatten(SEXP, SEXP, SEXP);
SEXP C_intake_for_schema(SEXP);
SEXP C_intake_form_for(SEXP, SEXP, SEXP);
SEXP C_intake_is_visible(SEXP, SEXP, SEXP, SEXP);
SEXP C_intake_layout(SEXP, SEXP);
SEXP C_intake_resolve(SEXP, SEXP);
SEXP C_intake_unflatten(SEXP, SEXP, SEXP);
SEXP C_library_is_open(SEXP);
SEXP C_node_alloc_address(SEXP);
SEXP C_node_child(SEXP, SEXP);
SEXP C_node_clear(SEXP);
SEXP C_node_copy(SEXP, SEXP);
SEXP C_node_delete(SEXP, SEXP);
SEXP C_node_insert(SEXP, SEXP, SEXP);
SEXP C_node_length(SEXP);
SEXP C_node_merge(SEXP, SEXP);
SEXP C_node_names(SEXP);
SEXP C_node_pointer(SEXP);
SEXP C_node_push(SEXP, SEXP);
SEXP C_node_read(SEXP, SEXP);
SEXP C_node_set(SEXP, SEXP, SEXP);
SEXP C_node_tag(SEXP);
SEXP C_number_read(SEXP);
SEXP C_open_library(SEXP, SEXP);
SEXP C_redact(SEXP, SEXP, SEXP, SEXP);
SEXP C_schema_validate(SEXP, SEXP);
SEXP C_serde_emit(SEXP, SEXP, SEXP, SEXP);
SEXP C_serde_parse(SEXP, SEXP, SEXP, SEXP);
SEXP C_time_format(SEXP, SEXP);
SEXP C_time_normalize(SEXP, SEXP);
SEXP C_time_parse(SEXP, SEXP, SEXP);
SEXP C_value_close(SEXP);
SEXP C_value_is_open(SEXP);
SEXP C_value_new(SEXP, SEXP);

void gt_kinds_init(void);

static const R_CallMethodDef calls[] = {
    {"C_alloc_address", (DL_FUNC)(void (*)(void)) & C_alloc_address, 1},
    {"C_alloc_binding", (DL_FUNC)(void (*)(void)) & C_alloc_binding, 0},
    {"C_alloc_counting", (DL_FUNC)(void (*)(void)) & C_alloc_counting, 0},
    {"C_alloc_live", (DL_FUNC)(void (*)(void)) & C_alloc_live, 1},
    {"C_date_add_span", (DL_FUNC)(void (*)(void)) & C_date_add_span, 2},
    {"C_datetime_add_span", (DL_FUNC)(void (*)(void)) & C_datetime_add_span, 2},
    {"C_duration_read", (DL_FUNC)(void (*)(void)) & C_duration_read, 1},
    {"C_has_export", (DL_FUNC)(void (*)(void)) & C_has_export, 2},
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
    {"C_library_is_open", (DL_FUNC)(void (*)(void)) & C_library_is_open, 1},
    {"C_node_alloc_address", (DL_FUNC)(void (*)(void)) & C_node_alloc_address, 1},
    {"C_node_child", (DL_FUNC)(void (*)(void)) & C_node_child, 2},
    {"C_node_clear", (DL_FUNC)(void (*)(void)) & C_node_clear, 1},
    {"C_node_copy", (DL_FUNC)(void (*)(void)) & C_node_copy, 2},
    {"C_node_delete", (DL_FUNC)(void (*)(void)) & C_node_delete, 2},
    {"C_node_insert", (DL_FUNC)(void (*)(void)) & C_node_insert, 3},
    {"C_node_length", (DL_FUNC)(void (*)(void)) & C_node_length, 1},
    {"C_node_merge", (DL_FUNC)(void (*)(void)) & C_node_merge, 2},
    {"C_node_names", (DL_FUNC)(void (*)(void)) & C_node_names, 1},
    {"C_node_pointer", (DL_FUNC)(void (*)(void)) & C_node_pointer, 1},
    {"C_node_push", (DL_FUNC)(void (*)(void)) & C_node_push, 2},
    {"C_node_read", (DL_FUNC)(void (*)(void)) & C_node_read, 2},
    {"C_node_set", (DL_FUNC)(void (*)(void)) & C_node_set, 3},
    {"C_node_tag", (DL_FUNC)(void (*)(void)) & C_node_tag, 1},
    {"C_number_read", (DL_FUNC)(void (*)(void)) & C_number_read, 1},
    {"C_open_library", (DL_FUNC)(void (*)(void)) & C_open_library, 2},
    {"C_redact", (DL_FUNC)(void (*)(void)) & C_redact, 4},
    {"C_schema_validate", (DL_FUNC)(void (*)(void)) & C_schema_validate, 2},
    {"C_serde_emit", (DL_FUNC)(void (*)(void)) & C_serde_emit, 4},
    {"C_serde_parse", (DL_FUNC)(void (*)(void)) & C_serde_parse, 4},
    {"C_time_format", (DL_FUNC)(void (*)(void)) & C_time_format, 2},
    {"C_time_normalize", (DL_FUNC)(void (*)(void)) & C_time_normalize, 2},
    {"C_time_parse", (DL_FUNC)(void (*)(void)) & C_time_parse, 3},
    {"C_value_close", (DL_FUNC)(void (*)(void)) & C_value_close, 1},
    {"C_value_is_open", (DL_FUNC)(void (*)(void)) & C_value_is_open, 1},
    {"C_value_new", (DL_FUNC)(void (*)(void)) & C_value_new, 2},
    {NULL, NULL, 0}};

void R_init_guatiao(DllInfo *dll) {
  gt_note_main_thread();
  gt_kinds_init();
  R_registerRoutines(dll, NULL, calls, NULL, NULL);
  R_useDynamicSymbols(dll, FALSE);
  R_forceSymbols(dll, TRUE);
}
