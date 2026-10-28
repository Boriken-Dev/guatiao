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
SEXP C_alloc_refuse(SEXP, SEXP);
SEXP C_answer(SEXP, SEXP);
SEXP C_date_add_span(SEXP, SEXP);
SEXP C_datetime_add_span(SEXP, SEXP);
SEXP C_duration_read(SEXP);
SEXP C_file_carries(SEXP, SEXP);
SEXP C_has_export(SEXP, SEXP);
SEXP C_integer64_text(SEXP);
SEXP C_kind_document_read(SEXP);
SEXP C_kind_new(SEXP, SEXP, SEXP, SEXP, SEXP, SEXP, SEXP, SEXP, SEXP, SEXP, SEXP);
SEXP C_library_close(SEXP);
SEXP C_library_entry(SEXP);
SEXP C_library_info(SEXP);
SEXP C_library_is_open(SEXP);
SEXP C_library_live(SEXP);
SEXP C_library_new(SEXP, SEXP, SEXP, SEXP);
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
SEXP C_pending_destroys(void);
SEXP C_redact(SEXP, SEXP, SEXP, SEXP);
SEXP C_registry_add_source_file(SEXP, SEXP, SEXP);
SEXP C_registry_collect(SEXP);
SEXP C_registry_create(SEXP, SEXP, SEXP);
SEXP C_registry_destroy(SEXP, SEXP, SEXP);
SEXP C_registry_free(SEXP);
SEXP C_registry_host(SEXP);
SEXP C_registry_import_search(SEXP);
SEXP C_registry_keyed_by(SEXP, SEXP, SEXP);
SEXP C_registry_kind_descriptions(SEXP, SEXP);
SEXP C_registry_load_file(SEXP, SEXP);
SEXP C_registry_new(SEXP, SEXP, SEXP);
SEXP C_registry_priority(SEXP, SEXP);
SEXP C_registry_provider_available(SEXP, SEXP);
SEXP C_registry_provider_config(SEXP, SEXP);
SEXP C_registry_provider_meta(SEXP, SEXP);
SEXP C_registry_provider_table(SEXP, SEXP, SEXP);
SEXP C_registry_query(SEXP, SEXP, SEXP);
SEXP C_registry_refresh(SEXP);
SEXP C_registry_register_entry(SEXP, SEXP, SEXP);
SEXP C_registry_retire(SEXP, SEXP);
SEXP C_registry_scan(SEXP, SEXP, SEXP, SEXP, SEXP);
SEXP C_registry_set_import_search(SEXP, SEXP);
SEXP C_registry_set_priority(SEXP, SEXP, SEXP);
SEXP C_registry_unload(SEXP, SEXP, SEXP);
SEXP C_registry_watch_dir(SEXP, SEXP, SEXP);
SEXP C_remote_call(SEXP, SEXP, SEXP, SEXP);
SEXP C_remote_close(SEXP);
SEXP C_remote_has(SEXP, SEXP);
SEXP C_remote_live(SEXP);
SEXP C_remote_new(SEXP, SEXP, SEXP, SEXP, SEXP);
SEXP C_schema_validate(SEXP, SEXP);
SEXP C_served_table(SEXP, SEXP, SEXP);
SEXP C_table_floor_hash(SEXP);
SEXP C_time_format(SEXP, SEXP);
SEXP C_time_normalize(SEXP, SEXP);
SEXP C_time_parse(SEXP, SEXP, SEXP);
SEXP C_unit_answer(SEXP, SEXP);
SEXP C_units(SEXP, SEXP, SEXP);
SEXP C_value_close(SEXP);
SEXP C_value_is_open(SEXP);
SEXP C_value_new(SEXP, SEXP);
SEXP C_wire_accept(SEXP, SEXP);
SEXP C_wire_decode(SEXP, SEXP);
SEXP C_wire_encode(SEXP);
SEXP C_wire_frame(SEXP, SEXP, SEXP);
SEXP C_wire_of(SEXP);
SEXP C_wire_read(SEXP, SEXP);
SEXP C_wire_receiver_free(SEXP);
SEXP C_wire_receiver_new(void);
SEXP C_xptr_live(SEXP);

