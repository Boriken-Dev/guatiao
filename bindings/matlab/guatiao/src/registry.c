/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The registry and the instances its providers build. Every answer that
 * is data is read into MATLAB data and freed here. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kinds.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* An instance a provider built: the key it was created under and its ctx;
 * destroyed through the registry, which keeps the building provider. */
typedef struct gt_instance {
  uint64_t registry;
  char *key;
  size_t key_len;
  void *ctx;
} gt_instance;

static int reg_arg(gt_call *c, int i, guatiao_registry **out) {
  return gt_arg_handle(c, i, GT_H_REGISTRY, "registry", (void **)out);
}

/* registry_new(id, version, alloc) -> id */
int cmd_registry_new(gt_call *c) {
  guatiao_str id, version;
  const guatiao_alloc *a = NULL;
  if (!gt_arg_text(c, 0, "the id", &id) || !gt_arg_text(c, 1, "the version", &version) || !gt_arg_alloc(c, 2, &a) ||
      !GT_NEED_CORE(c, guatiao_registry_new))
    return 0;
  guatiao_registry *reg = gt_core.guatiao_registry_new(id, version, a);
  if (!reg) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the registry was refused");
  uint64_t h = gt_handle_add(GT_H_REGISTRY, reg);
  if (!h) {
    gt_core.guatiao_registry_free(reg);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)h));
  return 1;
}

/* registry_free(id): the libraries it loaded stay mapped. */
int cmd_registry_free(gt_call *c) {
  uint64_t id = 0;
  if (!gt_arg_id(c, 0, &id)) {
    c->status = GUATIAO_OK;
    return 1;
  }
  guatiao_registry *reg = (guatiao_registry *)gt_handle_take(id, GT_H_REGISTRY);
  if (reg) gt_core.guatiao_registry_free(reg);
  return 1;
}

int cmd_registry_is_open(gt_call *c) {
  uint64_t id = 0;
  int open = gt_arg_id(c, 0, &id) && gt_handle_get(id, GT_H_REGISTRY) != NULL;
  c->status = GUATIAO_OK;
  gt_answer(c, 0, mxCreateLogicalScalar(open ? true : false));
  return 1;
}

/* An answer built in the binding's allocator: plain data, freed. */
static int answer(gt_call *c, guatiao_status st, guatiao_value *out, const char *what) {
  if (st) {
    gt_free(out);
    return gt_fail(c, st, "%s", what);
  }
  mxArray *x = NULL;
  int ok = gt_read(c, out, NULL, &x);
  gt_free(out);
  if (ok) gt_answer(c, 0, x);
  return ok;
}

int cmd_registry_load_file(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str p;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the path", &p)) return 0;
  guatiao_value out;
  set_absent(&out);
  return answer(c, gt_core.guatiao_registry_load_file(reg, p, &gt_binding_alloc, &out), &out, "the file was refused");
}

typedef const guatiao_library_info *(*entry_fn)(const guatiao_host_info *);

/* registry_register_entry(reg, name, entry address) */
int cmd_registry_register_entry(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str n;
  void *p = NULL;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the name", &n)) return 0;
  if (!gt_mx_addr(gt_arg(c, 2), &p) || !p) return gt_fail(c, GUATIAO_ERR_NULL, "no entry function");
  entry_fn fn;
  memcpy(&fn, &p, sizeof fn);
  guatiao_value out;
  set_absent(&out);
  return answer(c, gt_core.guatiao_registry_register_entry(reg, n, fn, &gt_binding_alloc, &out), &out,
                "the library was refused");
}

/* The scan's refusal in words: the core says NOT_FOUND for a path that is
   missing or names a file, BAD_VALUE for a rule that is not one. */
static const char *scan_refusal(guatiao_status st) {
  if (st == GUATIAO_ERR_NOT_FOUND) return "the path names no directory (missing, or a file)";
  if (st == GUATIAO_ERR_BAD_VALUE) return "the scan was refused (a rule that is not one?)";
  return "the scan was refused";
}

/* registry_scan(reg, where, rules, descending, is_path) */
int cmd_registry_scan(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str w, r;
  int desc = 0, path = 0;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the directory", &w) || !gt_arg_text(c, 2, "the rules", &r) ||
      !gt_arg_bool(c, 3, "descending", &desc) || !gt_arg_bool(c, 4, "is_path", &path))
    return 0;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = path ? gt_core.guatiao_registry_scan_path(reg, w, desc != 0, r, &gt_binding_alloc, &out)
                           : gt_core.guatiao_registry_scan_dir_rules(reg, w, desc != 0, r, &gt_binding_alloc, &out);
  return answer(c, st, &out, scan_refusal(st));
}

