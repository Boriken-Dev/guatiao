/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Commands on values and the nodes in them, and the allocators tests use. */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gt.h"

int gt_use_library(gt_call *c, guatiao_str path);
const char *gt_opened_path(int lib);

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

static const char *tag_name(uint32_t tag) {
  switch (tag) {
  case GUATIAO_ABSENT: return "absent";
  case GUATIAO_NULL: return "null";
  case GUATIAO_BOOL: return "bool";
  case GUATIAO_NUMBER: return "number";
  case GUATIAO_STRING: return "string";
  case GUATIAO_BYTES: return "bytes";
  case GUATIAO_LIST: return "list";
  case GUATIAO_MAP: return "map";
  default: return "unknown";
  }
}

/* A selector: a key (text) or a position from 1. */
typedef struct sel {
  int is_key;
  guatiao_str key;
  long long at; /* from 0; -1 when not a whole number from 1 */
} sel;

static int arg_sel(gt_call *c, int i, sel *s) {
  const mxArray *a = gt_arg(c, i);
  if (!a) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a key or a position is missing");
  if (mxIsChar(a)) {
    s->is_key = 1;
    return gt_mx_utf8(c, a, "a key", &s->key);
  }
  double d = 0;
  if (!gt_arg_double(c, i, "a key or a position", &d)) return 0;
  s->is_key = 0;
  s->at = (isfinite(d) && d >= 1 && d == floor(d)) ? (long long)d - 1 : -1;
  return 1;
}

static int read_opts(gt_call *c, int i, gt_read_opts *o) {
  double n = 0, m = 0;
  o->numbers = 0;
  o->maps = 0;
  if (gt_arg(c, i) && !gt_arg_double(c, i, "numbers", &n)) return 0;
  if (gt_arg(c, i + 1) && !gt_arg_double(c, i + 1, "maps", &m)) return 0;
  o->numbers = (int)n;
  o->maps = (int)m;
  return 1;
}

