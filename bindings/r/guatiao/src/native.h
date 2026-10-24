/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The exports this package calls, resolved at run time from the library a
 * session names: one function pointer per export in a table, and a
 * `static inline` caller per export. The other surfaces (guatiao_serde,
 * guatiao_intake) are opened here too, by slot, for the packages over
 * them, which resolve their own exports through `gt_symbol_of`.
 *
 * Each list entry is X(return type, name, parameters, arguments, feature):
 * a feature of "" is an export every build has, checked when the library
 * is opened; any other is reported missing by name when it is first used.
 * This file includes no R header, so the loader can include <windows.h>.
 */

#ifndef GT_R_NATIVE_H
#define GT_R_NATIVE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "guatiao.h"

#define GT_TEXT_FORMS(X, form)                                                                  \
  X(guatiao_status, guatiao_##form##_parse,                                                    \
    (const guatiao_alloc *alloc, guatiao_str text, guatiao_##form *out, guatiao_text_error *e), \
    (alloc, text, out, e), "")                                                                  \
  X(guatiao_status, guatiao_##form##_format,                                                   \
    (const guatiao_alloc *alloc, const guatiao_##form *f, guatiao_value *out), (alloc, f, out), "") \
  X(guatiao_status, guatiao_##form##_normalize,                                                \
    (const guatiao_alloc *alloc, guatiao_str text, guatiao_value *out, guatiao_text_error *e),   \
    (alloc, text, out, e), "")

#define GT_CORE(X)                                                                              \
  X(guatiao_status, guatiao_value_free, (guatiao_value *v), (v), "")                           \
  X(guatiao_status, guatiao_value_clone,                                                       \
    (const guatiao_alloc *alloc, const guatiao_value *src, guatiao_value *out), (alloc, src, out), "") \
  X(guatiao_status, guatiao_value_make_local,                                                  \
    (guatiao_value *v, const guatiao_alloc *alloc), (v, alloc), "make-local")                   \
  X(guatiao_status, guatiao_value_null, (guatiao_value *out), (out), "")                       \
  X(guatiao_status, guatiao_value_absent, (guatiao_value *out), (out), "")                     \
  X(guatiao_status, guatiao_value_bool, (bool b, guatiao_value *out), (b, out), "")            \
  X(guatiao_status, guatiao_value_map, (const guatiao_alloc *alloc, guatiao_value *out), (alloc, out), "") \
  X(guatiao_status, guatiao_value_list, (const guatiao_alloc *alloc, guatiao_value *out), (alloc, out), "") \
  X(guatiao_status, guatiao_value_string,                                                      \
    (const guatiao_alloc *alloc, guatiao_str text, guatiao_value *out), (alloc, text, out), "") \
  X(guatiao_status, guatiao_value_number,                                                      \
    (const guatiao_alloc *alloc, guatiao_str text, guatiao_value *out), (alloc, text, out), "") \
  X(guatiao_status, guatiao_value_bytes,                                                       \
    (const guatiao_alloc *alloc, guatiao_bytes data, guatiao_value *out), (alloc, data, out), "") \
  X(guatiao_status, guatiao_map_set,                                                           \
    (const guatiao_alloc *alloc, guatiao_value *node, guatiao_str key, guatiao_value *value),   \
    (alloc, node, key, value), "")                                                              \
  X(guatiao_status, guatiao_map_discard, (guatiao_value *node, guatiao_str key), (node, key), "") \
  X(guatiao_status, guatiao_map_clear, (guatiao_value *node), (node), "")                      \
  X(guatiao_status, guatiao_map_copy_from,                                                     \
    (const guatiao_alloc *alloc, guatiao_value *dst, const guatiao_value *src), (alloc, dst, src), "") \
  X(guatiao_status, guatiao_list_push,                                                         \
    (const guatiao_alloc *alloc, guatiao_value *node, guatiao_value *value), (alloc, node, value), "") \
  X(guatiao_status, guatiao_list_discard, (guatiao_value *node, size_t index), (node, index), "") \
  X(guatiao_status, guatiao_list_clear, (guatiao_value *node), (node), "")                     \
  X(guatiao_status, guatiao_schema_validate,                                                   \
    (const guatiao_value *schema, const guatiao_value *config, const guatiao_alloc *alloc,      \
     guatiao_value *out_error),                                                                 \
    (schema, config, alloc, out_error), "")                                                     \
  X(guatiao_status, guatiao_schema_redact,                                                     \
    (const guatiao_value *schema, const guatiao_value *value, guatiao_str placeholder,          \
     const guatiao_alloc *alloc, guatiao_value *out),                                           \
    (schema, value, placeholder, alloc, out), "")                                               \
  GT_TEXT_FORMS(X, date)                                                                        \
  GT_TEXT_FORMS(X, time)                                                                        \
  GT_TEXT_FORMS(X, datetime)                                                                    \
  GT_TEXT_FORMS(X, timestamp)                                                                   \
  GT_TEXT_FORMS(X, span)                                                                        \
  X(guatiao_status, guatiao_date_add_span,                                                     \
    (const guatiao_date *date, const guatiao_span *span, guatiao_date *out), (date, span, out), "") \
  X(guatiao_status, guatiao_datetime_add_span,                                                 \
    (const guatiao_datetime *at, const guatiao_span *span, guatiao_datetime *out), (at, span, out), "") \
  X(guatiao_status, guatiao_duration_read,                                                     \
    (const guatiao_value *value, uint64_t *seconds, uint32_t *nanoseconds),                     \
    (value, seconds, nanoseconds), "")                                                          \
  X(guatiao_status, guatiao_wire_encode,                                                         \
    (const guatiao_alloc *alloc, const guatiao_value *value, guatiao_value *out), (alloc, value, out), "") \
  X(guatiao_status, guatiao_wire_decode,                                                         \
    (const guatiao_alloc *alloc, guatiao_bytes bytes, guatiao_value *out), (alloc, bytes, out), "") \
  X(guatiao_status, guatiao_wire_announce,                                                       \
    (const guatiao_alloc *alloc, uint64_t channel, const guatiao_value *schema, guatiao_value *out), \
    (alloc, channel, schema, out), "")                                                           \
  X(guatiao_status, guatiao_wire_send,                                                           \
    (const guatiao_alloc *alloc, uint64_t channel, const guatiao_value *value, guatiao_value *out), \
    (alloc, channel, value, out), "")                                                            \
  X(guatiao_status, guatiao_wire_close, (const guatiao_alloc *alloc, uint64_t channel, guatiao_value *out), \
    (alloc, channel, out), "")                                                                   \
  X(guatiao_wire_receiver *, guatiao_wire_receiver_new, (const guatiao_alloc *alloc), (alloc), "") \
  X(guatiao_status, guatiao_wire_receiver_accept,                                                \
    (guatiao_wire_receiver *recv, guatiao_bytes frame, guatiao_wire_event *out_event),           \
    (recv, frame, out_event), "")                                                                \
  GT_VOID(guatiao_wire_receiver_free, (guatiao_wire_receiver *recv), (recv), "")                 \
  X(guatiao_registry *, guatiao_registry_new,                                                  \
    (guatiao_str id, guatiao_str version, const guatiao_alloc *alloc), (id, version, alloc), "load") \
  GT_VOID(guatiao_registry_free, (guatiao_registry *reg), (reg), "load")                        \
  X(const guatiao_host_info *, guatiao_registry_host, (guatiao_registry *reg), (reg), "load")   \
  X(guatiao_status, guatiao_registry_keyed_by, (guatiao_registry *reg, guatiao_str t), (reg, t), "load") \
  X(guatiao_status, guatiao_registry_libraries_keyed_by, (guatiao_registry *reg, guatiao_str t), \
    (reg, t), "load")                                                                           \
  X(guatiao_status, guatiao_registry_load_file,                                                \
    (guatiao_registry *reg, guatiao_str path, const guatiao_alloc *alloc, guatiao_value *out),  \
    (reg, path, alloc, out), "load")                                                            \
  X(guatiao_status, guatiao_registry_register_entry,                                           \
    (guatiao_registry *reg, guatiao_str name,                                                   \
     const guatiao_library_info *(*entry)(const guatiao_host_info *), const guatiao_alloc *alloc, \
     guatiao_value *out),                                                                       \
    (reg, name, entry, alloc, out), "load")                                                     \
  X(guatiao_status, guatiao_registry_add_source_file,                                          \
    (guatiao_registry *reg, guatiao_str path, const guatiao_value *config, guatiao_provider_error *err), \
    (reg, path, config, err), "load")                                                           \
  X(guatiao_status, guatiao_registry_watch_dir, (guatiao_registry *reg, guatiao_str dir, guatiao_str rules), \
    (reg, dir, rules), "load")                                                                  \
  X(guatiao_status, guatiao_registry_refresh,                                                  \
    (guatiao_registry *reg, const guatiao_alloc *alloc, guatiao_value *out), (reg, alloc, out), "load") \
  X(guatiao_status, guatiao_registry_scan_dir_rules,                                           \
    (guatiao_registry *reg, guatiao_str dir, bool descending, guatiao_str rules,                \
     const guatiao_alloc *alloc, guatiao_value *out),                                           \
    (reg, dir, descending, rules, alloc, out), "load")                                          \
  X(guatiao_status, guatiao_registry_scan_path,                                                \
    (guatiao_registry *reg, guatiao_str spec, bool descending, guatiao_str rules,               \
     const guatiao_alloc *alloc, guatiao_value *out),                                           \
    (reg, spec, descending, rules, alloc, out), "load")                                         \
  X(guatiao_status, guatiao_registry_libraries,                                                \
    (const guatiao_registry *reg, const guatiao_alloc *alloc, guatiao_value *out), (reg, alloc, out), "load") \
  X(guatiao_status, guatiao_registry_providers,                                                \
    (const guatiao_registry *reg, guatiao_str kind, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, kind, alloc, out), "load")                                                            \
  X(guatiao_status, guatiao_registry_provider,                                                 \
    (const guatiao_registry *reg, guatiao_str key, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, key, alloc, out), "load")                                                             \
  X(guatiao_status, guatiao_registry_available,                                                \
    (const guatiao_registry *reg, guatiao_str kind, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, kind, alloc, out), "load")                                                            \
  X(guatiao_status, guatiao_registry_why_not,                                                  \
    (const guatiao_registry *reg, guatiao_str kind, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, kind, alloc, out), "load")                                                            \
  X(guatiao_status, guatiao_registry_best,                                                     \
    (const guatiao_registry *reg, guatiao_str kind, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, kind, alloc, out), "load")                                                            \
  X(bool, guatiao_registry_provider_available,                                                 \
    (const guatiao_registry *reg, guatiao_str key, guatiao_str *reason), (reg, key, reason), "load") \
  X(guatiao_status, guatiao_registry_set_priority, (guatiao_registry *reg, guatiao_str id, int32_t p), \
    (reg, id, p), "load")                                                                       \
  X(int32_t, guatiao_registry_priority, (const guatiao_registry *reg, guatiao_str id), (reg, id), "load") \
  X(const void *, guatiao_registry_provider_vtable,                                            \
    (const guatiao_registry *reg, guatiao_str key, size_t *size), (reg, key, size), "load")     \
  X(const void *, guatiao_registry_provider_table,                                             \
    (const guatiao_registry *reg, guatiao_str key, guatiao_str kind, size_t *size),             \
    (reg, key, kind, size), "load")                                                             \
  X(void *, guatiao_registry_provider_ctx, (const guatiao_registry *reg, guatiao_str key), (reg, key), "load") \
  X(const guatiao_value *, guatiao_registry_provider_config,                                   \
    (const guatiao_registry *reg, guatiao_str key), (reg, key), "load")                        \
  X(guatiao_status, guatiao_registry_provider_meta,                                            \
    (const guatiao_registry *reg, guatiao_str key, const guatiao_alloc *alloc, guatiao_value *out), \
    (reg, key, alloc, out), "load")                                                             \
  X(guatiao_status, guatiao_registry_provider_create,                                          \
    (const guatiao_registry *reg, guatiao_str key, const guatiao_value *config, void **out,     \
     guatiao_provider_error *err),                                                              \
    (reg, key, config, out, err), "load")                                                       \
  GT_VOID(guatiao_registry_provider_destroy,                                                    \
          (const guatiao_registry *reg, guatiao_str key, void *instance), (reg, key, instance), "load") \
  X(guatiao_status, guatiao_registry_retire, (guatiao_registry *reg, guatiao_str key), (reg, key), "load") \
  X(guatiao_status, guatiao_registry_unload, (guatiao_registry *reg, guatiao_str key), (reg, key), "load") \
  X(guatiao_status, guatiao_registry_unload_unchecked, (guatiao_registry *reg, guatiao_str key), \
    (reg, key), "load")                                                                         \
  X(guatiao_status, guatiao_registry_collect,                                                  \
    (guatiao_registry *reg, const guatiao_alloc *alloc, guatiao_value *out), (reg, alloc, out), "load")

/* One table of pointers per library, a field per export. */
#define GT_FIELD(ret, name, params, args, feature) ret(*name) params;
#define GT_VOID_FIELD(name, params, args, feature) void(*name) params;

#define X GT_FIELD
#define GT_VOID GT_VOID_FIELD
typedef struct gt_core_fns {
  GT_CORE(X)
} gt_core_fns;
#undef X
#undef GT_VOID

extern gt_core_fns gt_core;

enum { GT_LIB_CORE = 0, GT_LIB_SERDE = 1, GT_LIB_INTAKE = 2 };

/*
 * Opens the library at `path` (UTF-8) as `lib`: the core's exports are
 * resolved; another surface's file must export its probe symbol. Answers
 * 0, or 1 when the file did not open (its reason in `err`), or 2 when an
 * export every build has, or the probe, is missing (its name in `err`). A
 * library opened stays open for the process: trees it built record its
 * allocator, whose code lives in its image.
 */
int gt_open(int lib, const char *path, char *err, size_t errlen);

/* The symbol only surface `lib` exports, checked when it opens. */
const char *gt_probe_of(int lib);

/* 1 when the file at `path` opens and exports `symbol`; it is closed again. */
int gt_file_carries(const char *path, const char *symbol);

/* Whether `lib` is open. */
int gt_is_open(int lib);

/* The feature an absent optional export of `lib` needs, or NULL when the
 * export is present or unknown. */
const char *gt_missing_feature(int lib, const char *name);

/* A function pointer from an image already open (`lib`), or NULL. */
void *gt_symbol_of(int lib, const char *name);

/* ---- allocators ---------------------------------------------------------- */

/* The binding's allocator, over the C library's malloc: every tree this
 * package builds without being told otherwise. It lives in this image,
 * which stays loaded for the process. */
extern const guatiao_alloc gt_binding_alloc;

/* An allocator that counts the blocks it handed out and has not had back. */
typedef struct gt_counting {
  guatiao_alloc alloc;
  int64_t live;
  int64_t total;
} gt_counting;
gt_counting *gt_counting_new(void);
int64_t gt_counting_live(const gt_counting *c);

/* malloc through the binding's allocator, for text written into an error
 * from any thread. */
void *gt_malloc(size_t n);

/* ---- the thread R runs on ------------------------------------------------- */

void gt_note_main_thread(void);
int gt_on_main_thread(void);

/* A lock for the few structures any thread may touch. */
void gt_lock(void);
void gt_unlock(void);

#endif