/* registry_query(reg, what, arg): 0 libraries, 1 providers, 2 available,
 * 3 why_not, 4 provider, 5 best */
int cmd_registry_query(gt_call *c) {
  guatiao_registry *reg = NULL;
  double what = 0;
  if (!reg_arg(c, 0, &reg) || !gt_arg_double(c, 1, "what", &what)) return 0;
  int w = (int)what;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  if (w == 0) {
    st = gt_core.guatiao_registry_libraries(reg, &gt_binding_alloc, &out);
  } else {
    guatiao_str a;
    if (!gt_arg_text(c, 2, "the kind or key", &a)) return 0;
    switch (w) {
    case 1: st = gt_core.guatiao_registry_providers(reg, a, &gt_binding_alloc, &out); break;
    case 2: st = gt_core.guatiao_registry_available(reg, a, &gt_binding_alloc, &out); break;
    case 3: st = gt_core.guatiao_registry_why_not(reg, a, &gt_binding_alloc, &out); break;
    case 4: st = gt_core.guatiao_registry_provider(reg, a, &gt_binding_alloc, &out); break;
    default: st = gt_core.guatiao_registry_best(reg, a, &gt_binding_alloc, &out); break;
    }
  }
  return answer(c, st, &out, w == 4 ? "no provider answers to that key" : w == 5 ? "nothing can serve that kind" : "refused");
}

/* registry_provider_available(reg, key) -> struct(available, reason) */
int cmd_registry_provider_available(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  guatiao_str why = {NULL, 0};
  bool ok = gt_core.guatiao_registry_provider_available(reg, k, &why);
  const char *names[2] = {"available", "reason"};
  mxArray *out = mxCreateStructMatrix(1, 1, 2, names);
  mxSetField(out, 0, "available", mxCreateLogicalScalar(ok));
  mxArray *r = NULL;
  if (!ok && why.len) {
    if (!gt_text_out(c, why, &r)) {
      mxDestroyArray(out);
      return 0;
    }
  } else {
    r = mxCreateString("");
  }
  mxSetField(out, 0, "reason", r);
  gt_answer(c, 0, out);
  return 1;
}

int cmd_registry_keyed_by(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str t;
  int libs = 0;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the template", &t) || !gt_arg_bool(c, 2, "libraries", &libs)) return 0;
  guatiao_status st = libs ? gt_core.guatiao_registry_libraries_keyed_by(reg, t) : gt_core.guatiao_registry_keyed_by(reg, t);
  if (st) return gt_fail(c, st, "the template was refused");
  return 1;
}

int cmd_registry_set_priority(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str id;
  double p = 0;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the id", &id) || !gt_arg_double(c, 2, "the priority", &p)) return 0;
  if (p < -2147483648.0 || p > 2147483647.0) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a priority is an int32");
  guatiao_status st = gt_core.guatiao_registry_set_priority(reg, id, (int32_t)p);
  if (st) return gt_fail(c, st, "the priority was refused");
  return 1;
}

int cmd_registry_priority(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str id;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the id", &id)) return 0;
  gt_answer(c, 0, mxCreateDoubleScalar((double)gt_core.guatiao_registry_priority(reg, id)));
  return 1;
}

/* 'library' or 'path'; any other text reaches the library as a value it
 * refuses (BAD_VALUE). */
int cmd_registry_set_import_search(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str s;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the import search", &s)) return 0;
  uint32_t code = 2;
  if (s.len == 7 && memcmp(s.ptr, "library", 7) == 0) code = GUATIAO_IMPORT_SEARCH_LIBRARY;
  if (s.len == 4 && memcmp(s.ptr, "path", 4) == 0) code = GUATIAO_IMPORT_SEARCH_PATH;
  guatiao_status st = gt_core.guatiao_registry_set_import_search(reg, code);
  if (st) return gt_fail(c, st, "the import search is 'library' or 'path'");
  return 1;
}

int cmd_registry_import_search(gt_call *c) {
  guatiao_registry *reg = NULL;
  if (!reg_arg(c, 0, &reg)) return 0;
  gt_answer(c, 0,
            mxCreateString(gt_core.guatiao_registry_import_search(reg) == GUATIAO_IMPORT_SEARCH_PATH
                               ? "path"
                               : "library"));
  return 1;
}