static int answer_id(gt_call *c, uint64_t id, guatiao_value *v) {
  if (!id) {
    gt_free(v);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* use_library(path) */
int cmd_use_library(gt_call *c) {
  guatiao_str p;
  if (!gt_arg_text(c, 0, "the path", &p)) return 0;
  return gt_use_library(c, p);
}

/* has_export(lib, name) -> logical */
int cmd_has_export(gt_call *c) {
  double lib = 0;
  guatiao_str name;
  if (!gt_arg_double(c, 0, "the library", &lib) || !gt_arg_text(c, 1, "the export", &name)) return 0;
  if (lib < 0 || lib > 2) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "no such library");
  if (!gt_need(c, (int)lib)) return 0;
  gt_answer(c, 0, mxCreateLogicalScalar(gt_symbol_of((int)lib, (const char *)name.ptr) != NULL));
  return 1;
}

/* library_path(lib) -> the file opened, '' when none */
int cmd_library_path(gt_call *c) {
  double lib = 0;
  if (!gt_arg_double(c, 0, "the library", &lib)) return 0;
  gt_answer(c, 0, gt_cstr_mx(gt_opened_path((int)lib)));
  return 1;
}

/* value_new(data, alloc) -> id; value_new() holds absent */
int cmd_value_new(gt_call *c) {
  const guatiao_alloc *a = NULL;
  if (!gt_arg_alloc(c, 1, &a)) return 0;
  if (!gt_need(c, GT_LIB_CORE)) return 0;
  guatiao_value v;
  set_absent(&v);
  if (gt_arg(c, 0) && !gt_build(c, gt_arg(c, 0), a, &v)) return 0;
  return answer_id(c, gt_owner_new(&v, a), &v);
}

/* value_close(id): a borrowed owner is not closed by its reader */
int cmd_value_close(gt_call *c) {
  uint64_t id = 0;
  if (!gt_arg_id(c, 0, &id)) return 1;
  gt_owner *o = (gt_owner *)gt_handle_get(id, GT_H_OWNER);
  if (o && (o->flags & GT_OWNED)) gt_owner_drop((gt_owner *)gt_handle_take(id, GT_H_OWNER));
  return 1;
}

int cmd_value_is_open(gt_call *c) {
  uint64_t id = 0;
  double d = 0;
  int open = gt_arg_double(c, 0, "a handle", &d) && d >= 1 && d == floor(d) &&
             (id = (uint64_t)d, gt_handle_get(id, GT_H_OWNER) != NULL);
  c->status = GUATIAO_OK;
  gt_answer(c, 0, mxCreateLogicalScalar(open ? true : false));
  return 1;
}

/* node_read(id, path, numbers, maps) -> data */
int cmd_node_read(gt_call *c) {
  guatiao_value *v = NULL;
  gt_read_opts o;
  if (!gt_arg_node(c, 0, NULL, &v) || !read_opts(c, 2, &o)) return 0;
  mxArray *out = NULL;
  if (!gt_read(c, v, &o, &out)) return 0;
  gt_answer(c, 0, out);
  return 1;
}

int cmd_node_tag(gt_call *c) {
  guatiao_value *v = NULL;
  if (!gt_arg_node(c, 0, NULL, &v)) return 0;
  gt_answer(c, 0, gt_cstr_mx(tag_name(guatiao_tag_of(v))));
  return 1;
}

int cmd_node_length(gt_call *c) {
  guatiao_value *v = NULL;
  if (!gt_arg_node(c, 0, NULL, &v)) return 0;
  double n = 1;
  switch (guatiao_tag_of(v)) {
  case GUATIAO_MAP: n = (double)guatiao_map_entries(v).len; break;
  case GUATIAO_LIST: n = (double)guatiao_list_items(v).len; break;
  case GUATIAO_ABSENT:
  case GUATIAO_NULL: n = 0; break;
  default: break;
  }
  gt_answer(c, 0, mxCreateDoubleScalar(n));
  return 1;
}

/* node_keys(id, path) -> a cell row of a map's keys, in order; {} otherwise */
int cmd_node_keys(gt_call *c) {
  guatiao_value *v = NULL;
  if (!gt_arg_node(c, 0, NULL, &v)) return 0;
  if (guatiao_tag_of(v) != GUATIAO_MAP) {
    gt_answer(c, 0, mxCreateCellMatrix(1, 0));
    return 1;
  }
  guatiao_entries es = guatiao_map_entries(v);
  mxArray *keys = mxCreateCellMatrix(1, (mwSize)es.len);
  for (size_t i = 0; i < es.len; i++) {
    mxArray *k = NULL;
    if (!gt_text_out(c, guatiao_entry_key(&es.ptr[i]), &k)) {
      mxDestroyArray(keys);
      return 0;
    }
    mxSetCell(keys, (mwIndex)i, k);
  }
  gt_answer(c, 0, keys);
  return 1;
}

/* node_child(id, path, sel) -> the path step below the node: a key, a
 * position from 0, or [] for a key the map does not hold */
int cmd_node_child(gt_call *c) {
  guatiao_value *v = NULL;
  sel s;
  if (!gt_arg_node(c, 0, NULL, &v) || !arg_sel(c, 2, &s)) return 0;
  uint32_t tag = guatiao_tag_of(v);
  if (s.is_key) {
    if (tag != GUATIAO_MAP) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s has no keys", tag_name(tag));
    if (!guatiao_map_find(v, s.key)) {
      gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
      return 1;
    }
    gt_answer(c, 0, mxDuplicateArray(gt_arg(c, 2)));
    return 1;
  }
  if (tag == GUATIAO_LIST) {
    if (s.at < 0 || (size_t)s.at >= guatiao_list_items(v).len)
      return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
    gt_answer(c, 0, mxCreateDoubleScalar((double)s.at));
    return 1;
  }
  if (tag == GUATIAO_MAP) {
    guatiao_entries es = guatiao_map_entries(v);
    if (s.at < 0 || (size_t)s.at >= es.len) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
    mxArray *k = NULL;
    if (!gt_text_out(c, guatiao_entry_key(&es.ptr[s.at]), &k)) return 0;
    gt_answer(c, 0, k);
    return 1;
  }
  return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s has no elements", tag_name(tag));
}

static int writable(gt_call *c, gt_owner **o, guatiao_value **v) {
  if (!gt_arg_node(c, 0, o, v)) return 0;
  if ((*o)->flags & GT_READONLY) return gt_fail(c, GUATIAO_ERR_UNSUPPORTED, "this view is read-only");
  return 1;
}

static void replace_element(guatiao_value *list, size_t at, guatiao_value *tmp) {
  guatiao_value *slot = &list->payload.list.ptr[at];
  gt_free(slot);
  *slot = *tmp;
  set_absent(tmp);
}

/* node_set(id, path, sel, x): a key on a map; a position replaces a list's
 * element, or appends at length + 1 */
int cmd_node_set(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  sel s;
  if (!writable(c, &o, &v) || !arg_sel(c, 2, &s)) return 0;
  if (!gt_arg(c, 3)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a value to set is missing");
  uint32_t tag = guatiao_tag_of(v);
  const guatiao_alloc *a = gt_alloc_for(v, o);
  guatiao_value tmp;
  if (s.is_key || tag == GUATIAO_MAP) {
    if (tag != GUATIAO_MAP) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s has no keys", tag_name(tag));
    guatiao_str k = s.key;
    if (!s.is_key) {
      guatiao_entries es = guatiao_map_entries(v);
      if (s.at < 0 || (size_t)s.at >= es.len) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
      guatiao_str ek = guatiao_entry_key(&es.ptr[s.at]);
      /* Copied before the tree changes: it views the map's own key. */
      char *copy = (char *)gt_scratch(c, ek.len + 1);
      if (!copy) return 0;
      if (ek.len) memcpy(copy, ek.ptr, ek.len);
      k = guatiao_str_from(copy, ek.len);
    }
    if (!gt_build(c, gt_arg(c, 3), a, &tmp)) return 0;
    if (!gt_node_again(c, 0, &v)) { /* building may have run M code */
      gt_free(&tmp);
      return 0;
    }
    guatiao_status st = gt_core.guatiao_map_set(a, v, k, &tmp);
    if (st) {
      gt_free(&tmp);
      return gt_fail(c, st, "the map refused the key");
    }
    return 1;
  }
  if (tag != GUATIAO_LIST) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s has no elements", tag_name(tag));
  if (s.at < 0 || (size_t)s.at > guatiao_list_items(v).len) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
  if (!gt_build(c, gt_arg(c, 3), a, &tmp)) return 0;
  if (!gt_node_again(c, 0, &v)) {
    gt_free(&tmp);
    return 0;
  }
  if ((size_t)s.at >= guatiao_list_items(v).len) {
    guatiao_status st = gt_core.guatiao_list_push(a, v, &tmp);
    if (st) {
      gt_free(&tmp);
      return gt_fail(c, st, "the list refused the element");
    }
  } else {
    replace_element(v, (size_t)s.at, &tmp);
  }
  return 1;
}

/* node_push(id, path, items): each element of the cell `items`, in order */
int cmd_node_push(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  if (!writable(c, &o, &v)) return 0;
  const mxArray *items = gt_arg(c, 2);
  if (!items || !mxIsCell(items)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "push takes a cell of items");
  if (guatiao_tag_of(v) != GUATIAO_LIST) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "only a list takes elements");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  for (size_t i = 0; i < mxGetNumberOfElements(items); i++) {
    guatiao_value tmp;
    const mxArray *x = mxGetCell(items, (mwIndex)i);
    if (x) {
      if (!gt_build(c, x, a, &tmp)) return 0;
    } else {
      set_absent(&tmp);
      gt_core.guatiao_value_null(&tmp);
    }
    if (!gt_node_again(c, 0, &v)) {
      gt_free(&tmp);
      return 0;
    }
    guatiao_status st = gt_core.guatiao_list_push(a, v, &tmp);
    if (st) {
      gt_free(&tmp);
      return gt_fail(c, st, "the list refused the element");
    }
  }
  return 1;
}

/* node_insert(id, path, at, x): before position `at` (from 1) */
int cmd_node_insert(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  sel s;
  if (!writable(c, &o, &v) || !arg_sel(c, 2, &s)) return 0;
  if (guatiao_tag_of(v) != GUATIAO_LIST) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "only a list takes elements");
  if (s.is_key || s.at < 0 || (size_t)s.at > guatiao_list_items(v).len)
    return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
  if (!gt_arg(c, 3)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a value to insert is missing");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  guatiao_value tmp;
  if (!gt_build(c, gt_arg(c, 3), a, &tmp)) return 0;
  if (!gt_node_again(c, 0, &v)) {
    gt_free(&tmp);
    return 0;
  }
  if ((size_t)s.at > guatiao_list_items(v).len) {
    gt_free(&tmp);
    return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "index out of bounds");
  }
  guatiao_status st = gt_core.guatiao_list_push(a, v, &tmp);
  if (st) {
    gt_free(&tmp);
    return gt_fail(c, st, "the list refused the element");
  }
  /* Elements are plain structs: the new last one moves to `at`. */
  guatiao_value *arr = v->payload.list.ptr;
  size_t len = v->payload.list.len;
  guatiao_value last = arr[len - 1];
  memmove(&arr[s.at + 1], &arr[s.at], (len - 1 - (size_t)s.at) * sizeof(guatiao_value));
  arr[s.at] = last;
  return 1;
}

