/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Every command the gateway answers, by the name the M side calls it with.
 * Each is `int cmd_<name>(gt_call *)`, answering 1, or 0 with a refusal
 * recorded in the call. */

#define GT_COMMANDS(X)                                                         \
  X(use_library)                                                               \
  X(has_export)                                                                \
  X(library_path)                                                              \
  X(value_new)                                                                 \
  X(value_close)                                                               \
  X(value_is_open)                                                             \
  X(node_read)                                                                 \
  X(node_tag)                                                                  \
  X(node_length)                                                               \
  X(node_keys)                                                                 \
  X(node_child)                                                                \
  X(node_set)                                                                  \
  X(node_push)                                                                 \
  X(node_insert)                                                               \
  X(node_remove)                                                               \
  X(node_clear)                                                                \
  X(node_merge)                                                                \
  X(node_copy)                                                                 \
  X(number_check)                                                              \
  X(number_read)                                                               \
  X(alloc_counting)                                                            \
  X(alloc_live)                                                                \
  X(node_alloc_is)                                                             \
  X(handles)                                                                   \
  X(node_pointer)                                                              \
  X(alloc_address)                                                             \
  X(serving)                                                                   \
  X(time_parse)                                                                \
  X(time_format)                                                               \
  X(time_normalize)                                                            \
  X(date_add_span)                                                             \
  X(datetime_add_span)                                                         \
  X(duration_read)                                                             \
  X(serde_parse)                                                               \
  X(serde_emit)                                                                \
  X(intake_check)                                                              \
  X(intake_layout)                                                             \
  X(intake_is_visible)                                                         \
  X(intake_for_schema)                                                         \
  X(intake_form_for)                                                           \
  X(intake_at)                                                                 \
  X(intake_resolve)                                                            \
  X(intake_flat_keys)                                                          \
  X(intake_flatten)                                                            \
  X(intake_unflatten)                                                          \
  X(redact)                                                                    \
  X(schema_validate)                                                           \
  X(kind_new)                                                                  \
  X(table_floor_hash)                                                          \
  X(remote_new)                                                                \
  X(remote_has)                                                                \
  X(remote_close)                                                              \
  X(remote_live)                                                               \
  X(remote_call)                                                               \
  X(pending_destroys)                                                          \
  X(served_table)                                                              \
  X(answer)                                                                    \
  X(library_new)                                                               \
  X(library_entry)                                                             \
  X(library_close)                                                             \
  X(library_live)                                                              \
  X(library_info)                                                              \
  X(registry_new)                                                              \
  X(registry_free)                                                             \
  X(registry_is_open)                                                          \
  X(registry_load_file)                                                        \
  X(registry_register_entry)                                                   \
  X(registry_scan)                                                             \
  X(registry_query)                                                            \
  X(registry_provider_available)                                               \
  X(registry_keyed_by)                                                         \
  X(registry_set_priority)                                                     \
  X(registry_priority)                                                         \
  X(registry_provider_table)                                                   \
  X(registry_provider_config)                                                  \
  X(registry_host)                                                             \
  X(registry_create)                                                           \
  X(instance_ctx)                                                              \
  X(instance_destroy)                                                          \
  X(registry_retire)                                                           \
  X(registry_collect)                                                          \
  X(registry_unload)                                                           \
  X(registry_add_source_file)                                                  \
  X(registry_watch_dir)                                                        \
  X(registry_refresh)