/* registry_provider_table(reg, key, kind or []) -> struct(table, size, ctx);
 * table [] for none */
int cmd_registry_provider_table(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k, kind;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  const mxArray *kind_arg = gt_arg(c, 2);
  int by_kind = kind_arg && !mxIsEmpty(kind_arg);
  if (by_kind && !gt_arg_text(c, 2, "the kind", &kind)) return 0;
  size_t size = 0;
  const void *table = by_kind ? gt_core.guatiao_registry_provider_table(reg, k, kind, &size)
                              : gt_core.guatiao_registry_provider_vtable(reg, k, &size);
  void *ctx = gt_core.guatiao_registry_provider_ctx(reg, k);
  const char *names[3] = {"table", "size", "ctx"};
  mxArray *out = mxCreateStructMatrix(1, 1, 3, names);
  mxSetField(out, 0, "table", table ? gt_addr_mx(table) : mxCreateDoubleMatrix(0, 0, mxREAL));
  mxSetField(out, 0, "size", mxCreateDoubleScalar(table ? (double)size : 0));
  mxSetField(out, 0, "ctx", gt_addr_mx(ctx));
  gt_answer(c, 0, out);
  return 1;
}

/* A copy of a provider's configuration schema, so nothing M holds reads
 * the registry's after a retire. */