/* node_remove(id, path, sel): a key or a position; nothing there is no error */
int cmd_node_remove(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  sel s;
  if (!writable(c, &o, &v) || !arg_sel(c, 2, &s)) return 0;
  uint32_t tag = guatiao_tag_of(v);
  guatiao_status st;
  if (tag == GUATIAO_MAP && s.is_key) {
    st = GT_NEED_CORE(c, guatiao_map_discard) ? gt_core.guatiao_map_discard(v, s.key) : GUATIAO_OK;
  } else if (tag == GUATIAO_MAP) {
    guatiao_entries es = guatiao_map_entries(v);
    if (s.at < 0 || (size_t)s.at >= es.len) return 1;
    guatiao_str k = guatiao_entry_key(&es.ptr[s.at]);
    char *copy = (char *)gt_scratch(c, k.len + 1);
    if (!copy) return 0;
    if (k.len) memcpy(copy, k.ptr, k.len);
    st = gt_core.guatiao_map_discard(v, guatiao_str_from(copy, k.len));
  } else if (tag == GUATIAO_LIST && !s.is_key) {
    if (s.at < 0) return 1;
    st = gt_core.guatiao_list_discard(v, (size_t)s.at);
  } else {
    return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s holds nothing to remove", tag_name(tag));
  }
  if (c->status) return 0;
  if (st && st != GUATIAO_ERR_NOT_FOUND) return gt_fail(c, st, "the removal was refused");
  return 1;
}