void gt_kinds_init(void);
void gt_api_register(void);

static const R_CallMethodDef calls[] = {
    {"C_alloc_address", (DL_FUNC)(void (*)(void)) & C_alloc_address, 1},
    {"C_alloc_binding", (DL_FUNC)(void (*)(void)) & C_alloc_binding, 0},
    {"C_alloc_counting", (DL_FUNC)(void (*)(void)) & C_alloc_counting, 0},
    {"C_alloc_live", (DL_FUNC)(void (*)(void)) & C_alloc_live, 1},
    {"C_alloc_refuse", (DL_FUNC)(void (*)(void)) & C_alloc_refuse, 2},
    {"C_answer", (DL_FUNC)(void (*)(void)) & C_answer, 2},
    {"C_date_add_span", (DL_FUNC)(void (*)(void)) & C_date_add_span, 2},
    {"C_datetime_add_span", (DL_FUNC)(void (*)(void)) & C_datetime_add_span, 2},
    {"C_duration_read", (DL_FUNC)(void (*)(void)) & C_duration_read, 1},
    {"C_file_carries", (DL_FUNC)(void (*)(void)) & C_file_carries, 2},
    {"C_has_export", (DL_FUNC)(void (*)(void)) & C_has_export, 2},
    {"C_integer64_text", (DL_FUNC)(void (*)(void)) & C_integer64_text, 1},
    {"C_kind_document_read", (DL_FUNC)(void (*)(void)) & C_kind_document_read, 1},
    {"C_kind_new", (DL_FUNC)(void (*)(void)) & C_kind_new, 11},
    {"C_library_close", (DL_FUNC)(void (*)(void)) & C_library_close, 1},
    {"C_library_entry", (DL_FUNC)(void (*)(void)) & C_library_entry, 1},
    {"C_library_info", (DL_FUNC)(void (*)(void)) & C_library_info, 1},
    {"C_library_is_open", (DL_FUNC)(void (*)(void)) & C_library_is_open, 1},
    {"C_library_live", (DL_FUNC)(void (*)(void)) & C_library_live, 1},
    {"C_library_new", (DL_FUNC)(void (*)(void)) & C_library_new, 4},
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
    {"C_pending_destroys", (DL_FUNC)(void (*)(void)) & C_pending_destroys, 0},
    {"C_redact", (DL_FUNC)(void (*)(void)) & C_redact, 4},
    {"C_registry_add_source_file", (DL_FUNC)(void (*)(void)) & C_registry_add_source_file, 3},
    {"C_registry_collect", (DL_FUNC)(void (*)(void)) & C_registry_collect, 1},
    {"C_registry_create", (DL_FUNC)(void (*)(void)) & C_registry_create, 3},
    {"C_registry_destroy", (DL_FUNC)(void (*)(void)) & C_registry_destroy, 3},
    {"C_registry_free", (DL_FUNC)(void (*)(void)) & C_registry_free, 1},
    {"C_registry_host", (DL_FUNC)(void (*)(void)) & C_registry_host, 1},
    {"C_registry_import_search", (DL_FUNC)(void (*)(void)) & C_registry_import_search, 1},
    {"C_registry_keyed_by", (DL_FUNC)(void (*)(void)) & C_registry_keyed_by, 3},
    {"C_registry_kind_descriptions", (DL_FUNC)(void (*)(void)) & C_registry_kind_descriptions, 2},
    {"C_registry_load_file", (DL_FUNC)(void (*)(void)) & C_registry_load_file, 2},
    {"C_registry_new", (DL_FUNC)(void (*)(void)) & C_registry_new, 3},
    {"C_registry_priority", (DL_FUNC)(void (*)(void)) & C_registry_priority, 2},
    {"C_registry_provider_available", (DL_FUNC)(void (*)(void)) & C_registry_provider_available, 2},
    {"C_registry_provider_config", (DL_FUNC)(void (*)(void)) & C_registry_provider_config, 2},
    {"C_registry_provider_meta", (DL_FUNC)(void (*)(void)) & C_registry_provider_meta, 2},
    {"C_registry_provider_table", (DL_FUNC)(void (*)(void)) & C_registry_provider_table, 3},
    {"C_registry_query", (DL_FUNC)(void (*)(void)) & C_registry_query, 3},
    {"C_registry_refresh", (DL_FUNC)(void (*)(void)) & C_registry_refresh, 1},
    {"C_registry_register_entry", (DL_FUNC)(void (*)(void)) & C_registry_register_entry, 3},
    {"C_registry_retire", (DL_FUNC)(void (*)(void)) & C_registry_retire, 2},
    {"C_registry_scan", (DL_FUNC)(void (*)(void)) & C_registry_scan, 5},
    {"C_registry_set_import_search", (DL_FUNC)(void (*)(void)) & C_registry_set_import_search, 2},
    {"C_registry_set_priority", (DL_FUNC)(void (*)(void)) & C_registry_set_priority, 3},
    {"C_registry_unload", (DL_FUNC)(void (*)(void)) & C_registry_unload, 3},
    {"C_registry_watch_dir", (DL_FUNC)(void (*)(void)) & C_registry_watch_dir, 3},
    {"C_remote_call", (DL_FUNC)(void (*)(void)) & C_remote_call, 4},
    {"C_remote_close", (DL_FUNC)(void (*)(void)) & C_remote_close, 1},
    {"C_remote_has", (DL_FUNC)(void (*)(void)) & C_remote_has, 2},
    {"C_remote_live", (DL_FUNC)(void (*)(void)) & C_remote_live, 1},
    {"C_remote_new", (DL_FUNC)(void (*)(void)) & C_remote_new, 5},
    {"C_schema_validate", (DL_FUNC)(void (*)(void)) & C_schema_validate, 2},
    {"C_served_table", (DL_FUNC)(void (*)(void)) & C_served_table, 3},
    {"C_table_floor_hash", (DL_FUNC)(void (*)(void)) & C_table_floor_hash, 1},
    {"C_time_format", (DL_FUNC)(void (*)(void)) & C_time_format, 2},
    {"C_time_normalize", (DL_FUNC)(void (*)(void)) & C_time_normalize, 2},
    {"C_time_parse", (DL_FUNC)(void (*)(void)) & C_time_parse, 3},
    {"C_unit_answer", (DL_FUNC)(void (*)(void)) & C_unit_answer, 2},
    {"C_units", (DL_FUNC)(void (*)(void)) & C_units, 3},
    {"C_value_close", (DL_FUNC)(void (*)(void)) & C_value_close, 1},
    {"C_value_is_open", (DL_FUNC)(void (*)(void)) & C_value_is_open, 1},
    {"C_value_new", (DL_FUNC)(void (*)(void)) & C_value_new, 2},
    {"C_wire_accept", (DL_FUNC)(void (*)(void)) & C_wire_accept, 2},
    {"C_wire_decode", (DL_FUNC)(void (*)(void)) & C_wire_decode, 2},
    {"C_wire_encode", (DL_FUNC)(void (*)(void)) & C_wire_encode, 1},
    {"C_wire_frame", (DL_FUNC)(void (*)(void)) & C_wire_frame, 3},
    {"C_wire_of", (DL_FUNC)(void (*)(void)) & C_wire_of, 1},
    {"C_wire_read", (DL_FUNC)(void (*)(void)) & C_wire_read, 2},
    {"C_wire_receiver_free", (DL_FUNC)(void (*)(void)) & C_wire_receiver_free, 1},
    {"C_wire_receiver_new", (DL_FUNC)(void (*)(void)) & C_wire_receiver_new, 0},
    {"C_xptr_live", (DL_FUNC)(void (*)(void)) & C_xptr_live, 1},
    {NULL, NULL, 0}};

void R_init_guatiao(DllInfo *dll) {
  gt_note_main_thread();
  gt_kinds_init();
  gt_api_register();
  R_registerRoutines(dll, NULL, calls, NULL, NULL);
  R_useDynamicSymbols(dll, FALSE);
  R_forceSymbols(dll, TRUE);
}