int cmd_registry_provider_config(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  const guatiao_value *s = gt_core.guatiao_registry_provider_config(reg, k);
  if (!s) return 1;
  guatiao_value copy;
  set_absent(&copy);
  guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, s, &copy);
  if (st) return gt_fail(c, st, "the copy was refused");
  uint64_t id = gt_owner_new(&copy, &gt_binding_alloc);
  if (!id) {
    gt_free(&copy);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* What a provider declared beyond its schema, a copy in this binding's
 * allocator; nothing answered when it declares nothing. */
int cmd_registry_provider_meta(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  guatiao_value meta;
  set_absent(&meta);
  guatiao_status st = gt_core.guatiao_registry_provider_meta(reg, k, &gt_binding_alloc, &meta);
  if (st) return gt_fail(c, st, st == GUATIAO_ERR_NOT_FOUND ? "no provider answers to that key" : "the meta was refused");
  if (meta.tag == GUATIAO_ABSENT) return 1;
  uint64_t id = gt_owner_new(&meta, &gt_binding_alloc);
  if (!id) {
    gt_free(&meta);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

int cmd_registry_host(gt_call *c) {
  guatiao_registry *reg = NULL;
  if (!reg_arg(c, 0, &reg)) return 0;
  gt_answer(c, 0, gt_addr_mx(gt_core.guatiao_registry_host(reg)));
  return 1;
}

/* registry_create(reg, key, config) -> instance id */
int cmd_registry_create(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  uint64_t reg_id = 0;
  if (!gt_arg_id(c, 0, &reg_id) || !reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  guatiao_value tmp;
  const guatiao_value *config = NULL;
  int at[1] = {2};
  if (!gt_tree_args(c, 1, at, &tmp, &config)) return 0;
  if (!gt_handle_get(reg_id, GT_H_REGISTRY)) { /* building may have run M code */
    gt_free(&tmp);
    return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the registry is closed");
  }
  gt_instance *inst = (gt_instance *)calloc(1, sizeof(gt_instance));
  char *key = (char *)malloc(k.len + 1);
  if (!inst || !key) {
    free(inst);
    free(key);
    gt_free(&tmp);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  memcpy(key, k.ptr, k.len);
  key[k.len] = '\0';
  void *instance = NULL;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = gt_core.guatiao_registry_provider_create(reg, k, config, &instance, &err);
  gt_free(&tmp);
  if (st) {
    free(inst);
    free(key);
    return gt_fail_err(c, st, &err,
                       st == GUATIAO_ERR_NOT_FOUND ? "no provider answers to that key"
                       : st == GUATIAO_ERR_NULL    ? "the provider builds no instances"
                                                   : NULL);
  }
  gt_err_free(&err);
  inst->registry = reg_id;
  inst->key = key;
  inst->key_len = k.len;
  inst->ctx = instance;
  uint64_t id = gt_handle_add(GT_H_INSTANCE, inst);
  if (!id) {
    gt_core.guatiao_registry_provider_destroy(reg, guatiao_str_from(key, k.len), instance);
    free(key);
    free(inst);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* instance_ctx(inst) -> the ctx address, for binding its tables */
int cmd_instance_ctx(gt_call *c) {
  gt_instance *inst = NULL;
  if (!gt_arg_handle(c, 0, GT_H_INSTANCE, "instance", (void **)&inst)) return 0;
  gt_answer(c, 0, gt_addr_mx(inst->ctx));
  return 1;
}

/* instance_destroy(inst): through the provider that built it, once; a
 * registry already closed destroys nothing. */
int cmd_instance_destroy(gt_call *c) {
  uint64_t id = 0;
  if (!gt_arg_id(c, 0, &id)) {
    c->status = GUATIAO_OK;
    return 1;
  }
  gt_instance *inst = (gt_instance *)gt_handle_take(id, GT_H_INSTANCE);
  if (!inst) return 1;
  guatiao_registry *reg = (guatiao_registry *)gt_handle_get(inst->registry, GT_H_REGISTRY);
  if (reg && inst->ctx) gt_core.guatiao_registry_provider_destroy(reg, guatiao_str_from(inst->key, inst->key_len), inst->ctx);
  free(inst->key);
  free(inst);
  return 1;
}

int cmd_registry_retire(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k)) return 0;
  guatiao_status st = gt_core.guatiao_registry_retire(reg, k);
  if (st) return gt_fail(c, st, "%s", st == GUATIAO_ERR_NOT_FOUND ? "no library answers to that key" : "the retire was refused");
  return 1;
}

int cmd_registry_collect(gt_call *c) {
  guatiao_registry *reg = NULL;
  if (!reg_arg(c, 0, &reg)) return 0;
  guatiao_value out;
  set_absent(&out);
  return answer(c, gt_core.guatiao_registry_collect(reg, &gt_binding_alloc, &out), &out, "the collect was refused");
}

int cmd_registry_unload(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str k;
  int unchecked = 0;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the key", &k) || !gt_arg_bool(c, 2, "unchecked", &unchecked)) return 0;
  guatiao_status st = unchecked ? gt_core.guatiao_registry_unload_unchecked(reg, k) : gt_core.guatiao_registry_unload(reg, k);
  if (st) {
    const char *why = st == GUATIAO_ERR_BUSY          ? "something it handed out is still alive"
                      : st == GUATIAO_ERR_UNSUPPORTED ? "the library has no unload function"
                      : st == GUATIAO_ERR_WRONG_KIND  ? "the library is linked, not mapped"
                      : st == GUATIAO_ERR_NOT_FOUND   ? "no library answers to that key"
                                                      : "the unload was refused";
    return gt_fail(c, st, "%s", why);
  }
  return 1;
}

int cmd_registry_add_source_file(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str p;
  uint64_t reg_id = 0;
  if (!gt_arg_id(c, 0, &reg_id) || !reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the path", &p)) return 0;
  guatiao_value tmp;
  const guatiao_value *config = NULL;
  int at[1] = {2};
  if (!gt_tree_args(c, 1, at, &tmp, &config)) return 0;
  if (!gt_handle_get(reg_id, GT_H_REGISTRY)) {
    gt_free(&tmp);
    return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the registry is closed");
  }
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = gt_core.guatiao_registry_add_source_file(reg, p, config, &err);
  gt_free(&tmp);
  if (st) return gt_fail_err(c, st, &err, "the source was refused");
  gt_err_free(&err);
  return 1;
}

int cmd_registry_watch_dir(gt_call *c) {
  guatiao_registry *reg = NULL;
  guatiao_str d, r;
  if (!reg_arg(c, 0, &reg) || !gt_arg_text(c, 1, "the directory", &d) || !gt_arg_text(c, 2, "the rules", &r)) return 0;
  guatiao_status st = gt_core.guatiao_registry_watch_dir(reg, d, r);
  if (st) return gt_fail(c, st, "the directory was refused (watched already, or a rule that is not one)");
  return 1;
}

int cmd_registry_refresh(gt_call *c) {
  guatiao_registry *reg = NULL;
  if (!reg_arg(c, 0, &reg)) return 0;
  guatiao_value out;
  set_absent(&out);
  return answer(c, gt_core.guatiao_registry_refresh(reg, &gt_binding_alloc, &out), &out, "the refresh was refused");
}