int cmd_node_clear(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  if (!writable(c, &o, &v)) return 0;
  uint32_t tag = guatiao_tag_of(v);
  guatiao_status st;
  if (tag == GUATIAO_MAP) st = gt_core.guatiao_map_clear(v);
  else if (tag == GUATIAO_LIST) st = gt_core.guatiao_list_clear(v);
  else return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "a %s holds nothing to clear", tag_name(tag));
  if (st) return gt_fail(c, st, "the clear was refused");
  return 1;
}

/* node_merge(id, path, src): a map's entries, replacing collisions */
int cmd_node_merge(gt_call *c) {
  gt_owner *o = NULL;
  guatiao_value *v = NULL;
  if (!writable(c, &o, &v)) return 0;
  if (guatiao_tag_of(v) != GUATIAO_MAP) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "only a map merges entries");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  guatiao_value tmp;
  const guatiao_value *src = NULL;
  int at[1] = {2};
  if (!gt_tree_args(c, 1, at, &tmp, &src)) return 0;
  if (!gt_node_again(c, 0, &v)) {
    gt_free(&tmp);
    return 0;
  }
  guatiao_status st = gt_core.guatiao_map_copy_from(a, v, src);
  gt_free(&tmp);
  if (st) return gt_fail(c, st, "the merge was refused");
  return 1;
}

/* node_copy(id, path, alloc) -> a new value's id */
int cmd_node_copy(gt_call *c) {
  const guatiao_alloc *a = NULL;
  guatiao_value *v = NULL;
  if (!gt_arg_alloc(c, 2, &a) || !gt_arg_node(c, 0, NULL, &v)) return 0;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = gt_core.guatiao_value_clone(a, v, &out);
  if (st) return gt_fail(c, st, "the copy was refused");
  return answer_id(c, gt_owner_new(&out, a), &out);
}

/* number_check(text): refuses text that is not a number */
int cmd_number_check(gt_call *c) {
  guatiao_str t;
  if (!gt_arg_text(c, 0, "a number", &t) || !GT_NEED_CORE(c, guatiao_value_number)) return 0;
  guatiao_value v;
  set_absent(&v);
  guatiao_status st = gt_core.guatiao_value_number(&gt_binding_alloc, t, &v);
  gt_free(&v);
  if (st) return gt_fail(c, st, "'%.*s' is not a number", (int)t.len, t.ptr);
  return 1;
}

/* number_read(text, numbers) -> what the number reads as */
int cmd_number_read(gt_call *c) {
  guatiao_str t;
  double n = 0;
  if (!gt_arg_text(c, 0, "a number", &t) || !gt_arg_double(c, 1, "numbers", &n)) return 0;
  mxArray *out = gt_number_mx(c, t, (int)n);
  if (!out) return 0;
  gt_answer(c, 0, out);
  return 1;
}

/* ---- allocators ------------------------------------------------------------------------ */

/* alloc_counting() -> id; kept for the session, as trees record it */
int cmd_alloc_counting(gt_call *c) {
  gt_counting *k = gt_counting_new();
  if (!k) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  uint64_t id = gt_handle_add(GT_H_COUNTING, k);
  if (!id) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

int cmd_alloc_live(gt_call *c) {
  gt_counting *k = NULL;
  if (!gt_arg_handle(c, 0, GT_H_COUNTING, "allocator", (void **)&k)) return 0;
  gt_answer(c, 0, mxCreateDoubleScalar((double)gt_counting_live(k)));
  return 1;
}

/* node_alloc_is(id, path, alloc) -> whether the node records that allocator */
int cmd_node_alloc_is(gt_call *c) {
  guatiao_value *v = NULL;
  const guatiao_alloc *a = NULL;
  if (!gt_arg_node(c, 0, NULL, &v) || !gt_arg_alloc(c, 2, &a)) return 0;
  const guatiao_alloc *rec = NULL;
  switch (guatiao_tag_of(v)) {
  case GUATIAO_MAP: rec = v->payload.map.alloc; break;
  case GUATIAO_LIST: rec = v->payload.list.alloc; break;
  case GUATIAO_STRING: rec = v->payload.text.alloc; break;
  case GUATIAO_NUMBER: rec = v->payload.number.alloc; break;
  case GUATIAO_BYTES: rec = v->payload.bytes.alloc; break;
  default: break;
  }
  gt_answer(c, 0, mxCreateLogicalScalar(rec == a));
  return 1;
}

/* handles(type) -> how many entries of a type the table files */
int cmd_handles(gt_call *c) {
  double t = 0;
  if (!gt_arg_double(c, 0, "a handle type", &t)) return 0;
  gt_answer(c, 0, mxCreateDoubleScalar((double)gt_handle_count((int)t)));
  return 1;
}

/* node_pointer(id, path) -> the node's address, for C code a test calls;
 * valid while its owner is and nothing moves it. */
int cmd_node_pointer(gt_call *c) {
  guatiao_value *v = NULL;
  if (!gt_arg_node(c, 0, NULL, &v)) return 0;
  mxArray *x = mxCreateNumericMatrix(1, 1, mxUINT64_CLASS, mxREAL);
  *(uint64_t *)mxGetData(x) = (uint64_t)(uintptr_t)v;
  gt_answer(c, 0, x);
  return 1;
}

/* alloc_address(alloc) -> the allocator's address, for C code a test calls */
int cmd_alloc_address(gt_call *c) {
  const guatiao_alloc *a = NULL;
  if (!gt_arg_alloc(c, 0, &a)) return 0;
  mxArray *x = mxCreateNumericMatrix(1, 1, mxUINT64_CLASS, mxREAL);
  *(uint64_t *)mxGetData(x) = (uint64_t)(uintptr_t)a;
  gt_answer(c, 0, x);
  return 1;
}

/* serving(fn): calls the M function `fn` inside this gateway call, so C
 * code it reaches may call slots served in M. Errors are fn's to catch:
 * guatiao.serving wraps it so nothing is raised here. */
int cmd_serving(gt_call *c) {
  const mxArray *fn = gt_arg(c, 0);
  if (!fn || !mxIsClass(fn, "function_handle")) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "serving takes a function handle");
  mxArray *in[1] = {(mxArray *)fn};
  return gt_call_m(c, "feval", 0, NULL, 1, in);
}
